/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! FastCGI runtime ownership — pool capacity, delegate timeout, metrics, status mapping (KD1).
//!
//! Generation publish: PREPARE (build N+1) → COMMIT (swap active) → RETIRE (drain N).
//! `POOL_REUSE_ACROSS_GENERATIONS = NO`.

use crate::adapter::FcgiModuleExecutor;
use crate::metrics;
use async_trait::async_trait;
use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiBindError, FcgiCompiledSlot, FcgiDispatchOutcome, FcgiDispatchRequest,
    FcgiDispatchService, FcgiMetricsSnapshot, FcgiRegisterError, FcgiRuntimeRegistration,
    MaterializedBackendOutcome, DEFAULT_FCGI_MAX_CONCURRENCY,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Safe Arc cell for generation registry publish (load/swap).
/// Prefer correctness over lock-free; no raw-pointer Arc games on the dispatch path.
struct ArcCell<T> {
    inner: RwLock<Arc<T>>,
}

impl<T> ArcCell<T> {
    fn new(value: Arc<T>) -> Self {
        Self {
            inner: RwLock::new(value),
        }
    }

    fn load(&self) -> Arc<T> {
        Arc::clone(&self.inner.read().expect("fcgi registry rwlock poisoned"))
    }

    fn swap(&self, next: Arc<T>) -> Arc<T> {
        let mut guard = self.inner.write().expect("fcgi registry rwlock poisoned");
        std::mem::replace(&mut *guard, next)
    }
}

struct GenerationRegistry {
    active: Arc<FcgiRuntimeState>,
    retiring: Vec<Arc<FcgiRuntimeState>>,
}

/// Delegate timeout around blocking FastCGI wire work.
pub const FCGI_DELEGATE_TIMEOUT: Duration = Duration::from_secs(30);

struct FcgiRuntimeState {
    generation: u64,
    executor: ActiveExecutor,
    semaphores: HashMap<u32, Arc<Semaphore>>,
    /// Present when executor is a module ConnPool owner (for quiescence / retire).
    module_executor: Option<Arc<FcgiModuleExecutor>>,
}

/// Concrete module path vs legacy registration `dyn` (keeps Arc<dyn count at baseline=1).
#[derive(Clone)]
enum ActiveExecutor {
    Module(Arc<FcgiModuleExecutor>),
    External(Arc<dyn FcgiBackendExecutor>),
}

impl ActiveExecutor {
    fn dispatch(&self, request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        match self {
            Self::Module(m) => m.dispatch(request),
            Self::External(e) => e.dispatch(request),
        }
    }

    fn begin_drain(&self) {
        match self {
            Self::Module(m) => m.begin_drain(),
            Self::External(e) => e.begin_drain(),
        }
    }
}

impl FcgiRuntimeState {
    fn from_registration(registration: FcgiRuntimeRegistration) -> Result<Self, FcgiRegisterError> {
        let mut semaphores = HashMap::new();
        for (pool_id, capacity) in registration.pool_capacities {
            if capacity == 0 {
                return Err(FcgiRegisterError::InvalidCapacity);
            }
            semaphores
                .entry(pool_id)
                .or_insert_with(|| Arc::new(Semaphore::new(capacity)));
        }
        if semaphores.is_empty() {
            semaphores.insert(0, Arc::new(Semaphore::new(DEFAULT_FCGI_MAX_CONCURRENCY)));
        }
        Ok(Self {
            generation: 0,
            executor: ActiveExecutor::External(registration.executor),
            semaphores,
            module_executor: None,
        })
    }

    fn from_compiled(generation: u64, slots: &[FcgiCompiledSlot]) -> Result<Self, FcgiBindError> {
        let module = Arc::new(FcgiModuleExecutor::from_compiled_slots(generation, slots)?);
        let mut semaphores = HashMap::new();
        for slot in slots {
            if slot.max_concurrency == 0 {
                return Err(FcgiBindError::InvalidCapacity {
                    pool: slot.name.clone(),
                });
            }
            semaphores.insert(
                slot.pool_id,
                Arc::new(Semaphore::new(slot.max_concurrency as usize)),
            );
        }
        Ok(Self {
            generation,
            executor: ActiveExecutor::Module(Arc::clone(&module)),
            semaphores,
            module_executor: Some(module),
        })
    }

