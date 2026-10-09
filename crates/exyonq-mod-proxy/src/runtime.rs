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
//! `ProxyDispatchService` runtime — Hyper upstream execution (KD3.2) + P2B WRR.

use crate::headers::{request_headers_safe_for_proxy, strip_hop_by_hop_headers};
use crate::health::{peer_key_from_http_uri, HealthSupervisor};
use crate::hyper_client::{build_incoming_client, post_body_client, ProxyClient};
use crate::hyper_forward::{
    attempt_forward_empty_body, classify_hyper_response, forward_get_streaming, forward_request,
    note_connect_retry, response_for_attempt_class, ProxyHyperMetrics,
};
use crate::retry::{
    classify_hyper_error, may_retry_connect, remaining_budget, shared_deadline,
    UpstreamAttemptClass,
};
use crate::selector::{
    endpoint_transport_identity, EndpointSelector, EndpointSpec, FailoverMode, SelectionOutcome,
};
use crate::sse::upstream_timeout_for_path;
use crate::upstream_target::UpstreamTarget;
use crate::UpstreamDescriptor;
use async_trait::async_trait;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest, ProxyDispatchService,
    ProxyMethod, ProxyMetricsSnapshot, PROXY_MAX_REQUEST_BODY_BYTES,
};
use http::Uri;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{HeaderValue, HOST};
use hyper::{Method, Request};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tracing::warn;

enum ClusterBinding {
    NoEligible,
    Single(UpstreamTarget),
    Multi {
        selector: Arc<EndpointSelector>,
        /// Parallel to `selector.endpoints()` — `None` when ineligible / unparseable.
        targets_by_index: Box<[Option<UpstreamTarget>]>,
    },
}

struct ClusterTable {
    generation: u64,
    /// Index = cluster_id.
    bindings: Box<[Option<ClusterBinding>]>,
}

impl Default for ClusterTable {
    fn default() -> Self {
        Self {
            generation: 0,
            bindings: Box::new([]),
        }
    }
}

/// Module-owned proxy runtime.
pub struct ProxyRuntime {
    clusters: RwLock<ClusterTable>,
    metrics: Arc<ProxyHyperMetrics>,
    incoming_client: ProxyClient,
    responses_503: AtomicU64,
    /// Cap024 active health supervisor (generation-scoped probe tasks).
    health: HealthSupervisor,
}

impl Default for ProxyRuntime {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_METRICS: std::sync::OnceLock<Arc<ProxyHyperMetrics>> = std::sync::OnceLock::new();

pub fn global_hyper_metrics() -> Arc<ProxyHyperMetrics> {
    GLOBAL_METRICS
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(ProxyHyperMetrics::default()))
}

impl ProxyRuntime {
    pub fn new() -> Self {
        let metrics = Arc::new(ProxyHyperMetrics::default());
        let _ = GLOBAL_METRICS.set(Arc::clone(&metrics));
        Self {
            clusters: RwLock::new(ClusterTable::default()),
            metrics,
            incoming_client: build_incoming_client().clone(),
            responses_503: AtomicU64::new(0),
            health: HealthSupervisor::new(),
        }
    }

    pub fn incoming_client(&self) -> ProxyClient {
        self.incoming_client.clone()
    }

    pub fn hyper_metrics(&self) -> Arc<ProxyHyperMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Cap024: abort health probe tasks (process shutdown / test teardown).
    pub fn shutdown_health(&self) {
        self.health.shutdown();
    }

