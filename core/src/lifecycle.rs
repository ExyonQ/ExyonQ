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
//! Generic kernel lifecycle primitives: drain, shutdown signal, connection counting.
//!
//! PS1A (L2): one accepted TCP connection → at most one successful [`LifecycleState::try_enter`]
//! → exactly one [`ConnectionLifecycleToken`] → exactly one drop decrement.
//!
//! PS1A-LINUX: admission uses increment-and-recheck (Option A) so `try_enter` is linealizable
//! with `start_drain` — no connection remains admitted after drain completes.

use std::cell::RefCell;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::watch;

/// Admission rejected while the server is draining (no counter increment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainRejected;

#[derive(Debug)]
pub struct LifecycleState {
    draining: AtomicBool,
    active_connections: AtomicU64,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

/// Test-only latch to force the interleaving between increment and drain recheck.
#[cfg(test)]
pub static TEST_TRY_ENTER_PAUSE_AFTER_INCREMENT: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
pub static TEST_TRY_ENTER_RELEASE: AtomicBool = AtomicBool::new(false);

impl LifecycleState {
    pub fn new() -> Arc<Self> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Arc::new(Self {
            draining: AtomicBool::new(false),
            active_connections: AtomicU64::new(0),
            shutdown_tx,
            shutdown_rx,
        })
    }

    pub fn start_drain(self: &Arc<Self>) {
        self.draining.store(true, Ordering::Release);
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }

    pub fn request_shutdown(self: &Arc<Self>) {
        self.draining.store(true, Ordering::Release);
        let _ = self.shutdown_tx.send(true);
    }

    pub fn shutdown_requested(&self) -> bool {
        *self.shutdown_rx.borrow()
    }

    pub fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown_rx.clone()
    }

    pub async fn wait_for_shutdown(ops: &Arc<Self>) {
        if ops.shutdown_requested() {
            return;
        }
        let mut rx = ops.shutdown_rx();
        let _ = rx.changed().await;
    }

    /// Observed together with [`Self::is_draining`] for drain completion — Acquire pairs with
    /// token enter (AcqRel) and drop (Release).
    pub fn active_connections(&self) -> u64 {
        self.active_connections.load(Ordering::Acquire)
    }

    /// Admit one connection (Option A: increment and recheck).
    ///
    /// Linearization point on success: recheck observes `draining == false` after increment.
    /// Linearization point on failure: rollback decrement completes before returning `Err`.
    pub fn try_enter(self: &Arc<Self>) -> Result<ConnectionLifecycleToken, DrainRejected> {
        if self.is_draining() {
            return Err(DrainRejected);
        }
        self.active_connections.fetch_add(1, Ordering::AcqRel);

        #[cfg(test)]
        while TEST_TRY_ENTER_PAUSE_AFTER_INCREMENT.load(Ordering::Acquire)
            && !TEST_TRY_ENTER_RELEASE.load(Ordering::Acquire)
        {
            std::thread::yield_now();
        }

        if self.is_draining() {
            self.active_connections.fetch_sub(1, Ordering::Release);
            return Err(DrainRejected);
        }

        Ok(ConnectionLifecycleToken {
            state: Arc::clone(self),
        })
    }

    /// Returns true when graceful drain may complete (no admitted connections remain).
    pub fn drain_complete(&self) -> bool {
        self.is_draining() && self.active_connections() == 0
    }
}

/// RAII connection admission: drop decrements `active_connections` exactly once.
pub struct ConnectionLifecycleToken {
    state: Arc<LifecycleState>,
}

impl Drop for ConnectionLifecycleToken {
    fn drop(&mut self) {
        self.state
            .active_connections
            .fetch_sub(1, Ordering::Release);
    }
}

tokio::task_local! {
    static TASK_CONNECTION_TOKEN: RefCell<Option<ConnectionLifecycleToken>>;
}

/// Run an async connection task with a task-local admission token (Tokio keep-alive handoff).
pub async fn run_with_connection_token<F>(token: ConnectionLifecycleToken, fut: F) -> F::Output
where
    F: Future,
{
    TASK_CONNECTION_TOKEN
        .scope(RefCell::new(Some(token)), fut)
        .await
}

/// Take the task-local token for epoll keep-alive registration (moves ownership out).
pub(crate) fn take_task_connection_token() -> Option<ConnectionLifecycleToken> {
    TASK_CONNECTION_TOKEN
        .try_with(|slot| slot.borrow_mut().take())
        .ok()
        .flatten()
}

