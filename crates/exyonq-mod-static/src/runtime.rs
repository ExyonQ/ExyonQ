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
//! KD2.2 — `StaticDispatchService` runtime (resolution + outcome ownership).

#[cfg(target_os = "linux")]
use crate::outcome::try_sendfile_outcome;
use crate::outcome::{identity_to_snapshot, static_error_outcome};
use crate::sendfile_handle::SendfileHandleRegistry;
use crate::{PreloadLimits, StaticError, StaticRoot};
use async_trait::async_trait;
use exyonq_module_api::fcgi_dispatch::MaterializedBackendOutcome;
use exyonq_module_api::static_dispatch::{
    SendfileHandle, StaticCacheLoadOutcome, StaticCompiledSlot, StaticDispatchBody,
    StaticDispatchOutcome, StaticDispatchRequest, StaticDispatchService, StaticMethod,
    StaticMetricsSnapshot,
};
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

struct RootTable {
    generation: u64,
    slots: Box<[Arc<StaticRoot>]>,
}

impl Default for RootTable {
    fn default() -> Self {
        Self {
            generation: 0,
            slots: Box::new([]),
        }
    }
}

/// Module-owned static runtime — root table + sendfile handle registry.
pub struct StaticRuntime {
    roots: RwLock<RootTable>,
    sendfile_handles: Arc<SendfileHandleRegistry>,
    metrics: StaticMetrics,
}

struct StaticMetrics {
    responses_200: AtomicU64,
    responses_404: AtomicU64,
    responses_403: AtomicU64,
    responses_500: AtomicU64,
    sendfile_engagements: AtomicU64,
}

impl Default for StaticMetrics {
    fn default() -> Self {
        Self {
            responses_200: AtomicU64::new(0),
            responses_404: AtomicU64::new(0),
            responses_403: AtomicU64::new(0),
            responses_500: AtomicU64::new(0),
            sendfile_engagements: AtomicU64::new(0),
        }
    }
}

impl Default for StaticRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl StaticRuntime {
    pub fn new() -> Self {
        Self {
            roots: RwLock::new(RootTable::default()),
            sendfile_handles: Arc::new(SendfileHandleRegistry::new()),
            metrics: StaticMetrics::default(),
        }
    }

    /// Bind compiled static roots (called from core reload shell).
    pub fn bind_roots(&self, generation: u64, slots: Box<[Arc<StaticRoot>]>) {
        self.sendfile_handles
            .invalidate_through_generation(generation);
        let mut roots = self.roots.write().expect("static roots poisoned");
        roots.generation = generation;
        roots.slots = slots;
    }

    /// Build operational roots from compiled snapshot slots (KD2.5).
    ///
    /// Builds the full new root set privately; publishes atomically via [`Self::bind_roots`].
    /// On hard failure, keeps the previously published snapshot (reload safety).
    pub fn bind_compiled_slots(&self, generation: u64, slots: &[StaticCompiledSlot]) {
        let mut built = Vec::with_capacity(slots.len());
        for slot in slots {
            let limits = PreloadLimits {
                max_file_bytes: slot.preload_max_file_bytes,
                max_total_bytes: slot.preload_max_total_bytes,
                max_entries: slot.preload_max_entries,
            };
            let mut root = match StaticRoot::new_with_preload_limits(
                &slot.filesystem_root,
                &slot.route_prefix,
                slot.index_file.as_deref(),
                limits,
            ) {
                Ok(root) => root,
                Err(err) => {
                    tracing::error!(
                        generation,
                        root = %slot.filesystem_root.display(),
                        error = %err,
                        "static reload/build failed; old snapshot retained"
                    );
                    return;
                }
            };
            if let Err(err) = root.preload_tree() {
                tracing::error!(
                    generation,
                    root = %slot.filesystem_root.display(),
                    error = %err,
                    "static preload build failed; old snapshot retained"
                );
                return;
            }
            built.push(Arc::new(root));
        }
        tracing::info!(
            generation,
            slots = built.len(),
            "static preload snapshot published"
        );
        self.bind_roots(generation, built.into_boxed_slice());
    }