    pub fn bind_compiled_slots(&self, generation: u64, slots: &[ProxyCompiledSlot]) {
        let mut prior: HashMap<String, UpstreamTarget> = HashMap::new();
        if let Ok(prev) = self.clusters.read() {
            for binding in prev.bindings.iter().flatten() {
                match binding {
                    ClusterBinding::Single(t) => {
                        if let Some(k) = endpoint_transport_identity_from_target(t) {
                            prior.insert(k, t.clone());
                        }
                    }
                    ClusterBinding::Multi {
                        targets_by_index, ..
                    } => {
                        for t in targets_by_index.iter().flatten() {
                            if let Some(k) = endpoint_transport_identity_from_target(t) {
                                prior.insert(k, t.clone());
                            }
                        }
                    }
                    ClusterBinding::NoEligible => {}
                }
            }
        }

        let len = slots
            .iter()
            .map(|s| s.cluster_id as usize + 1)
            .max()
            .unwrap_or(0);
        let mut built: Vec<Option<ClusterBinding>> =
            (0..len.max(slots.len())).map(|_| None).collect();
        for slot in slots {
            let idx = slot.cluster_id as usize;
            if idx >= built.len() {
                built.extend((built.len()..=idx).map(|_| None));
            }
            built[idx] = Some(build_cluster_binding(generation, slot, &prior));
        }
        // Cap024: publish cluster bindings BEFORE health views so a reload that
        // removes a peer cannot race (old bindings + new health map fail-open).
        let mut clusters = self.clusters.write().expect("proxy clusters poisoned");
        clusters.generation = generation;
        clusters.bindings = built.into_boxed_slice();
        drop(clusters);

        let mut health_clusters = Vec::new();
        for slot in slots {
            if !slot.health_check.enabled {
                continue;
            }
            let mut peers = Vec::new();
            for ep in slot.endpoints.iter() {
                if !(ep.admin_enabled && ep.weight > 0) {
                    continue;
                }
                if let Some(key) = peer_key_from_http_uri(&ep.http_uri) {
                    peers.push((key, ep.http_uri.clone()));
                }
            }
            if peers.is_empty() && !slot.target.is_empty() {
                if let Some(key) = peer_key_from_http_uri(&slot.target) {
                    peers.push((key, slot.target.clone()));
                }
            }
            health_clusters.push((slot.cluster_id, slot.health_check.clone(), peers));
        }
        self.health.replace_generation(generation, &health_clusters);
    }

    pub fn bind_upstream_targets(&self, generation: u64, targets: &[UpstreamTarget]) {
        let mut clusters = self.clusters.write().expect("proxy clusters poisoned");
        clusters.generation = generation;
        clusters.bindings = targets
            .iter()
            .cloned()
            .map(|t| Some(ClusterBinding::Single(t)))
            .collect::<Vec<_>>()
            .into_boxed_slice();
    }

    pub fn upstream_for_cluster(&self, cluster_id: u32) -> Option<UpstreamTarget> {
        match self.resolve(cluster_id) {
            Resolve::Selected(t) => Some(t),
            Resolve::NoEligible | Resolve::Missing => None,
        }
    }

    pub fn cluster_generation(&self) -> u64 {
        self.clusters
            .read()
            .map(|table| table.generation)
            .unwrap_or(0)
    }

    fn resolve(&self, cluster_id: u32) -> Resolve {
        self.resolve_excluding(cluster_id, None)
    }