    fn empty(generation: u64) -> Self {
        let module =
            Arc::new(FcgiModuleExecutor::from_compiled_slots(generation, &[]).expect("empty"));
        Self {
            generation,
            executor: ActiveExecutor::Module(Arc::clone(&module)),
            semaphores: HashMap::new(),
            module_executor: Some(module),
        }
    }

    fn try_acquire(&self, pool_id: u32) -> Result<OwnedSemaphorePermit, FcgiDispatchOutcome> {
        let Some(semaphore) = self.semaphores.get(&pool_id) else {
            return Err(FcgiDispatchOutcome::BadGateway);
        };
        semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| FcgiDispatchOutcome::ServiceUnavailable)
    }

    fn is_quiescent(&self) -> bool {
        self.module_executor
            .as_ref()
            .map(|m| m.is_quiescent())
            .unwrap_or(true)
    }
}

// EmptyExecutor removed — empty generations use FcgiModuleExecutor with zero slots.

struct InflightGuard;

impl InflightGuard {
    fn enter() -> Self {
        metrics::note_fcgi_inflight_acquire();
        Self
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        metrics::note_fcgi_inflight_release();
    }
}

/// Production/test FastCGI runtime — sole owner of pool policy and wire delegate orchestration.
#[derive(Clone)]
pub struct FcgiRuntime {
    registry: Arc<ArcCell<GenerationRegistry>>,
    active_generation: Arc<AtomicU64>,
    /// When set, bind/reload is rejected so drain cannot accidentally re-open admission.
    global_drain: Arc<AtomicBool>,
    delegate_timeout: Duration,
}

impl FcgiRuntime {
    pub fn new(registration: FcgiRuntimeRegistration) -> Result<Self, FcgiRegisterError> {
        let state = Arc::new(FcgiRuntimeState::from_registration(registration)?);
        let registry = Arc::new(ArcCell::new(Arc::new(GenerationRegistry {
            active: state.clone(),
            retiring: Vec::new(),
        })));
        Ok(Self {
            active_generation: Arc::new(AtomicU64::new(state.generation)),
            registry,
            global_drain: Arc::new(AtomicBool::new(false)),
            delegate_timeout: delegate_timeout_budget(),
        })
    }

    #[doc(hidden)]
    pub fn with_delegate_timeout(
        registration: FcgiRuntimeRegistration,
        timeout: Duration,
    ) -> Result<Self, FcgiRegisterError> {
        let state = Arc::new(FcgiRuntimeState::from_registration(registration)?);
        let registry = Arc::new(ArcCell::new(Arc::new(GenerationRegistry {
            active: state.clone(),
            retiring: Vec::new(),
        })));
        Ok(Self {
            active_generation: Arc::new(AtomicU64::new(state.generation)),
            registry,
            global_drain: Arc::new(AtomicBool::new(false)),
            delegate_timeout: timeout,
        })
    }

    fn load_active(&self) -> Arc<FcgiRuntimeState> {
        Arc::clone(&self.registry.load().active)
    }

    fn reap_retiring_in(reg: &mut GenerationRegistry) {
        reg.retiring.retain(|gen| {
            if gen.is_quiescent() {
                metrics::note_fcgi_pool_generation_retired();
                false
            } else {
                true
            }
        });
    }

    fn reap_retiring(&self) {
        let cur = self.registry.load();
        if cur.retiring.is_empty() {
            return;
        }
        let mut next = GenerationRegistry {
            active: Arc::clone(&cur.active),
            retiring: cur.retiring.clone(),
        };
        Self::reap_retiring_in(&mut next);
        if next.retiring.len() != cur.retiring.len() {
            self.registry.swap(Arc::new(next));
        }
    }

    /// Test/ops: count of generations still draining (bounded by reload rate).
    pub fn retiring_count(&self) -> usize {
        self.reap_retiring();
        self.registry.load().retiring.len()
    }

