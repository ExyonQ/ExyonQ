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
//! `ProxyDispatchService` runtime — Hyper upstream execution (KD3.2).

use crate::headers::{request_headers_safe_for_proxy, strip_hop_by_hop_headers};
use crate::hyper_client::{build_incoming_client, post_body_client, ProxyClient};
use crate::hyper_forward::{
    classify_hyper_response, forward_get, forward_get_streaming, forward_request, ProxyHyperMetrics,
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
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use tracing::warn;

struct ClusterTable {
    generation: u64,
    targets: Box<[UpstreamTarget]>,
}

impl Default for ClusterTable {
    fn default() -> Self {
        Self {
            generation: 0,
            targets: Box::new([]),
        }
    }
}

/// Module-owned proxy runtime.
pub struct ProxyRuntime {
    clusters: RwLock<ClusterTable>,
    metrics: Arc<ProxyHyperMetrics>,
    incoming_client: ProxyClient,
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
        }
    }

    pub fn incoming_client(&self) -> ProxyClient {
        self.incoming_client.clone()
    }

    pub fn hyper_metrics(&self) -> Arc<ProxyHyperMetrics> {
        Arc::clone(&self.metrics)
    }

    pub fn bind_compiled_slots(&self, generation: u64, slots: &[ProxyCompiledSlot]) {
        let mut built = Vec::with_capacity(slots.len());
        for slot in slots {
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
            built.push(
                UpstreamTarget::from_descriptor(&desc).expect("compiled proxy slot must parse"),
            );
        }
        let mut clusters = self.clusters.write().expect("proxy clusters poisoned");
        clusters.generation = generation;
        clusters.targets = built.into_boxed_slice();
    }

    pub fn bind_upstream_targets(&self, generation: u64, targets: Box<[UpstreamTarget]>) {
        let mut clusters = self.clusters.write().expect("proxy clusters poisoned");
        clusters.generation = generation;
        clusters.targets = targets;
    }

    pub fn upstream_for_cluster(&self, cluster_id: u32) -> Option<UpstreamTarget> {
        self.target_for(cluster_id)
    }

    pub fn cluster_generation(&self) -> u64 {
        self.clusters
            .read()
            .map(|table| table.generation)
            .unwrap_or(0)
    }

    fn target_for(&self, cluster_id: u32) -> Option<UpstreamTarget> {
        self.clusters
            .read()
            .ok()?
            .targets
            .get(cluster_id as usize)
            .cloned()
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
                target.clear_api_cache(&path_and_query);
                self.metrics.responses_502.fetch_add(1, Ordering::Relaxed);
                ProxyDispatchOutcome::BadGateway
            }
            Err(_) => {
                warn!(timeout_ms = target.timeout.as_millis(), "upstream timeout");
                target.clear_api_cache(&path_and_query);
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

#[async_trait]
impl ProxyDispatchService for ProxyRuntime {
    async fn dispatch(&self, request: ProxyDispatchRequest) -> ProxyDispatchOutcome {
        let Some(target) = self.target_for(request.cluster_id) else {
            return ProxyDispatchOutcome::BadGateway;
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
            responses_503: 0,
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
        let target = self.target_for(cluster_id)?;
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
            response: crate::hyper_forward::bad_gateway(),
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
        return crate::hyper_forward::bad_gateway();
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