    /// Cap021: on connect-only retry, skip the peer that just failed (request-local).
    /// Cap024: skip peers marked UNHEALTHY by active health (cross-request).
    fn resolve_excluding(&self, cluster_id: u32, exclude_peer: Option<&str>) -> Resolve {
        let health = self.health.view(cluster_id);
        let peer_ok = |peer_key: &str| -> bool {
            health
                .as_ref()
                .map(|h| h.is_selectable(peer_key))
                .unwrap_or(true)
        };
        let Ok(table) = self.clusters.read() else {
            return Resolve::Missing;
        };
        match table.bindings.get(cluster_id as usize) {
            None | Some(None) => Resolve::Missing,
            Some(Some(ClusterBinding::NoEligible)) => Resolve::NoEligible,
            Some(Some(ClusterBinding::Single(t))) => {
                if !peer_ok(&t.peer_key) {
                    return Resolve::NoEligible;
                }
                if exclude_peer.is_some_and(|k| k == t.peer_key) {
                    // Sole peer failed connect — still return it so final attempt can map 502.
                    // Caller stops retrying when may_retry is false / same peer only.
                }
                Resolve::Selected(t.clone())
            }
            Some(Some(ClusterBinding::Multi {
                selector,
                targets_by_index,
            })) => {
                // Cap045 / LA-CAP045-001: WRR among currently selectable peers only.
                // Do not charge SWWR on Cap024-unhealthy / Cap021-excluded peers, and
                // do not fall back to first-match scan among remaining healthy peers.
                match selector.select_filtered(|endpoint_index| {
                    let Some(t) = targets_by_index
                        .get(endpoint_index)
                        .and_then(|slot| slot.as_ref())
                    else {
                        return false;
                    };
                    if exclude_peer.is_some_and(|k| k == t.peer_key) {
                        return false;
                    }
                    peer_ok(&t.peer_key)
                }) {
                    SelectionOutcome::NoEligibleEndpoint => Resolve::NoEligible,
                    SelectionOutcome::Selected { endpoint_index, .. } => {
                        match targets_by_index.get(endpoint_index).and_then(|t| t.clone()) {
                            Some(t) => Resolve::Selected(t),
                            None => Resolve::NoEligible,
                        }
                    }
                }
            }
        }
    }

    /// Default: do **not** trust inbound `X-Forwarded-For` — use peer `remote_addr` only.
    fn xff_from_request(request: &ProxyDispatchRequest) -> Option<HeaderValue> {
        HeaderValue::from_str(&request.remote_addr).ok()
    }

    fn xfp_from_request(request: &ProxyDispatchRequest) -> Option<HeaderValue> {
        if request.scheme.is_empty() {
            None
        } else {
            HeaderValue::from_str(&request.scheme).ok()
        }
    }

    fn hyper_method(method: ProxyMethod) -> Method {
        match method {
            ProxyMethod::Get => Method::GET,
            ProxyMethod::Head => Method::HEAD,
            ProxyMethod::Post => Method::POST,
            ProxyMethod::Put => Method::PUT,
            ProxyMethod::Patch => Method::PATCH,
            ProxyMethod::Delete => Method::DELETE,
            ProxyMethod::Options => Method::OPTIONS,
            ProxyMethod::Other => Method::GET,
        }
    }

    async fn attempt_get_like(
        &self,
        target: &UpstreamTarget,
        request: &ProxyDispatchRequest,
        budget: std::time::Duration,
    ) -> Result<ProxyDispatchOutcome, UpstreamAttemptClass> {
        // GET and HEAD must carry the same headers as POST. The empty-body
        // fast path sent the upstream's own Host and dropped Cookie.
        self.attempt_post_like(target, request, budget).await
    }

    async fn attempt_post_like(
        &self,
        target: &UpstreamTarget,
        request: &ProxyDispatchRequest,
        budget: std::time::Duration,
    ) -> Result<ProxyDispatchOutcome, UpstreamAttemptClass> {
        if budget.is_zero() {
            return Err(UpstreamAttemptClass::TimedOut);
        }
        let body = request.body.clone().unwrap_or_default();
        if body.len() > PROXY_MAX_REQUEST_BODY_BYTES {
            return Ok(ProxyDispatchOutcome::BadGateway);
        }

        let mut header_map = hyper::HeaderMap::new();
        for (name, value) in &request.headers {
            if let (Ok(n), Ok(v)) = (
                hyper::header::HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                header_map.insert(n, v);
            }
        }
        if !request_headers_safe_for_proxy(&header_map) {
            return Ok(ProxyDispatchOutcome::BadGateway);
        }

        let path_and_query = request.path_and_query.clone();
        let mut builder = Request::builder()
            .method(Self::hyper_method(request.method))
            .uri(target.uri_for(&path_and_query));

        for (name, value) in &request.headers {
            if name.eq_ignore_ascii_case("x-forwarded-for")
                || name.eq_ignore_ascii_case("x-forwarded-proto")
                || name.eq_ignore_ascii_case("x-forwarded-host")
                || name.eq_ignore_ascii_case("forwarded")
            {
                continue;
            }
            builder = builder.header(name.as_str(), value.as_str());
        }
        let client_sent_host = request
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("host"));
        if !client_sent_host {
            if let Some(host) = &request.host {
                builder = builder.header("host", host.as_str());
            } else if let Some(host) = &target.host {
                builder = builder.header(HOST, host.clone());
            }
        }