    async fn run_delegate(&self, request: FcgiDispatchRequest) -> FcgiDispatchOutcome {
        let state = self.load_active();
        let permit = match state.try_acquire(request.pool_id) {
            Ok(permit) => permit,
            Err(FcgiDispatchOutcome::ServiceUnavailable) => {
                metrics::note_fcgi_saturation_rejection();
                return FcgiDispatchOutcome::ServiceUnavailable;
            }
            Err(other) => return other,
        };

        let executor = state.executor.clone();
        let abort_gate = crate::request_abort::AbortGate::new();
        let abort_for_task = std::sync::Arc::clone(&abort_gate);
        let task = tokio::task::spawn_blocking(move || {
            let _inflight = InflightGuard::enter();
            let _abort_scope = crate::request_abort::AbortScope::enter(abort_for_task);
            executor.dispatch(&request)
        });

        let outcome = match tokio::time::timeout(self.delegate_timeout, task).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_join)) => FcgiDispatchOutcome::BadGateway,
            Err(_elapsed) => {
                abort_gate.signal_abort();
                FcgiDispatchOutcome::GatewayTimeout
            }
        };
        drop(permit);
        self.reap_retiring();
        outcome
    }
}

#[async_trait]
impl FcgiDispatchService for FcgiRuntime {
    async fn dispatch(&self, request: FcgiDispatchRequest) -> MaterializedBackendOutcome {
        map_dispatch_outcome(self.run_delegate(request).await)
    }

    fn metrics(&self) -> FcgiMetricsSnapshot {
        metrics::snapshot()
    }

    fn begin_drain(&self) {
        self.global_drain.store(true, Ordering::Release);
        let active = self.load_active();
        active.executor.begin_drain();
        metrics::note_fcgi_pool_generation_drained();
    }

    fn bind_compiled_pools(
        &self,
        generation: u64,
        slots: &[FcgiCompiledSlot],
    ) -> Result<(), FcgiBindError> {
        // Policy H: never exit global drain via reload (reject, keep current plan/pools).
        if self.global_drain.load(Ordering::Acquire) {
            return Err(FcgiBindError::Draining);
        }

        // PREPARE — fail closed before touching active.
        let prepared = if slots.is_empty() {
            Arc::new(FcgiRuntimeState::empty(generation))
        } else {
            Arc::new(FcgiRuntimeState::from_compiled(generation, slots)?)
        };

        // COMMIT — publish plan-aligned generation + pool registry together.
        let cur = self.registry.load();
        let previous = Arc::clone(&cur.active);
        let mut retiring = cur.retiring.clone();
        // RETIRE — drain previous; never checkin into N+1 (separate ConnPool objects).
        if previous.generation != generation || previous.module_executor.is_some() {
            previous.executor.begin_drain();
            metrics::note_fcgi_pool_generation_drained();
            retiring.push(previous);
        }
        let mut next = GenerationRegistry {
            active: Arc::clone(&prepared),
            retiring,
        };
        Self::reap_retiring_in(&mut next);
        self.registry.swap(Arc::new(next));
        self.active_generation.store(generation, Ordering::Release);
        metrics::set_fcgi_pool_generation_active(generation);
        Ok(())
    }

    fn active_pool_generation(&self) -> u64 {
        self.active_generation.load(Ordering::Acquire)
    }
}

fn delegate_timeout_budget() -> Duration {
    if let Ok(raw) = std::env::var("EXYONQ_FCGI_DELEGATE_TIMEOUT_MS") {
        if let Ok(ms) = raw.parse::<u64>() {
            if ms > 0 {
                return Duration::from_millis(ms);
            }
        }
    }
    FCGI_DELEGATE_TIMEOUT
}

fn map_dispatch_outcome(outcome: FcgiDispatchOutcome) -> MaterializedBackendOutcome {
    match outcome {
        FcgiDispatchOutcome::NotRegistered => {
            metrics::note_fcgi_response_501();
            error_outcome(501, "not implemented")
        }
        FcgiDispatchOutcome::ServiceUnavailable => {
            metrics::note_fcgi_response_503();
            error_outcome(503, "service unavailable")
        }
        FcgiDispatchOutcome::BadGateway => {
            metrics::note_fcgi_response_502();
            error_outcome(502, "bad gateway")
        }
        FcgiDispatchOutcome::GatewayTimeout => {
            metrics::note_fcgi_response_504();
            error_outcome(504, "gateway timeout")
        }
        FcgiDispatchOutcome::Success(response) => {
            if response.status == 200 {
                metrics::note_fcgi_response_200();
            }
            MaterializedBackendOutcome {
                status: response.status,
                headers: response.headers,
                body: response.body,
            }
        }
    }
}