    pub fn generation(&self) -> u64 {
        self.roots.read().expect("static roots poisoned").generation
    }

    fn root_for_slot(&self, root_slot: u32) -> Result<Arc<StaticRoot>, StaticError> {
        let roots = self.roots.read().expect("static roots poisoned");
        roots
            .slots
            .get(root_slot as usize)
            .cloned()
            .ok_or(StaticError::NotFound)
    }

    /// Epoll bench hook — resolve root without surfacing error type to module-api.
    #[cfg(target_os = "linux")]
    pub(crate) fn root_for_slot_public(
        &self,
        root_slot: u32,
    ) -> Result<Arc<StaticRoot>, StaticError> {
        self.root_for_slot(root_slot)
    }

    /// Issue a sendfile handle for epoll bench path (generation-tagged).
    #[cfg(target_os = "linux")]
    pub(crate) fn issue_sendfile_handle(
        &self,
        asset: Arc<crate::sendfile::SendfileAsset>,
    ) -> SendfileHandle {
        self.sendfile_handles.issue(self.generation(), asset)
    }

    fn note_outcome(&self, outcome: &StaticDispatchOutcome) {
        match outcome.status {
            200 => {
                self.metrics.responses_200.fetch_add(1, Ordering::Relaxed);
            }
            404 => {
                self.metrics.responses_404.fetch_add(1, Ordering::Relaxed);
            }
            403 => {
                self.metrics.responses_403.fetch_add(1, Ordering::Relaxed);
            }
            500..=599 => {
                self.metrics.responses_500.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        if matches!(outcome.body, StaticDispatchBody::SendfileHandle(_)) {
            self.metrics
                .sendfile_engagements
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Consume a sendfile handle exactly once (KD2.3 FSM entry; KD2.2 materializer).
    pub fn take_sendfile_handle(
        &self,
        handle: SendfileHandle,
    ) -> Option<Arc<crate::sendfile::SendfileAsset>> {
        self.sendfile_handles.take(handle)
    }

    /// Cancel a handle without transfer (error/timeout).
    pub fn release_sendfile_handle(&self, handle: SendfileHandle) {
        self.sendfile_handles.release(handle);
    }

    pub fn sendfile_handle_registry(&self) -> &SendfileHandleRegistry {
        &self.sendfile_handles
    }
}

#[async_trait]
impl StaticDispatchService for StaticRuntime {
    async fn dispatch(&self, request: StaticDispatchRequest) -> StaticDispatchOutcome {
        let Ok(root) = self.root_for_slot(request.root_slot) else {
            let outcome = static_error_outcome(StaticError::NotFound);
            self.note_outcome(&outcome);
            return outcome;
        };
        let request_path = request.request_path.as_ref();
        let budget = request.materialization_budget_bytes;

        // SendfileHandle → Hyper/H3 materialize does a full read_exact. Skip that path
        // when a materialization budget is present (H3); H1/H2 keep budget=None.
        #[cfg(target_os = "linux")]
        if budget.is_none() {
            if let Some(outcome) = try_sendfile_outcome(
                &root,
                request.method,
                request_path,
                self.generation(),
                &self.sendfile_handles,
            ) {
                self.note_outcome(&outcome);
                return outcome;
            }
        }

        let result = match request.method {
            StaticMethod::Head => root.serve_head_request(request_path),
            StaticMethod::Get => root.serve_request(request_path),
        };

        match result {
            Ok(response) => {
                let outcome =
                    hyper_response_to_dispatch_outcome(request.method, response, budget).await;
                self.note_outcome(&outcome);
                outcome
            }
            Err(StaticError::NotFound) => {
                // Preload miss / skipped / disabled: serve on demand from disk.
                let outcome = match root.resolve_live_file_path(request_path) {
                    Ok(path) => {
                        self.serve_resolved_path(request.method, &path, budget).await
                    }
                    Err(err) => static_error_outcome(err),
                };
                self.note_outcome(&outcome);
                outcome
            }
            Err(err) => {
                let outcome = static_error_outcome(err);
                self.note_outcome(&outcome);
                outcome
            }
        }
    }

    async fn load_for_cache(&self, request: StaticDispatchRequest) -> StaticCacheLoadOutcome {
        let Ok(root) = self.root_for_slot(request.root_slot) else {
            return StaticCacheLoadOutcome {
                cacheable: false,
                outcome: static_error_outcome(StaticError::NotFound),
                identity: None,
            };
        };
        let request_path = request.request_path.as_ref();
        let budget = request.materialization_budget_bytes;
        let outcome = match request.method {
            StaticMethod::Head => match root
                .resolve_live_file_path(request_path)
                .and_then(|path| crate::serve_head_sync(&path))
            {
                Ok(response) => {
                    hyper_response_to_dispatch_outcome(request.method, response, budget).await
                }
                Err(err) => static_error_outcome(err),
            },
            StaticMethod::Get => match root
                .resolve_live_file_path(request_path)
                .and_then(|path| crate::read_file_bytes_with_budget(&path, budget))
            {
                Ok((bytes, content_type)) => bytes_to_dispatch_outcome(bytes, content_type, budget),
                Err(err) => static_error_outcome(err),
            },
        };
        self.note_outcome(&outcome);
        let identity = if outcome.status == 200 {
            root.capture_identity_for_request(request_path)
                .ok()
                .map(identity_to_snapshot)
        } else {
            None
        };
        StaticCacheLoadOutcome {
            cacheable: outcome.status == 200,
            outcome,
            identity,
        }
    }

    fn metrics(&self) -> StaticMetricsSnapshot {
        StaticMetricsSnapshot {
            responses_200: self.metrics.responses_200.load(Ordering::Relaxed),
            responses_404: self.metrics.responses_404.load(Ordering::Relaxed),
            responses_403: self.metrics.responses_403.load(Ordering::Relaxed),
            responses_500: self.metrics.responses_500.load(Ordering::Relaxed),
            cache_hits: crate::static_cache_hits_total(),
            cache_misses: crate::static_cache_misses_total(),
            sendfile_engagements: self.metrics.sendfile_engagements.load(Ordering::Relaxed),
        }
    }

    fn bind_compiled_slots(&self, generation: u64, slots: &[StaticCompiledSlot]) {
        StaticRuntime::bind_compiled_slots(self, generation, slots);
    }

    fn materialize_outcome(&self, outcome: StaticDispatchOutcome) -> MaterializedBackendOutcome {
        let (status, headers, body) =
            crate::materialize_outcome(outcome, self.sendfile_handle_registry());
        MaterializedBackendOutcome {
            status,
            headers,
            body,
        }
    }

    fn snapshot_matches_current(
        &self,
        snapshot: &exyonq_module_api::static_dispatch::StaticResourceIdentitySnapshot,
    ) -> bool {
        crate::snapshot_matches_current(snapshot)
    }

    fn canonical_path_for_invalidation(&self, path: &std::path::Path) -> std::path::PathBuf {
        crate::canonical_path_for_invalidation(path)
    }

    fn canonical_path_for_snapshot(
        &self,
        snapshot: &exyonq_module_api::static_dispatch::StaticResourceIdentitySnapshot,
    ) -> std::path::PathBuf {
        crate::canonical_path_for_snapshot(snapshot)
    }

    fn cache_storage_method(&self, client: StaticMethod) -> StaticMethod {
        crate::cache_storage_method(client)
    }

    fn note_cache_revalidation_success(&self) {
        crate::note_static_revalidation_success();
    }

    fn note_cache_revalidation_failure(&self) {
        crate::note_static_revalidation_failure();
    }

    fn note_cache_invalidation(&self) {
        crate::note_static_cache_invalidation();
    }

    fn cache_revalidation_success_total(&self) -> u64 {
        crate::revalidation_success_total()
    }

    fn cache_revalidation_failure_total(&self) -> u64 {
        crate::revalidation_failure_total()
    }

    fn cache_invalidations_total(&self) -> u64 {
        crate::static_cache_invalidations_total()
    }

    fn probe_static_index(
        &self,
        root_slot: u32,
        dir_uri: &str,
        candidates: &[String],
    ) -> Option<String> {
        let root = self.root_for_slot(root_slot).ok()?;
        for name in candidates {
            if !exyonq_module_api::validate_directory_index_candidate(name) {
                continue;
            }
            let Some(child_uri) = exyonq_module_api::join_directory_index_uri(dir_uri, name) else {
                continue;
            };
            if root.resolve_path_sync(&child_uri).is_ok() {
                return Some(child_uri);
            }
        }
        None
    }

    async fn serve_resolved_path(
        &self,
        method: StaticMethod,
        path: &std::path::Path,
        materialization_budget_bytes: Option<u64>,
    ) -> StaticDispatchOutcome {
        match method {
            StaticMethod::Head => match crate::serve_head_sync(path) {
                Ok(response) => {
                    hyper_response_to_dispatch_outcome(method, response, materialization_budget_bytes)
                        .await
                }
                Err(err) => static_error_outcome(err),
            },
            StaticMethod::Get => {
                match crate::read_file_bytes_with_budget(path, materialization_budget_bytes) {
                    Ok((bytes, content_type)) => {
                        bytes_to_dispatch_outcome(bytes, content_type, materialization_budget_bytes)
                    }
                    Err(err) => static_error_outcome(err),
                }
            }
        }
    }
}

fn bytes_to_dispatch_outcome(
    bytes: Vec<u8>,
    content_type: &'static str,
    budget: Option<u64>,
) -> StaticDispatchOutcome {
    if let Some(limit) = budget {
        if bytes.len() as u64 > limit {
            return static_error_outcome(StaticError::BudgetExceeded);
        }
    }
    let len = bytes.len();
    let body = if bytes.is_empty() {
        StaticDispatchBody::Empty
    } else {
        StaticDispatchBody::Inline(bytes)
    };
    StaticDispatchOutcome {
        status: 200,
        headers: vec![
            ("content-type".to_string(), content_type.to_string()),
            ("content-length".to_string(), len.to_string()),
        ],
        body,
    }
}

async fn hyper_response_to_dispatch_outcome<B>(
    method: StaticMethod,
    response: hyper::Response<B>,
    budget: Option<u64>,
) -> StaticDispatchOutcome
where
    B: hyper::body::Body<Data = bytes::Bytes, Error = hyper::Error> + Send + Unpin + 'static,
{
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect();
    let body = if method == StaticMethod::Head {
        StaticDispatchBody::Empty
    } else {
        match response.into_body().collect().await {
            Ok(collected) => {
                let bytes = collected.to_bytes();
                if let Some(limit) = budget {
                    if bytes.len() as u64 > limit {
                        return static_error_outcome(StaticError::BudgetExceeded);
                    }
                }
                if bytes.is_empty() {
                    StaticDispatchBody::Empty
                } else {
                    StaticDispatchBody::Inline(bytes.to_vec())
                }
            }
            Err(_) => StaticDispatchBody::Inline(b"read error".to_vec()),
        }
    };
    StaticDispatchOutcome {
        status,
        headers,
        body,
    }
}

#[cfg(test)]
impl StaticRuntime {
    pub fn bind_roots_for_tests(&self, generation: u64, slots: Box<[Arc<StaticRoot>]>) {
        self.bind_roots(generation, slots);
    }
}