        let mut req = match builder.body(Full::from(body)) {
            Ok(req) => req,
            Err(err) => {
                warn!(%err, "proxy request builder failed (operator/network headers)");
                return Ok(ProxyDispatchOutcome::BadGateway);
            }
        };
        {
            let headers = req.headers_mut();
            headers.remove("x-forwarded-for");
            headers.remove("x-forwarded-proto");
            headers.remove("x-forwarded-host");
            headers.remove("forwarded");
            // Strip before re-injecting intermediary identity headers so Connection
            // cannot nominate-and-wipe XFF / Host after injection.
            strip_hop_by_hop_headers(headers);
            if !client_sent_host {
                if let Some(host) = &request.host {
                    if let Ok(v) = HeaderValue::from_str(host.as_str()) {
                        headers.insert(HOST, v);
                    }
                } else if let Some(host) = &target.host {
                    headers.insert(HOST, host.clone());
                }
            }
            if let Some(xff) = Self::xff_from_request(request) {
                headers.insert("x-forwarded-for", xff);
            }
            if let Some(xfp) = Self::xfp_from_request(request) {
                headers.insert("x-forwarded-proto", xfp);
            }
        }

        let upstream_req = post_body_client().request(req);
        match tokio::time::timeout(budget, upstream_req).await {
            Ok(Ok(resp)) => {
                use http_body_util::BodyExt;
                let (mut parts, body) = resp.into_parts();
                strip_hop_by_hop_headers(&mut parts.headers);
                let response: hyper::Response<
                    http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>,
                > = hyper::Response::from_parts(parts, body.boxed());
                Ok(classify_hyper_response(response, &path_and_query, request.method).await)
            }
            Ok(Err(err)) => {
                let class = classify_hyper_error(&err);
                warn!(%err, ?class, "upstream error");
                Err(class)
            }
            Err(_) => {
                warn!(budget_ms = budget.as_millis(), "upstream attempt timeout");
                Err(UpstreamAttemptClass::TimedOut)
            }
        }
    }

    fn outcome_for_class(&self, class: UpstreamAttemptClass) -> ProxyDispatchOutcome {
        match class {
            UpstreamAttemptClass::TimedOut => {
                self.metrics.responses_504.fetch_add(1, Ordering::Relaxed);
                ProxyDispatchOutcome::GatewayTimeout
            }
            UpstreamAttemptClass::ConnectFailed | UpstreamAttemptClass::UnretryableError => {
                self.metrics.responses_502.fetch_add(1, Ordering::Relaxed);
                ProxyDispatchOutcome::BadGateway
            }
        }
    }

    /// Forward a live Hyper request (WebSocket upgrade path — preserves `Incoming` body).
    pub async fn forward_incoming_request(
        &self,
        target: &UpstreamTarget,
        req: Request<Incoming>,
        x_forwarded_for: Option<&HeaderValue>,
    ) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>> {
        forward_request(
            &self.incoming_client,
            target,
            req,
            x_forwarded_for,
            &self.metrics,
        )
        .await
    }
}

enum Resolve {
    Selected(UpstreamTarget),
    NoEligible,
    Missing,
}

fn endpoint_transport_identity_from_target(t: &UpstreamTarget) -> Option<String> {
    let uri = t.uri_for("/");
    let scheme = uri.scheme_str().unwrap_or("http");
    let host = uri.host()?;
    let port = uri
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some(format!("{scheme}://{host}:{port}"))
}