fn error_outcome(status: u16, body: &str) -> MaterializedBackendOutcome {
    MaterializedBackendOutcome {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

static RUNTIME_OVERRIDE: Mutex<Option<Arc<FcgiRuntime>>> = Mutex::new(None);

/// Test-only global runtime override (core test harness).
#[doc(hidden)]
pub fn set_runtime_for_tests(runtime: Option<Arc<FcgiRuntime>>) {
    if let Ok(mut slot) = RUNTIME_OVERRIDE.lock() {
        *slot = runtime;
    }
}

#[doc(hidden)]
pub fn runtime_for_tests() -> Option<Arc<FcgiRuntime>> {
    RUNTIME_OVERRIDE.lock().ok()?.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_module_api::fcgi_dispatch::FcgiSuccessResponse;

    /// Unit-test executor that returns a fixed outcome (not a product transport).
    struct FixedOutcomeExecutor(FcgiDispatchOutcome);

    impl FcgiBackendExecutor for FixedOutcomeExecutor {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            self.0.clone()
        }
    }

    fn test_request() -> FcgiDispatchRequest {
        FcgiDispatchRequest {
            pool_id: 0,
            method: "GET".into(),
            request_uri: "/index.php".into(),
            query_string: String::new(),
            script_name: "/index.php".into(),
            script_filename: "/var/www/index.php".into(),
            path_info: None,
            document_root: "/var/www".into(),
            server_name: "localhost".into(),
            server_port: 80,
            remote_addr: "127.0.0.1".into(),
            server_protocol: "HTTP/1.1".into(),
            content_type: None,
            body: Vec::new(),
            headers: Vec::new(),
        }
    }

    fn success_response() -> FcgiDispatchOutcome {
        FcgiDispatchOutcome::Success(FcgiSuccessResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"ok".to_vec(),
        })
    }

    #[tokio::test]
    async fn success_maps_to_200() {
        let _gate = metrics::fcgi_metric_test_gate();
        let runtime = FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(FixedOutcomeExecutor(success_response())),
            pool_capacities: vec![(0, 16)],
        })
        .unwrap();
        let before = metrics::fcgi_responses_200_total();
        let outcome = runtime.dispatch(test_request()).await;
        assert_eq!(outcome.status, 200);
        assert_eq!(metrics::fcgi_responses_200_total(), before + 1);
    }

    #[tokio::test]
    async fn saturation_maps_to_503() {
        let _gate = metrics::fcgi_metric_test_gate();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        struct GateExecutor {
            started_tx: std::sync::mpsc::Sender<()>,
            release_rx: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl FcgiBackendExecutor for GateExecutor {
            fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
                let _ = self.started_tx.send(());
                if let Ok(rx) = self.release_rx.lock() {
                    let _ = rx.recv();
                }
                success_response()
            }
        }
        let runtime = FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(GateExecutor {
                started_tx,
                release_rx: std::sync::Mutex::new(release_rx),
            }),
            pool_capacities: vec![(0, 1)],
        })
        .unwrap();

        let fut_a = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.dispatch(test_request()).await }
        });
        while started_rx.try_recv().is_err() {
            tokio::task::yield_now().await;
        }
        let outcome_b = runtime.dispatch(test_request()).await;
        assert_eq!(outcome_b.status, 503);
        release_tx.send(()).unwrap();
        let outcome_a = fut_a.await.unwrap();
        assert_eq!(outcome_a.status, 200);
    }

    #[tokio::test]
    async fn timeout_releases_pool_permit_before_blocking_task_finishes() {
        let _gate = metrics::fcgi_metric_test_gate();
        struct SlowOnce(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl FcgiBackendExecutor for SlowOnce {
            fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
                if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    std::thread::sleep(Duration::from_millis(400));
                }
                success_response()
            }
        }
        let runtime = FcgiRuntime::with_delegate_timeout(
            FcgiRuntimeRegistration {
                executor: Arc::new(SlowOnce(Arc::new(std::sync::atomic::AtomicUsize::new(0)))),
                pool_capacities: vec![(0, 1)],
            },
            Duration::from_millis(50),
        )
        .unwrap();

        let slow = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.dispatch(test_request()).await }
        });
        let timed_out = slow.await.unwrap();
        assert_eq!(timed_out.status, 504);

        let immediate = runtime.dispatch(test_request()).await;
        assert_eq!(
            immediate.status, 200,
            "pool permit must be released after timeout, not held until slow task completes"
        );
    }

    #[tokio::test]
    async fn join_error_releases_pool_permit() {
        let _gate = metrics::fcgi_metric_test_gate();
        struct PanicOnce(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl FcgiBackendExecutor for PanicOnce {
            fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
                if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    panic!("forced join failure for pool-permit release test");
                }
                success_response()
            }
        }
        let runtime = FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(PanicOnce(Arc::new(std::sync::atomic::AtomicUsize::new(0)))),
            pool_capacities: vec![(0, 1)],
        })
        .unwrap();

        let outcome = runtime.dispatch(test_request()).await;
        assert_eq!(outcome.status, 502);

        let recovered = runtime.dispatch(test_request()).await;
        assert_eq!(
            recovered.status, 200,
            "pool permit must be released after join error"
        );
    }

    #[tokio::test]
    async fn executor_error_releases_pool_permit() {
        let _gate = metrics::fcgi_metric_test_gate();
        let runtime = FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(FixedOutcomeExecutor(FcgiDispatchOutcome::BadGateway)),
            pool_capacities: vec![(0, 1)],
        })
        .unwrap();
        assert_eq!(runtime.dispatch(test_request()).await.status, 502);
        assert_eq!(runtime.dispatch(test_request()).await.status, 502);
    }

    /// DB5 Part C: repeated timeouts must not exhaust semaphore capacity.
    #[tokio::test]
    async fn db5_repeated_timeouts_do_not_accumulate_beyond_cap() {
        let _gate = metrics::fcgi_metric_test_gate();
        struct AlwaysSlow;
        impl FcgiBackendExecutor for AlwaysSlow {
            fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
                std::thread::sleep(Duration::from_millis(200));
                success_response()
            }
        }
        let runtime = FcgiRuntime::with_delegate_timeout(
            FcgiRuntimeRegistration {
                executor: Arc::new(AlwaysSlow),
                pool_capacities: vec![(0, 1)],
            },
            Duration::from_millis(20),
        )
        .unwrap();

        for i in 0..8 {
            let outcome = runtime.dispatch(test_request()).await;
            assert_eq!(
                outcome.status, 504,
                "iteration {i}: expect timeout; semaphore must release so next acquire succeeds"
            );
        }
    }

    /// DB5 Part C: timeout stops waiting; blocking work may still run (not cancelled).
    #[tokio::test]
    async fn db5_timeout_stops_waiting_but_blocking_work_may_continue() {
        let _gate = metrics::fcgi_metric_test_gate();
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct FinishFlag(Arc<std::sync::atomic::AtomicBool>);
        impl FcgiBackendExecutor for FinishFlag {
            fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
                std::thread::sleep(Duration::from_millis(150));
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                success_response()
            }
        }
        let flag = Arc::clone(&finished);
        let runtime = FcgiRuntime::with_delegate_timeout(
            FcgiRuntimeRegistration {
                executor: Arc::new(FinishFlag(flag)),
                pool_capacities: vec![(0, 2)],
            },
            Duration::from_millis(20),
        )
        .unwrap();

        let outcome = runtime.dispatch(test_request()).await;
        assert_eq!(outcome.status, 504);
        assert!(
            !finished.load(std::sync::atomic::Ordering::SeqCst),
            "at timeout return, slow blocking work should often still be running"
        );
        // Wait for orphaned blocking task to finish (Tokio does not cancel spawn_blocking on JoinHandle drop).
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !finished.load(std::sync::atomic::Ordering::SeqCst)
            && std::time::Instant::now() < deadline
        {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            finished.load(std::sync::atomic::Ordering::SeqCst),
            "blocking work eventually completes even after await timed out — ACCEPTED_BOUNDED_LIMITATION"
        );
    }
}
