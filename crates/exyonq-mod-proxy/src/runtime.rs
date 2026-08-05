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
use crate::hyper_client::{build_incoming_client, post_body_client, ProxyClient};
use crate::hyper_forward::{
    classify_hyper_response, forward_get, forward_get_streaming, forward_request, ProxyHyperMetrics,
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
            incoming_client: build_incoming_client(),
            responses_503: AtomicU64::new(0),
        }
    }

    pub fn incoming_client(&self) -> ProxyClient {
        self.incoming_client.clone()
    }

    pub fn hyper_metrics(&self) -> Arc<ProxyHyperMetrics> {
        Arc::clone(&self.metrics)
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
        let mut clusters = self.clusters.write().expect("proxy clusters poisoned");
        clusters.generation = generation;
        clusters.bindings = built.into_boxed_slice();
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
        let Ok(table) = self.clusters.read() else {
            return Resolve::Missing;
        };
        match table.bindings.get(cluster_id as usize) {
            None | Some(None) => Resolve::Missing,
            Some(Some(ClusterBinding::NoEligible)) => Resolve::NoEligible,
            Some(Some(ClusterBinding::Single(t))) => Resolve::Selected(t.clone()),
            Some(Some(ClusterBinding::Multi {
                selector,
                targets_by_index,
            })) => match selector.select() {
                SelectionOutcome::NoEligibleEndpoint => Resolve::NoEligible,
                SelectionOutcome::Selected { endpoint_index, .. } => targets_by_index
                    .get(endpoint_index)
                    .and_then(|t| t.clone())
                    .map(Resolve::Selected)
                    .unwrap_or(Resolve::NoEligible),
            },
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

    async fn dispatch_get_like(
        &self,
        target: &UpstreamTarget,
        request: &ProxyDispatchRequest,
        streaming: bool,
    ) -> ProxyDispatchOutcome {
        let xff = Self::xff_from_request(request);
        let xfp = Self::xfp_from_request(request);
        let response = if streaming {
            crate::hyper_forward::forward_get_streaming_with_proto(
                target,
                &request.path_and_query,
                xff.as_ref(),
                xfp.as_ref(),
                &self.metrics,
            )
            .await
        } else if request.method == ProxyMethod::Head {
            crate::hyper_forward::forward_head_with_proto(
                target,
                &request.path_and_query,
                xff.as_ref(),
                xfp.as_ref(),
                &self.metrics,
            )
            .await
        } else {
            crate::hyper_forward::forward_get_with_proto(
                target,
                &request.path_and_query,
                xff.as_ref(),
                xfp.as_ref(),
                &self.metrics,
            )
            .await
        };
        classify_hyper_response(response, &request.path_and_query, request.method).await
    }

    async fn dispatch_post_like(
        &self,
        target: &UpstreamTarget,
        request: &ProxyDispatchRequest,
    ) -> ProxyDispatchOutcome {
        let body = request.body.clone().unwrap_or_default();
        if body.len() > PROXY_MAX_REQUEST_BODY_BYTES {
            return ProxyDispatchOutcome::BadGateway;
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
            return ProxyDispatchOutcome::BadGateway;
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
        if let Some(host) = &request.host {
            builder = builder.header("host", host.as_str());
        } else if let Some(host) = &target.host {
            builder = builder.header(HOST, host.clone());
        }

        let mut req = builder
            .body(Full::from(body))
            .expect("valid proxy body request");
        {
            let headers = req.headers_mut();
            headers.remove("x-forwarded-for");
            headers.remove("x-forwarded-proto");
            headers.remove("x-forwarded-host");
            headers.remove("forwarded");
            if let Some(xff) = Self::xff_from_request(request) {
                headers.insert("x-forwarded-for", xff);
            }
            if let Some(xfp) = Self::xfp_from_request(request) {
                headers.insert("x-forwarded-proto", xfp);
            }
        }

        let timeout = upstream_timeout_for_path(target.timeout, &path_and_query);
        let upstream_req = post_body_client().request(req);
        match tokio::time::timeout(timeout, upstream_req).await {
            Ok(Ok(resp)) => {
                use http_body_util::BodyExt;
                let (mut parts, body) = resp.into_parts();
                strip_hop_by_hop_headers(&mut parts.headers);
                let response: hyper::Response<
                    http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>,
                > = hyper::Response::from_parts(parts, body.boxed());
                classify_hyper_response(response, &path_and_query, request.method).await
            }
            Ok(Err(err)) => {
                warn!(%err, "upstream error");
                self.metrics.responses_502.fetch_add(1, Ordering::Relaxed);
                ProxyDispatchOutcome::BadGateway
            }
            Err(_) => {
                warn!(timeout_ms = target.timeout.as_millis(), "upstream timeout");
                self.metrics.responses_504.fetch_add(1, Ordering::Relaxed);
                ProxyDispatchOutcome::GatewayTimeout
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
        };
        return ClusterBinding::Single(
            UpstreamTarget::from_descriptor(&desc).expect("compiled proxy slot must parse"),
        );
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
            return ClusterBinding::Single(sole.expect("eligible_count==1"));
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
        let target = match self.resolve(request.cluster_id) {
            Resolve::Selected(t) => t,
            Resolve::NoEligible | Resolve::Missing => {
                self.responses_503.fetch_add(1, Ordering::Relaxed);
                return ProxyDispatchOutcome::ServiceUnavailable;
            }
        };

        match request.method {
            ProxyMethod::Get => self.dispatch_get_like(&target, &request, false).await,
            ProxyMethod::Head => self.dispatch_get_like(&target, &request, false).await,
            ProxyMethod::Post | ProxyMethod::Put | ProxyMethod::Patch | ProxyMethod::Delete => {
                self.dispatch_post_like(&target, &request).await
            }
            ProxyMethod::Options | ProxyMethod::Other => {
                self.dispatch_post_like(&target, &request).await
            }
        }
    }

    fn metrics(&self) -> ProxyMetricsSnapshot {
        ProxyMetricsSnapshot {
            responses_502: self.metrics.responses_502.load(Ordering::Relaxed),
            responses_503: self.responses_503.load(Ordering::Relaxed),
            responses_504: self.metrics.responses_504.load(Ordering::Relaxed),
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
        target: &UpstreamTarget,
        path_and_query: &str,
        x_forwarded_for: Option<&HeaderValue>,
    ) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>> {
        forward_get(target, path_and_query, x_forwarded_for, &self.metrics).await
    }
}