fn build_cluster_binding(
    generation: u64,
    slot: &ProxyCompiledSlot,
    prior: &HashMap<String, UpstreamTarget>,
) -> ClusterBinding {
    if slot.single_endpoint_executable && !slot.target.is_empty() {
        let key = endpoint_transport_identity(&slot.target);
        if let Some(key) = key.as_ref() {
            if let Some(existing) = prior.get(key) {
                let mut reused = existing.clone();
                reused.timeout = slot.timeout;
                reused.max_connect_retries = slot.max_connect_retries;
                return ClusterBinding::Single(reused);
            }
        }
        let host = slot
            .target
            .parse::<Uri>()
            .ok()
            .and_then(|u| u.host().map(str::to_string));
        let desc = UpstreamDescriptor {
            cluster_id: slot.cluster_id,
            upstream_name: slot.upstream_name.clone(),
            target: slot.target.clone(),
            timeout: slot.timeout,
            host,
            max_connect_retries: slot.max_connect_retries,
        };
        return match UpstreamTarget::from_descriptor(&desc) {
            Ok(target) => ClusterBinding::Single(target),
            Err(err) => {
                warn!(
                    %err,
                    cluster_id = slot.cluster_id,
                    target = %slot.target,
                    "compiled proxy slot failed to parse; binding NoEligible"
                );
                ClusterBinding::NoEligible
            }
        };
    }

    if slot.multi_endpoint_executable && !slot.endpoints.is_empty() {
        let specs: Vec<EndpointSpec> = slot
            .endpoints
            .iter()
            .map(|ep| EndpointSpec {
                endpoint_id: ep.endpoint_id.clone(),
                http_uri: ep.http_uri.clone(),
                weight: ep.weight,
                priority: ep.priority,
                admin_enabled: ep.admin_enabled,
            })
            .collect();
        let failover = if slot.failover_priority_bands {
            FailoverMode::PriorityBands
        } else {
            FailoverMode::None
        };
        let selector = Arc::new(EndpointSelector::build(generation, failover, specs));
        let mut targets_by_index: Vec<Option<UpstreamTarget>> = vec![None; slot.endpoints.len()];
        let mut any = false;
        let mut sole: Option<UpstreamTarget> = None;
        let mut eligible_count = 0usize;
        for (i, ep) in slot.endpoints.iter().enumerate() {
            if !(ep.admin_enabled && ep.weight > 0) {
                continue;
            }
            let Some(key) = endpoint_transport_identity(&ep.http_uri) else {
                continue;
            };
            let target = if let Some(existing) = prior.get(&key) {
                let mut reused = existing.clone();
                reused.timeout = slot.timeout;
                reused.max_connect_retries = slot.max_connect_retries;
                reused
            } else {
                let host = ep
                    .http_uri
                    .parse::<Uri>()
                    .ok()
                    .and_then(|u| u.host().map(str::to_string));
                let desc = UpstreamDescriptor {
                    cluster_id: slot.cluster_id,
                    upstream_name: slot.upstream_name.clone(),
                    target: ep.http_uri.clone(),
                    timeout: slot.timeout,
                    host,
                    max_connect_retries: slot.max_connect_retries,
                };
                match UpstreamTarget::from_descriptor(&desc) {
                    Ok(t) => t,
                    Err(_) => continue,
                }
            };
            targets_by_index[i] = Some(target.clone());
            any = true;
            eligible_count += 1;
            sole = Some(target);
        }
        if !any {
            return ClusterBinding::NoEligible;
        }
        // OPEN-002 defense-in-depth: effective N=1 uses Single even if multi flag set.
        if eligible_count == 1 {
            // Structural: eligible_count==1 implies `sole` was set in the loop above.
            return match sole {
                Some(target) => ClusterBinding::Single(target),
                None => ClusterBinding::NoEligible,
            };
        }
        return ClusterBinding::Multi {
            selector,
            targets_by_index: targets_by_index.into_boxed_slice(),
        };
    }

    // SILENT_ENDPOINT_TRUNCATION = NO / FIRST_ENDPOINT_FALLBACK = FORBIDDEN
    ClusterBinding::NoEligible
}

