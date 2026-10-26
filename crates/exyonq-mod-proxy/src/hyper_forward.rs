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
//! Hyper upstream forward paths (module-owned runtime).

use crate::attach::{store_streaming_response, store_websocket_response};
use crate::hyper_client::{get_empty_body_client, ProxyClient};
use crate::retry::{classify_hyper_error, UpstreamAttemptClass};
use crate::upstream_target::UpstreamTarget;
use crate::{
    content_type_is_event_stream, request_headers_safe_for_proxy, strip_hop_by_hop_headers,
    upstream_timeout_for_path,
};
use exyonq_module_api::proxy_dispatch::{
    ProxyDispatchOutcome, ProxyMaterializedResponse, ProxyMethod, ProxyStreamHandle,
    ProxyWebSocketHandle,
};
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, HOST};
use hyper::{Method, Request, Response};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::warn;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

pub struct ProxyHyperMetrics {
    pub responses_502: AtomicU64,
    pub responses_504: AtomicU64,
    /// Cap021: connect-only retries that were issued (not final failures).
    pub connect_retries: AtomicU64,
    /// Responses classified or forwarded as SSE / event-stream.
    pub sse_streams: AtomicU64,
}

impl Default for ProxyHyperMetrics {
    fn default() -> Self {
        Self {
            responses_502: AtomicU64::new(0),
            responses_504: AtomicU64::new(0),
            connect_retries: AtomicU64::new(0),
            sse_streams: AtomicU64::new(0),
        }
    }
}

pub fn note_connect_retry(metrics: &ProxyHyperMetrics) {
    metrics.connect_retries.fetch_add(1, Ordering::Relaxed);
}

/// Map a final Cap021 attempt class to a client-facing Hyper error response.
pub fn response_for_attempt_class(
    class: UpstreamAttemptClass,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    response_for_attempt_class_ctx(class, metrics, None)
}

/// Like [`response_for_attempt_class`], emitting Q-02 `EXY-RT-*` with route context when given.
pub fn response_for_attempt_class_ctx(
    class: UpstreamAttemptClass,
    metrics: &ProxyHyperMetrics,
    ctx: Option<&crate::runtime_diag::RtFailCtx<'_>>,
) -> Response<BoxBody> {
    let fallback = crate::runtime_diag::RtFailCtx {
        cluster_id: 0,
        path: "",
        peer: None,
        host: None,
        detail: None,
    };
    crate::runtime_diag::emit_upstream_class(ctx.unwrap_or(&fallback), class);
    match class {
        UpstreamAttemptClass::TimedOut => {
            note_504(metrics);
            gateway_timeout()
        }
        UpstreamAttemptClass::ConnectFailed | UpstreamAttemptClass::UnretryableError => {
            note_502(metrics);
            bad_gateway()
        }
    }
}

pub fn response_is_event_stream(parts: &http::response::Parts) -> bool {
    let content_type = parts
        .headers
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    content_type_is_event_stream(content_type)
}

