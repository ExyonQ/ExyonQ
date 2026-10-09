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

use crate::conditional::{
    decide_conditional, not_modified_headers, single_header_value, ConditionalDecision,
    StaticValidators,
};
use crate::identity::StaticResourceIdentity;
#[cfg(target_os = "linux")]
use crate::outcome::try_sendfile_outcome;
use crate::outcome::{identity_to_snapshot, static_error_outcome};
use crate::sendfile_handle::SendfileHandleRegistry;
use crate::{
    decide_range, range_header_value, range_response_headers, PreloadLimits, RangeDecision,
    StaticError, StaticRoot,
};
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
                Ok(mut root) => {
                    root.set_route_host(slot.route_host.clone());
                    root.set_allow_sensitive(slot.allow_sensitive);
                    root
                }
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

    /// Longest static route prefix that contains `path` and allows `request_host`.
    ///
    /// Hostless routes match any Host. A named host beats a hostless route of the
    /// same prefix, and an exact host beats a wildcard, matching route ranking.
    #[cfg(target_os = "linux")]
    pub(crate) fn slot_for_request_path(
        &self,
        path: &str,
        request_host: Option<&str>,
    ) -> Option<u32> {
        let roots = self.roots.read().expect("static roots poisoned");
        let mut best: Option<(u32, usize, u8)> = None;
        for (idx, root) in roots.slots.iter().enumerate() {
            if !Self::static_host_allows(root.route_host(), request_host) {
                continue;
            }
            let prefix = root.route_prefix().trim_end_matches('/');
            let matches = if prefix.is_empty() {
                path.starts_with('/')
            } else {
                path == prefix || path.starts_with(&format!("{prefix}/"))
            };
            if !matches {
                continue;
            }
            let len = prefix.len();
            let rank = Self::static_host_rank(root.route_host());
            if best
                .map(|(_, best_len, best_rank)| (len, rank) > (best_len, best_rank))
                .unwrap_or(true)
            {
                best = Some((idx as u32, len, rank));
            }
        }
        best.map(|(idx, _, _)| idx)
    }

    /// Cap033 host identity: case-insensitive, trailing dot ignored. `*.suffix` is a wildcard.
    #[cfg(target_os = "linux")]
    fn static_host_allows(route_host: Option<&str>, request_host: Option<&str>) -> bool {
        let Some(expected) = route_host else {
            return true;
        };
        let Some(actual) = request_host else {
            return false;
        };
        let actual_n = Self::normalize_static_host(actual);
        if let Some(suffix) = expected.strip_prefix('*') {
            return actual_n.ends_with(&Self::normalize_static_host(suffix));
        }
        Self::normalize_static_host(expected) == actual_n
    }

    #[cfg(target_os = "linux")]
    fn static_host_rank(route_host: Option<&str>) -> u8 {
        match route_host {
            None => 0,
            Some(host) if host.starts_with('*') => 1,
            Some(_) => 2,
        }
    }

    #[cfg(target_os = "linux")]
    fn normalize_static_host(host: &str) -> String {
        host.trim_end_matches('.').to_ascii_lowercase()
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

    #[cfg(target_os = "linux")]
    pub(crate) fn note_sendfile_engagement(&self) {
        self.metrics
            .sendfile_engagements
            .fetch_add(1, Ordering::Relaxed);
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
        let range_raw = range_header_value(&request.headers);
        let inm = single_header_value(&request.headers, "if-none-match");
        let ims = single_header_value(&request.headers, "if-modified-since");

        // Cap004 authorize → Cap020 validators → Cap019 Range (shared live-path gate).
        if let Ok(path) = root.resolve_live_file_path(request_path) {
            match serve_authorized_path_conditional_range(
                request.method,
                &path,
                inm,
                ims,
                range_raw,
                budget,
            ) {
                Ok(CondRangeGate::Handled(outcome)) => {
                    self.note_outcome(&outcome);
                    return outcome;
                }
                Ok(CondRangeGate::ServeFull { validators }) => {
                    // Cap020 Continue + Cap019 Ignore: serve Cap004 path with fresh I/O.
                    // Attach the Cap020 decision validators (no second metadata read).
                    #[cfg(target_os = "linux")]
                    if budget.is_none() {
                        if let Some(mut outcome) = try_sendfile_outcome(
                            &root,
                            request.method,
                            request_path,
                            self.generation(),
                            &self.sendfile_handles,
                        ) {
                            attach_validators(&mut outcome, &validators);
                            self.note_outcome(&outcome);
                            return outcome;
                        }
                    }
                    let mut outcome = self
                        .serve_resolved_path(request.method, &path, budget)
                        .await;
                    attach_validators(&mut outcome, &validators);
                    self.note_outcome(&outcome);
                    return outcome;
                }
                Err(err) => {
                    let outcome = static_error_outcome(err);
                    self.note_outcome(&outcome);
                    return outcome;
                }
            }
        }

        // SendfileHandle → Hyper/H3 materialize does a full read_exact. Skip that path
        // when a materialization budget is present (H3); H1/H2 keep budget=None.
        // Cap019: honored Range already returned above; Ignore→200 must still use sendfile
        // so headers match the no-Range path.
        #[cfg(target_os = "linux")]
        if budget.is_none() {
            if let Some(mut outcome) = try_sendfile_outcome(
                &root,
                request.method,
                request_path,
                self.generation(),
                &self.sendfile_handles,
            ) {
                if let Ok(path) = root.resolve_live_file_path(request_path) {
                    attach_validators_if_success(&mut outcome, &path);
                }
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
                let mut outcome =
                    hyper_response_to_dispatch_outcome(request.method, response, budget).await;
                if let Ok(path) = root.resolve_live_file_path(request_path) {
                    attach_validators_if_success(&mut outcome, &path);
                }
                self.note_outcome(&outcome);
                outcome
            }
            Err(StaticError::NotFound) => {
                // Preload miss / skipped / disabled: serve on demand from disk.
                let outcome = match root.resolve_live_file_path(request_path) {
                    Ok(path) => {
                        let mut outcome = self
                            .serve_resolved_path(request.method, &path, budget)
                            .await;
                        attach_validators_if_success(&mut outcome, &path);
                        outcome
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
        // Cap020: fill path must emit ETag/Last-Modified on the stored 200 representation.
        // Conditional requests bypass cache eligibility and use `dispatch` instead.
        // Bind validators + cache identity from one metadata snapshot before body I/O.
        let (outcome, identity) = match root.resolve_live_file_path(request_path) {
            Ok(path) => match StaticResourceIdentity::capture(&path) {
                Ok(id) => {
                    let validators = StaticValidators::from_identity(&id);
                    let mut outcome = match request.method {
                        StaticMethod::Head => match crate::serve_head_sync(&path) {
                            Ok(response) => {
                                hyper_response_to_dispatch_outcome(request.method, response, budget)
                                    .await
                            }
                            Err(err) => static_error_outcome(err),
                        },
                        StaticMethod::Get => {
                            match crate::read_file_bytes_with_budget(&path, budget) {
                                Ok((bytes, content_type)) => {
                                    bytes_to_dispatch_outcome(bytes, content_type, budget)
                                }
                                Err(err) => static_error_outcome(err),
                            }
                        }
                    };
                    if outcome.status == 200 {
                        attach_validators(&mut outcome, &validators);
                    }
                    let snap = if outcome.status == 200 {
                        Some(identity_to_snapshot(id))
                    } else {
                        None
                    };
                    (outcome, snap)
                }
                Err(err) => (static_error_outcome(StaticError::Io(err)), None),
            },
            Err(err) => (static_error_outcome(err), None),
        };
        self.note_outcome(&outcome);
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
                    hyper_response_to_dispatch_outcome(
                        method,
                        response,
                        materialization_budget_bytes,
                    )
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

/// Cap020 Continue + Cap019 Ignore: caller serves the full entity using these
/// validators (same metadata snapshot as the Cap020 decision — no second stat).
enum CondRangeGate {
    Handled(StaticDispatchOutcome),
    ServeFull { validators: StaticValidators },
}

/// Cap020 then Cap019 on an already Cap004-authorized filesystem path.
fn serve_authorized_path_conditional_range(
    method: StaticMethod,
    path: &std::path::Path,
    if_none_match: Option<&str>,
    if_modified_since: Option<&str>,
    range_raw: Option<&str>,
    budget: Option<u64>,
) -> Result<CondRangeGate, StaticError> {
    let meta = std::fs::metadata(path).map_err(StaticError::Io)?;
    if !meta.is_file() {
        return Err(StaticError::NotFound);
    }
    let identity = StaticResourceIdentity::from_metadata(path, &meta);
    let validators = StaticValidators::from_identity(&identity);

    // Cap020 before Cap019: matching validators → 304 even with Range present.
    if decide_conditional(if_none_match, if_modified_since, &validators)
        == ConditionalDecision::NotModified
    {
        return Ok(CondRangeGate::Handled(StaticDispatchOutcome {
            status: 304,
            headers: not_modified_headers(&validators),
            body: StaticDispatchBody::Empty,
        }));
    }

    let full_length = meta.len();
    let content_type = crate::content_type_for(path);
    let decision = decide_range(range_raw, full_length);
    let head_only = method == StaticMethod::Head;
    let Some((status, mut headers, selected_len)) =
        range_response_headers(content_type, decision, head_only)
    else {
        // Cap019 Ignore — full entity; keep Cap020 validators from this metadata.
        return Ok(CondRangeGate::ServeFull { validators });
    };
    // Cap020: validators on Cap019 responses (416/206).
    validators.append_to_headers(&mut headers);

    if status == 416 {
        return Ok(CondRangeGate::Handled(StaticDispatchOutcome {
            status: 416,
            headers,
            body: StaticDispatchBody::Empty,
        }));
    }

    // 206
    if head_only {
        return Ok(CondRangeGate::Handled(StaticDispatchOutcome {
            status: 206,
            headers,
            body: StaticDispatchBody::Empty,
        }));
    }

    let RangeDecision::Satisfied(sel) = decision else {
        return Ok(CondRangeGate::ServeFull { validators });
    };
    if let Some(limit) = budget {
        if selected_len > limit {
            return Err(StaticError::BudgetExceeded);
        }
    }
    let (bytes, _) = match crate::read_file_range(path, sel.start, selected_len) {
        Ok(v) => v,
        Err(StaticError::Io(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
            let mut headers = vec![
                ("accept-ranges".into(), "bytes".into()),
                ("content-range".into(), format!("bytes */{full_length}")),
                ("content-length".into(), "0".into()),
                ("content-type".into(), content_type.to_string()),
            ];
            validators.append_to_headers(&mut headers);
            return Ok(CondRangeGate::Handled(StaticDispatchOutcome {
                status: 416,
                headers,
                body: StaticDispatchBody::Empty,
            }));
        }
        Err(err) => return Err(err),
    };
    let body = if bytes.is_empty() {
        StaticDispatchBody::Empty
    } else {
        StaticDispatchBody::Inline(bytes)
    };
    Ok(CondRangeGate::Handled(StaticDispatchOutcome {
        status: 206,
        headers,
        body,
    }))
}

fn attach_validators(outcome: &mut StaticDispatchOutcome, validators: &StaticValidators) {
    if outcome.status != 200 && outcome.status != 206 {
        return;
    }
    if outcome
        .headers
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case("etag"))
    {
        return;
    }
    validators.append_to_headers(&mut outcome.headers);
}

/// Fallback when Cap020 gate was not entered (preload/sendfile secondary paths).
fn attach_validators_if_success(outcome: &mut StaticDispatchOutcome, path: &std::path::Path) {
    if outcome.status != 200 && outcome.status != 206 {
        return;
    }
    if outcome
        .headers
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case("etag"))
    {
        return;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if !meta.is_file() {
        return;
    }
    let identity = StaticResourceIdentity::from_metadata(path, &meta);
    attach_validators(outcome, &StaticValidators::from_identity(&identity));
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
            Err(_) => {
                return static_error_outcome(StaticError::Io(std::io::Error::other(
                    "static response body collect failed",
                )));
            }
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