#[async_trait]
impl ProxyDispatchService for ProxyRuntime {
    async fn dispatch(&self, request: ProxyDispatchRequest) -> ProxyDispatchOutcome {
        let first = match self.resolve(request.cluster_id) {
            Resolve::Selected(t) => t,
            Resolve::NoEligible | Resolve::Missing => {
                self.responses_503.fetch_add(1, Ordering::Relaxed);
                return ProxyDispatchOutcome::ServiceUnavailable;
            }
        };
        let total_timeout = upstream_timeout_for_path(first.timeout, &request.path_and_query);
        let deadline = shared_deadline(total_timeout);
        let max_retries = first.max_connect_retries;
        let mut retries_used: u8 = 0;
        let mut target = first;

        loop {
            let budget = remaining_budget(deadline);
            // timeout_ms = 0 is already elapsed. Do not poll the upstream:
            // a zero tokio timeout can still observe a ready localhost future.
            if budget.is_zero() {
                return self.outcome_for_class(UpstreamAttemptClass::TimedOut);
            }
            let attempt = match request.method {
                ProxyMethod::Get | ProxyMethod::Head => {
                    self.attempt_get_like(&target, &request, budget).await
                }
                ProxyMethod::Post
                | ProxyMethod::Put
                | ProxyMethod::Patch
                | ProxyMethod::Delete
                | ProxyMethod::Options
                | ProxyMethod::Other => self.attempt_post_like(&target, &request, budget).await,
            };
            match attempt {
                Ok(outcome) => return outcome,
                Err(class) => {
                    let remain = remaining_budget(deadline);
                    if may_retry_connect(max_retries, retries_used, class, remain) {
                        let failed_peer = target.peer_key.clone();
                        target = match self
                            .resolve_excluding(request.cluster_id, Some(failed_peer.as_str()))
                        {
                            Resolve::Selected(t) => t,
                            Resolve::NoEligible | Resolve::Missing => {
                                return self.outcome_for_class(class);
                            }
                        };
                        note_connect_retry(&self.metrics);
                        retries_used = retries_used.saturating_add(1);
                        warn!(
                            retries_used,
                            peer = %failed_peer,
                            "cap021 connect-only retry"
                        );
                        continue;
                    }
                    return self.outcome_for_class(class);
                }
            }
        }
    }