/// Admit using task-local token when present, otherwise [`LifecycleState::try_enter`].
pub fn admit_connection(
    ops: &Arc<LifecycleState>,
) -> Result<ConnectionLifecycleToken, DrainRejected> {
    if let Some(token) = take_task_connection_token() {
        return Ok(token);
    }
    ops.try_enter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    fn reset_test_latch() {
        TEST_TRY_ENTER_PAUSE_AFTER_INCREMENT.store(false, Ordering::Release);
        TEST_TRY_ENTER_RELEASE.store(false, Ordering::Release);
    }

    #[test]
    fn try_enter_increments_once() {
        let ops = LifecycleState::new();
        assert_eq!(ops.active_connections(), 0);
        let _t = ops.try_enter().unwrap();
        assert_eq!(ops.active_connections(), 1);
    }

    #[test]
    fn token_drop_decrements_once() {
        let ops = LifecycleState::new();
        {
            let _t = ops.try_enter().unwrap();
            assert_eq!(ops.active_connections(), 1);
        }
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn drain_rejects_new_enter() {
        let ops = LifecycleState::new();
        ops.start_drain();
        assert!(ops.try_enter().is_err());
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn failed_enter_does_not_increment() {
        let ops = LifecycleState::new();
        ops.start_drain();
        let _ = ops.try_enter();
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn rollback_admission_decrements_exactly_once() {
        reset_test_latch();
        TEST_TRY_ENTER_PAUSE_AFTER_INCREMENT.store(true, Ordering::Release);
        let ops = LifecycleState::new();
        let worker = {
            let ops = Arc::clone(&ops);
            thread::spawn(move || ops.try_enter())
        };
        while ops.active_connections() == 0 {
            thread::yield_now();
        }
        assert_eq!(ops.active_connections(), 1);
        ops.start_drain();
        assert!(!ops.drain_complete());
        TEST_TRY_ENTER_RELEASE.store(true, Ordering::Release);
        assert!(worker.join().unwrap().is_err());
        assert_eq!(ops.active_connections(), 0);
        assert!(ops.drain_complete());
        reset_test_latch();
    }

    #[test]
    fn token_move_does_not_decrement_until_final_drop() {
        let ops = LifecycleState::new();
        let t1 = ops.try_enter().unwrap();
        assert_eq!(ops.active_connections(), 1);
        let t2 = t1;
        drop(t2);
        assert_eq!(ops.active_connections(), 0);
    }

    #[tokio::test]
    async fn task_local_scope_preserves_count_until_end() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        run_with_connection_token(token, async {
            assert_eq!(ops.active_connections(), 1);
            let taken = take_task_connection_token();
            assert!(taken.is_some());
            assert_eq!(ops.active_connections(), 1);
            drop(taken);
            assert_eq!(ops.active_connections(), 0);
        })
        .await;
        assert_eq!(ops.active_connections(), 0);
    }

    #[tokio::test]
    async fn admit_connection_reuses_task_local_without_second_enter() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        run_with_connection_token(token, async {
            let again = admit_connection(&ops).unwrap();
            drop(again);
            assert_eq!(ops.active_connections(), 0);
        })
        .await;
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn drain_blocks_until_last_token_drops() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        ops.start_drain();
        assert_eq!(ops.active_connections(), 1);
        assert!(!ops.drain_complete());
        assert!(ops.try_enter().is_err());
        drop(token);
        assert_eq!(ops.active_connections(), 0);
        assert!(ops.drain_complete());
    }

    #[test]
    fn shutdown_signal_with_active_connection() {
        let ops = LifecycleState::new();
        let _token = ops.try_enter().unwrap();
        ops.request_shutdown();
        assert!(ops.shutdown_requested());
        assert!(ops.is_draining());
        assert_eq!(ops.active_connections(), 1);
    }

    #[test]
    fn concurrent_try_enter_while_drain_starts() {
        reset_test_latch();
        let ops = LifecycleState::new();
        let threads = 32;
        let start = Arc::new(Barrier::new(threads + 1));
        let mut handles = Vec::new();
        let successes = Arc::new(AtomicU64::new(0));
        let failures = Arc::new(AtomicU64::new(0));

        for _ in 0..threads {
            let ops = Arc::clone(&ops);
            let start = Arc::clone(&start);
            let successes = Arc::clone(&successes);
            let failures = Arc::clone(&failures);
            handles.push(thread::spawn(move || {
                start.wait();
                match ops.try_enter() {
                    Ok(token) => {
                        successes.fetch_add(1, Ordering::AcqRel);
                        drop(token);
                    }
                    Err(DrainRejected) => {
                        failures.fetch_add(1, Ordering::AcqRel);
                    }
                }
            }));
        }
        start.wait();
        ops.start_drain();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(ops.active_connections(), 0);
        assert!(ops.drain_complete());
        reset_test_latch();
    }

    #[test]
    fn stress_try_enter_never_underflows() {
        const ROUNDS: usize = 200;
        for _ in 0..ROUNDS {
            let ops = LifecycleState::new();
            let threads = 16;
            let barrier = Arc::new(Barrier::new(threads));
            let mut handles = Vec::new();
            for t in 0..threads {
                let ops = Arc::clone(&ops);
                let barrier = Arc::clone(&barrier);
                handles.push(thread::spawn(move || {
                    barrier.wait();
                    if t % 4 == 0 {
                        ops.start_drain();
                    }
                    let maybe = ops.try_enter();
                    std::thread::sleep(std::time::Duration::from_micros(t as u64));
                    drop(maybe);
                }));
            }
            for handle in handles {
                handle.join().unwrap();
            }
            assert_eq!(ops.active_connections(), 0);
        }
    }
}