pub fn bad_request(message: &str) -> Response<BoxBody> {
    Response::builder()
        .status(400)
        .header("content-type", "text/plain; charset=utf-8")
        .body(
            Full::from(bytes::Bytes::from(message.to_string()))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid 400 response")
}

pub fn bad_gateway() -> Response<BoxBody> {
    Response::builder()
        .status(502)
        .body(
            Full::<bytes::Bytes>::from(bytes::Bytes::new())
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid 502 response")
}

/// No eligible upstream endpoint (empty set / all disabled / unbound cluster).
pub fn service_unavailable() -> Response<BoxBody> {
    Response::builder()
        .status(503)
        .body(
            Full::<bytes::Bytes>::from(bytes::Bytes::new())
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid 503 response")
}

pub fn gateway_timeout() -> Response<BoxBody> {
    Response::builder()
        .status(504)
        .body(
            Full::<bytes::Bytes>::from(bytes::Bytes::new())
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid 504 response")
}

fn note_502(metrics: &ProxyHyperMetrics) {
    metrics.responses_502.fetch_add(1, Ordering::Relaxed);
}

fn note_504(metrics: &ProxyHyperMetrics) {
    metrics.responses_504.fetch_add(1, Ordering::Relaxed);
}

fn headers_to_pairs(headers: &hyper::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

pub fn materialize_parts(
    parts: http::response::Parts,
    body: bytes::Bytes,
) -> ProxyMaterializedResponse {
    ProxyMaterializedResponse {
        status: parts.status.as_u16(),
        headers: headers_to_pairs(&parts.headers),
        body: body.to_vec(),
    }
}

pub async fn forward_get(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    forward_get_with_proto(upstream, path_and_query, x_forwarded_for, None, metrics).await
}

pub async fn forward_get_with_proto(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    x_forwarded_proto: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    forward_empty_body(
        upstream,
        path_and_query,
        Method::GET,
        x_forwarded_for,
        x_forwarded_proto,
        metrics,
    )
    .await
}

pub async fn forward_head(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    forward_head_with_proto(upstream, path_and_query, x_forwarded_for, None, metrics).await
}

pub async fn forward_head_with_proto(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    x_forwarded_proto: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    forward_empty_body(
        upstream,
        path_and_query,
        Method::HEAD,
        x_forwarded_for,
        x_forwarded_proto,
        metrics,
    )
    .await
}

async fn forward_empty_body(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    method: Method,
    x_forwarded_for: Option<&HeaderValue>,
    x_forwarded_proto: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    let timeout = upstream_timeout_for_path(upstream.timeout, path_and_query);
    match attempt_forward_empty_body(
        upstream,
        path_and_query,
        method,
        x_forwarded_for,
        x_forwarded_proto,
        metrics,
        timeout,
    )
    .await
    {
        Ok(resp) => resp,
        Err(class) => response_for_attempt_class(class, metrics),
    }
}

/// Cap021: one attempt under an explicit remaining budget (shared deadline).
pub async fn attempt_forward_empty_body(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    method: Method,
    x_forwarded_for: Option<&HeaderValue>,
    x_forwarded_proto: Option<&HeaderValue>,
    _metrics: &ProxyHyperMetrics,
    budget: Duration,
) -> Result<Response<BoxBody>, UpstreamAttemptClass> {
    attempt_forward_empty_body_with_headers(
        upstream,
        path_and_query,
        method,
        x_forwarded_for,
        x_forwarded_proto,
        &[],
        _metrics,
        budget,
    )
    .await
}

/// NG-05: GET/HEAD forward with optional outbound header replacements.
#[allow(clippy::too_many_arguments)]
pub async fn attempt_forward_empty_body_with_headers(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    method: Method,
    x_forwarded_for: Option<&HeaderValue>,
    x_forwarded_proto: Option<&HeaderValue>,
    extra_headers: &[(String, String)],
    _metrics: &ProxyHyperMetrics,
    budget: Duration,
) -> Result<Response<BoxBody>, UpstreamAttemptClass> {
    if budget.is_zero() {
        return Err(UpstreamAttemptClass::TimedOut);
    }
    let is_head = method == Method::HEAD;

    // Assemble + strip hop-by-hop (incl. Connection nominations) before Request::builder.
    // Hyper may drop Connection during build; stripping afterward would miss nominations.
    let mut header_map = hyper::HeaderMap::new();
    for (name, value) in extra_headers {
        if name.eq_ignore_ascii_case("x-forwarded-for")
            || name.eq_ignore_ascii_case("x-forwarded-proto")
            || name.eq_ignore_ascii_case("host")
        {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            hyper::header::HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            header_map.insert(n, v);
        }
    }
    strip_hop_by_hop_headers(&mut header_map);

    let mut builder = Request::builder()
        .method(method)
        .uri(upstream.uri_for(path_and_query));
    for (name, value) in header_map.iter() {
        builder = builder.header(name, value);
    }
    // Identity headers after strip so Connection cannot wipe them.
    if let Some(value) = x_forwarded_for {
        builder = builder.header("x-forwarded-for", value.clone());
    }
    if let Some(value) = x_forwarded_proto {
        builder = builder.header("x-forwarded-proto", value.clone());
    }
    // Explicit Host from proxy_set_headers wins over upstream default.
    if let Some((_, host_v)) = extra_headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("host"))
    {
        builder = builder.header(HOST, host_v.as_str());
    } else if let Some(host) = &upstream.host {
        builder = builder.header(HOST, host.clone());
    }

    let req = builder
        .body(Empty::<bytes::Bytes>::new())
        .expect("valid upstream GET");

    let client = get_empty_body_client();
    let request = client.request(req);
    // Default ON: Cap021 budget via tokio::time::timeout (TimerEntry per attempt).
    // Opt-out A/B: EXYONQ_PROXY_SKIP_ATTEMPT_TIMEOUT=1 — rely on connector
    // connect_timeout only (residual profile: TimerEntry drop/reregister ~1.3%).
    let result = if skip_attempt_timeout_enabled() {
        request.await
    } else {
        match tokio::time::timeout(budget, request).await {
            Ok(inner) => inner,
            Err(_) => {
                warn!(budget_ms = budget.as_millis(), "upstream attempt timeout");
                // NG-08: attempt elapsed against this peer.
                upstream.passive.record(false);
                return Err(UpstreamAttemptClass::TimedOut);
            }
        }
    };
    match result {
        Ok(resp) => {
            // NG-08: upstream responded — passive health success.
            upstream.passive.record(true);
            let (mut parts, body) = resp.into_parts();
            strip_hop_by_hop_headers(&mut parts.headers);

            if is_head {
                return Ok(Response::from_parts(
                    parts,
                    Empty::<bytes::Bytes>::new()
                        .map_err(|never| match never {})
                        .boxed(),
                ));
            }

            // Cap030: never full-collect merely because Content-Length is declared.
            // Pass the upstream body through (SSE / large / small CL / unknown).
            let _ = path_and_query;
            Ok(Response::from_parts(parts, body.boxed()))
        }
        Err(err) => {
            let class = classify_hyper_error(&err);
            warn!(%err, ?class, "upstream error");
            // NG-08: transport failure against this peer.
            upstream.passive.record(false);
            Err(class)
        }
    }
}

fn skip_attempt_timeout_enabled() -> bool {
    use std::sync::atomic::{AtomicI8, Ordering};
    static CACHE: AtomicI8 = AtomicI8::new(-1);
    let cached = CACHE.load(Ordering::Relaxed);
    if cached >= 0 {
        return cached != 0;
    }
    let on = matches!(
        std::env::var("EXYONQ_PROXY_SKIP_ATTEMPT_TIMEOUT")
            .ok()
            .as_deref(),
        Some("1") | Some("true") | Some("on")
    );
    CACHE.store(if on { 1 } else { 0 }, Ordering::Relaxed);
    on
}

/// Headers never copied from the client on the cache GET path: connection
/// management, the ones we set ourselves, and body framing.
const GET_FORWARD_SKIP_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "te",
    "trailer",
    "upgrade",
    "proxy-connection",
    "proxy-authorization",
    "x-forwarded-for",
    "x-forwarded-proto",
];

pub async fn forward_get_streaming(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
    request_headers: &[(String, String)],
) -> Response<BoxBody> {
    forward_get_streaming_with_proto(
        upstream,
        path_and_query,
        x_forwarded_for,
        None,
        metrics,
        request_headers,
    )
    .await
}

pub async fn forward_get_streaming_with_proto(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    x_forwarded_proto: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
    request_headers: &[(String, String)],
) -> Response<BoxBody> {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(upstream.uri_for(path_and_query));

    if let Some(value) = x_forwarded_for {
        builder = builder.header("x-forwarded-for", value.clone());
    }
    if let Some(value) = x_forwarded_proto {
        builder = builder.header("x-forwarded-proto", value.clone());
    }
    if let Some(host) = &upstream.host {
        builder = builder.header(HOST, host.clone());
    }
    // The cache path must forward the client request faithfully: keying a
    // response on a header the origin never saw would cache one variant for all.
    for (name, value) in request_headers {
        let lower = name.to_ascii_lowercase();
        if GET_FORWARD_SKIP_HEADERS.contains(&lower.as_str()) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            hyper::header::HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            builder = builder.header(n, v);
        }
    }

    let req = builder
        .body(Empty::<bytes::Bytes>::new())
        .expect("valid upstream GET");

    let client = get_empty_body_client();
    let request = client.request(req);
    let timeout = upstream_timeout_for_path(upstream.timeout, path_and_query);
    match tokio::time::timeout(timeout, request).await {
        Ok(Ok(resp)) => {
            upstream.passive.record(true);
            let (mut parts, body) = resp.into_parts();
            strip_hop_by_hop_headers(&mut parts.headers);

            if response_is_event_stream(&parts) {
                metrics.sse_streams.fetch_add(1, Ordering::Relaxed);
            }
            // Cap030: always pass body through (no CL-based full collect).
            Response::from_parts(parts, body.boxed())
        }
        Ok(Err(err)) => {
            warn!(%err, "upstream error");
            upstream.passive.record(false);
            note_502(metrics);
            bad_gateway()
        }
        Err(_) => {
            warn!(
                timeout_ms = upstream.timeout.as_millis(),
                "upstream timeout"
            );
            upstream.passive.record(false);
            note_504(metrics);
            gateway_timeout()
        }
    }
}

pub async fn forward_request(
    client: &ProxyClient,
    upstream: &UpstreamTarget,
    mut req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    if crate::websocket::is_websocket_upgrade(req.headers()) {
        return crate::websocket::forward_websocket(
            client,
            upstream,
            req,
            x_forwarded_for,
            metrics,
        )
        .await;
    }

    if !request_headers_safe_for_proxy(req.headers()) {
        return bad_request("ambiguous request headers");
    }

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();

    *req.uri_mut() = upstream.uri_for(&path_and_query);

    // Strip client hop / Connection-nominated fields before injecting identity headers.
    strip_hop_by_hop_headers(req.headers_mut());

    if let Some(value) = x_forwarded_for {
        req.headers_mut().insert("x-forwarded-for", value.clone());
    }

    if let Some(host) = &upstream.host {
        req.headers_mut().insert(HOST, host.clone());
    }

    let request = client.request(req);
    let timeout = upstream_timeout_for_path(upstream.timeout, &path_and_query);
    match tokio::time::timeout(timeout, request).await {
        Ok(Ok(resp)) => {
            upstream.passive.record(true);
            let (mut parts, body) = resp.into_parts();
            strip_hop_by_hop_headers(&mut parts.headers);

            // Cap030: never full-collect merely because Content-Length is declared.
            Response::from_parts(parts, body.boxed())
        }
        Ok(Err(err)) => {
            warn!(%err, "upstream error");
            upstream.passive.record(false);
            note_502(metrics);
            bad_gateway()
        }
        Err(_) => {
            warn!(
                timeout_ms = upstream.timeout.as_millis(),
                "upstream timeout"
            );
            upstream.passive.record(false);
            note_504(metrics);
            gateway_timeout()
        }
    }
}

/// Spike forward: do not commit the upstream status until its body is complete.
///
/// A declared Content-Length that ends early, or a body read error, is 502.
/// The streaming product path stays in [`forward_request`].
pub async fn forward_request_complete(
    client: &ProxyClient,
    upstream: &UpstreamTarget,
    mut req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    if !request_headers_safe_for_proxy(req.headers()) {
        return bad_request("ambiguous request headers");
    }
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();
    *req.uri_mut() = upstream.uri_for(&path_and_query);
    strip_hop_by_hop_headers(req.headers_mut());
    if let Some(value) = x_forwarded_for {
        req.headers_mut().insert("x-forwarded-for", value.clone());
    }
    if let Some(host) = &upstream.host {
        req.headers_mut().insert(HOST, host.clone());
    }

    let request = client.request(req);
    let timeout = upstream_timeout_for_path(upstream.timeout, &path_and_query);
    let resp = match tokio::time::timeout(timeout, request).await {
        Ok(Ok(resp)) => {
            upstream.passive.record(true);
            resp
        }
        Ok(Err(err)) => {
            warn!(%err, "upstream error");
            upstream.passive.record(false);
            note_502(metrics);
            return bad_gateway();
        }
        Err(_) => {
            warn!(
                timeout_ms = upstream.timeout.as_millis(),
                "upstream timeout"
            );
            upstream.passive.record(false);
            note_504(metrics);
            return gateway_timeout();
        }
    };
    let (mut parts, body) = resp.into_parts();
    strip_hop_by_hop_headers(&mut parts.headers);
    let declared = parts
        .headers
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    match body.collect().await {
        Ok(collected) => {
            let bytes = collected.to_bytes();
            if let Some(expected) = declared {
                if bytes.len() as u64 != expected {
                    warn!(expected, got = bytes.len(), "upstream body collect error");
                    // NG-08: truncated body mid-exchange is a transport failure
                    // (net outcome: head success + body failure = one fail).
                    upstream.passive.record(false);
                    note_502(metrics);
                    return bad_gateway();
                }
            }
            let len = bytes.len();
            parts.headers.insert(
                hyper::header::CONTENT_LENGTH,
                HeaderValue::from_str(&len.to_string()).expect("length header"),
            );
            Response::from_parts(
                parts,
                Full::new(bytes).map_err(|never| match never {}).boxed(),
            )
        }
        Err(err) => {
            warn!(%err, "upstream body collect error");
            upstream.passive.record(false);
            note_502(metrics);
            bad_gateway()
        }
    }
}

/// Classify a completed hyper response into contract outcome (single upstream call).
///
/// Cap030 / SEC-PROXY-002: response bodies always stream — never full-collect merely
/// because Content-Length is declared (including ≤16 KiB). HEAD materializes empty.
pub async fn classify_hyper_response(
    response: Response<BoxBody>,
    path_and_query: &str,
    method: ProxyMethod,
) -> ProxyDispatchOutcome {
    let _ = path_and_query;
    let (parts, body) = response.into_parts();
    if parts.status.as_u16() == 502 {
        return ProxyDispatchOutcome::BadGateway;
    }
    if parts.status.as_u16() == 504 {
        return ProxyDispatchOutcome::GatewayTimeout;
    }

    if method == ProxyMethod::Head {
        return ProxyDispatchOutcome::Materialized(materialize_parts(parts, bytes::Bytes::new()));
    }

    let status = parts.status.as_u16();
    let headers = headers_to_pairs(&parts.headers);
    let handle = store_streaming_response(Response::from_parts(parts, body.boxed()));
    ProxyDispatchOutcome::Streaming {
        status,
        headers,
        stream: handle,
    }
}

pub async fn classify_websocket_response(response: Response<BoxBody>) -> ProxyDispatchOutcome {
    let (parts, body) = response.into_parts();
    if parts.status.as_u16() == 101 {
        let handle = store_websocket_response(Response::from_parts(parts, body.boxed()));
        return ProxyDispatchOutcome::Upgraded(handle);
    }
    if parts.status.as_u16() == 502 {
        return ProxyDispatchOutcome::BadGateway;
    }
    match body.collect().await {
        Ok(collected) => {
            ProxyDispatchOutcome::Materialized(materialize_parts(parts, collected.to_bytes()))
        }
        Err(_) => ProxyDispatchOutcome::BadGateway,
    }
}

pub fn take_streaming(handle: ProxyStreamHandle) -> Option<Response<BoxBody>> {
    crate::attach::take_streaming_response(handle)
}

pub fn take_websocket(handle: ProxyWebSocketHandle) -> Option<Response<BoxBody>> {
    crate::attach::take_websocket_response(handle)
}