    fn metrics(&self) -> ProxyMetricsSnapshot {
        ProxyMetricsSnapshot {
            responses_502: self.metrics.responses_502.load(Ordering::Relaxed),
            responses_503: self.responses_503.load(Ordering::Relaxed),
            responses_504: self.metrics.responses_504.load(Ordering::Relaxed),
            connect_retries: self.metrics.connect_retries.load(Ordering::Relaxed),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    fn bind_compiled_slots(&self, generation: u64, slots: &[ProxyCompiledSlot]) {
        ProxyRuntime::bind_compiled_slots(self, generation, slots);
    }
}

impl ProxyRuntime {
    /// Streaming GET for Plan 12 proxy cache load (returns live Hyper response).
    pub async fn forward_get_streaming_for_cache(
        &self,
        cluster_id: u32,
        path_and_query: &str,
        x_forwarded_for: Option<&HeaderValue>,
    ) -> Option<hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>>>
    {
        let target = self.upstream_for_cluster(cluster_id)?;
        Some(forward_get_streaming(&target, path_and_query, x_forwarded_for, &self.metrics).await)
    }
}

/// Cache-eligible GET load by compiled cluster id (KD3.6 — no UpstreamTarget in core).
pub async fn load_get_for_cache_by_cluster(
    cluster_id: u32,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    max_object_bytes: usize,
    request_headers: &[(String, String)],
) -> crate::proxy_cache::ProxyCacheLoad {
    use crate::proxy_cache::ProxyCacheLoad;
    use exyonq_module_api::CacheRejection;

    let rt = crate::wire_conn::pinned_runtime();
    let Some(response) = rt
        .forward_get_streaming_for_cache(cluster_id, path_and_query, x_forwarded_for)
        .await
    else {
        return ProxyCacheLoad::Passthrough {
            response: crate::hyper_forward::service_unavailable(),
            rejection: CacheRejection::BodyNotMaterialized,
        };
    };
    crate::proxy_cache::prepare_proxy_cache_load(response, max_object_bytes, request_headers).await
}

/// WebSocket upgrade by compiled cluster id (KD3.6 — core passes cluster_id only).
pub async fn forward_websocket_by_cluster(
    client: &ProxyClient,
    cluster_id: u32,
    req: hyper::Request<hyper::body::Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>> {
    let rt = crate::wire_conn::pinned_runtime();
    let Some(target) = rt.upstream_for_cluster(cluster_id) else {
        return crate::hyper_forward::service_unavailable();
    };
    crate::websocket::forward_websocket(
        client,
        &target,
        req,
        x_forwarded_for,
        rt.hyper_metrics().as_ref(),
    )
    .await
}

impl ProxyRuntime {
    pub async fn forward_get_for_wire(
        &self,
        cluster_id: u32,
        target: &UpstreamTarget,
        path_and_query: &str,
        x_forwarded_for: Option<&HeaderValue>,
    ) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>> {
        // Cap021: wire GET shares connect-only retry + peer exclusion + deadline with dispatch.
        let total = upstream_timeout_for_path(target.timeout, path_and_query);
        let deadline = shared_deadline(total);
        let max_retries = target.max_connect_retries;
        let mut retries_used = 0u8;

        // Happy path: borrow `target` — no UpstreamTarget/HeaderValue clone until a connect retry.
        let budget = remaining_budget(deadline);
        match attempt_forward_empty_body(
            target,
            path_and_query,
            Method::GET,
            x_forwarded_for,
            None,
            &self.metrics,
            budget,
        )
        .await
        {
            Ok(resp) => resp,
            Err(class) => {
                let remain = remaining_budget(deadline);
                if !may_retry_connect(max_retries, retries_used, class, remain) {
                    return response_for_attempt_class(class, &self.metrics);
                }
                let failed_peer = target.peer_key.clone();
                let mut current =
                    match self.resolve_excluding(cluster_id, Some(failed_peer.as_str())) {
                        Resolve::Selected(t) => t,
                        Resolve::NoEligible | Resolve::Missing => {
                            return response_for_attempt_class(class, &self.metrics);
                        }
                    };
                note_connect_retry(&self.metrics);
                retries_used = retries_used.saturating_add(1);
                warn!(
                    retries_used,
                    peer = %failed_peer,
                    "cap021 connect-only retry"
                );

                loop {
                    let budget = remaining_budget(deadline);
                    match attempt_forward_empty_body(
                        &current,
                        path_and_query,
                        Method::GET,
                        x_forwarded_for,
                        None,
                        &self.metrics,
                        budget,
                    )
                    .await
                    {
                        Ok(resp) => return resp,
                        Err(class) => {
                            let remain = remaining_budget(deadline);
                            if may_retry_connect(max_retries, retries_used, class, remain) {
                                let failed_peer = current.peer_key.clone();
                                current = match self
                                    .resolve_excluding(cluster_id, Some(failed_peer.as_str()))
                                {
                                    Resolve::Selected(t) => t,
                                    Resolve::NoEligible | Resolve::Missing => {
                                        return response_for_attempt_class(class, &self.metrics);
                                    }
                                };
                                note_connect_retry(&self.metrics);
                                retries_used = retries_used.saturating_add(1);
                                warn!(
                                    retries_used,
                                    peer = %failed_peer,
                                    "cap021 connect-only retry"
                                );
                                continue;
                            }
                            return response_for_attempt_class(class, &self.metrics);
                        }
                    }
                }
            }
        }
    }
}
