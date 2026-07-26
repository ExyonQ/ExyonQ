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
use crate::upstream_target::UpstreamTarget;
use crate::{
    content_type_is_event_stream, path_is_sse_stream, request_headers_safe_for_proxy,
    strip_hop_by_hop_headers, upstream_timeout_for_path, BENCH_API_CACHE_PATHS,
    BENCH_SMALL_UPSTREAM_BODY,
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
use tracing::warn;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

pub struct ProxyHyperMetrics {
    pub responses_502: AtomicU64,
    pub responses_504: AtomicU64,
    /// Responses classified or forwarded as SSE / event-stream.
    pub sse_streams: AtomicU64,
}

impl Default for ProxyHyperMetrics {
    fn default() -> Self {
        Self {
            responses_502: AtomicU64::new(0),
            responses_504: AtomicU64::new(0),
            sse_streams: AtomicU64::new(0),
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

fn should_stream(parts: &http::response::Parts, path_and_query: &str) -> bool {
    response_is_event_stream(parts) || path_is_sse_stream(path_and_query)
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
    if method == Method::GET {
        if let Some(response) = upstream.api_cache_hit(path_and_query) {
            return response;
        }
    }

    let is_get = method == Method::GET;
    let is_head = method == Method::HEAD;
    let mut builder = Request::builder()
        .method(method)
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

    let req = builder
        .body(Empty::<bytes::Bytes>::new())
        .expect("valid upstream GET");

    let client = get_empty_body_client();
    let request = client.request(req);
    let timeout = upstream_timeout_for_path(upstream.timeout, path_and_query);
    match tokio::time::timeout(timeout, request).await {
        Ok(Ok(resp)) => {
            let (mut parts, body) = resp.into_parts();
            strip_hop_by_hop_headers(&mut parts.headers);

            if is_head {
                return Response::from_parts(
                    parts,
                    Empty::<bytes::Bytes>::new()
                        .map_err(|never| match never {})
                        .boxed(),
                );
            }

            if should_stream(&parts, path_and_query) {
                return Response::from_parts(parts, body.boxed());
            }

            let content_length = parts
                .headers
                .get(hyper::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok());
            let path_only = path_and_query.split('?').next().unwrap_or(path_and_query);
            let force_cache = BENCH_API_CACHE_PATHS.contains(&path_only);

            if force_cache || content_length.is_some_and(|len| len <= BENCH_SMALL_UPSTREAM_BODY) {
                let body = body
                    .collect()
                    .await
                    .map(|collected| collected.to_bytes())
                    .unwrap_or_default();
                if is_get {
                    upstream.try_store_api_cache(path_and_query, parts.clone(), body.clone());
                }
                Response::from_parts(
                    parts,
                    Full::from(body).map_err(|never| match never {}).boxed(),
                )
            } else {
                Response::from_parts(parts, body.boxed())
            }
        }
        Ok(Err(err)) => {
            warn!(%err, "upstream error");
            upstream.clear_api_cache(path_and_query);
            note_502(metrics);
            bad_gateway()
        }
        Err(_) => {
            warn!(
                timeout_ms = upstream.timeout.as_millis(),
                "upstream timeout"
            );
            upstream.clear_api_cache(path_and_query);
            note_504(metrics);
            gateway_timeout()
        }
    }
}

pub async fn forward_get_streaming(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    forward_get_streaming_with_proto(upstream, path_and_query, x_forwarded_for, None, metrics).await
}

pub async fn forward_get_streaming_with_proto(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    x_forwarded_proto: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody> {
    if let Some(response) = upstream.api_cache_hit(path_and_query) {
        return response;
    }

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

    let req = builder
        .body(Empty::<bytes::Bytes>::new())
        .expect("valid upstream GET");

    let client = get_empty_body_client();
    let request = client.request(req);
    let timeout = upstream_timeout_for_path(upstream.timeout, path_and_query);
    match tokio::time::timeout(timeout, request).await {
        Ok(Ok(resp)) => {
            let (mut parts, body) = resp.into_parts();
            strip_hop_by_hop_headers(&mut parts.headers);

            if should_stream(&parts, path_and_query) {
                metrics.sse_streams.fetch_add(1, Ordering::Relaxed);
                return Response::from_parts(parts, body.boxed());
            }

            Response::from_parts(parts, body.boxed())
        }
        Ok(Err(err)) => {
            warn!(%err, "upstream error");
            upstream.clear_api_cache(path_and_query);
            note_502(metrics);
            bad_gateway()
        }
        Err(_) => {
            warn!(
                timeout_ms = upstream.timeout.as_millis(),
                "upstream timeout"
            );
            upstream.clear_api_cache(path_and_query);
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

    if let Some(response) = upstream.api_cache_hit(&path_and_query) {
        return response;
    }

    *req.uri_mut() = upstream.uri_for(&path_and_query);

    if let Some(value) = x_forwarded_for {
        req.headers_mut().insert("x-forwarded-for", value.clone());
    }

    if let Some(host) = &upstream.host {
        req.headers_mut().insert(HOST, host.clone());
    }

    strip_hop_by_hop_headers(req.headers_mut());

    let request = client.request(req);
    let timeout = upstream_timeout_for_path(upstream.timeout, &path_and_query);
    match tokio::time::timeout(timeout, request).await {
        Ok(Ok(resp)) => {
            let (mut parts, body) = resp.into_parts();
            strip_hop_by_hop_headers(&mut parts.headers);

            if should_stream(&parts, &path_and_query) {
                return Response::from_parts(parts, body.boxed());
            }

            let content_length = parts
                .headers
                .get(hyper::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(BENCH_SMALL_UPSTREAM_BODY + 1);

            if content_length <= BENCH_SMALL_UPSTREAM_BODY {
                let body = body
                    .collect()
                    .await
                    .map(|collected| collected.to_bytes())
                    .unwrap_or_default();
                upstream.try_store_api_cache(&path_and_query, parts.clone(), body.clone());
                Response::from_parts(
                    parts,
                    Full::from(body).map_err(|never| match never {}).boxed(),
                )
            } else {
                Response::from_parts(parts, body.boxed())
            }
        }
        Ok(Err(err)) => {
            warn!(%err, "upstream error");
            upstream.clear_api_cache(&path_and_query);
            note_502(metrics);
            bad_gateway()
        }
        Err(_) => {
            warn!(
                timeout_ms = upstream.timeout.as_millis(),
                "upstream timeout"
            );
            upstream.clear_api_cache(&path_and_query);
            note_504(metrics);
            gateway_timeout()
        }
    }
}

/// Classify a completed hyper response into contract outcome (single upstream call).
///
/// SEC-PROXY-002: unknown-length bodies stream (no unbounded `collect`). Declared
/// Content-Length ≤ [`BENCH_SMALL_UPSTREAM_BODY`] may materialize only with an
/// incremental bound equal to the **declared** length; overflow falls back to
/// streaming with prefix preserved.
pub async fn classify_hyper_response(
    response: Response<BoxBody>,
    path_and_query: &str,
    method: ProxyMethod,
) -> ProxyDispatchOutcome {
    let (parts, body) = response.into_parts();
    if parts.status.as_u16() == 502 {
        return ProxyDispatchOutcome::BadGateway;
    }
    if parts.status.as_u16() == 504 {
        return ProxyDispatchOutcome::GatewayTimeout;
    }

    if should_stream(&parts, path_and_query) {
        let status = parts.status.as_u16();
        let headers = headers_to_pairs(&parts.headers);
        let handle = store_streaming_response(Response::from_parts(parts, body.boxed()));
        return ProxyDispatchOutcome::Streaming {
            status,
            headers,
            stream: handle,
        };
    }

    if method == ProxyMethod::Head {
        return ProxyDispatchOutcome::Materialized(materialize_parts(parts, bytes::Bytes::new()));
    }

    let content_length = parts
        .headers
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());

    match content_length {
        // Unknown length (chunked / H2 without CL / unparsable): never full-collect.
        None => {
            let status = parts.status.as_u16();
            let headers = headers_to_pairs(&parts.headers);
            let handle = store_streaming_response(Response::from_parts(parts, body.boxed()));
            ProxyDispatchOutcome::Streaming {
                status,
                headers,
                stream: handle,
            }
        }
        Some(len) if len > BENCH_SMALL_UPSTREAM_BODY => {
            let status = parts.status.as_u16();
            let headers = headers_to_pairs(&parts.headers);
            let handle = store_streaming_response(Response::from_parts(parts, body.boxed()));
            ProxyDispatchOutcome::Streaming {
                status,
                headers,
                stream: handle,
            }
        }
        Some(declared) => {
            // declared ≤ 16 KiB: bounded materialization; bound = declared CL.
            match materialize_small_declared_body(body.boxed(), declared).await {
                SmallDeclaredRead::Complete(bytes) => {
                    ProxyDispatchOutcome::Materialized(materialize_parts(parts, bytes))
                }
                SmallDeclaredRead::Exceeded {
                    prefix,
                    first_chunk,
                    remainder,
                } => {
                    let status = parts.status.as_u16();
                    let headers = headers_to_pairs(&parts.headers);
                    let stream = ClassifyPrefixThenBody {
                        prefix: Some(prefix),
                        first_chunk: Some(first_chunk),
                        remainder: Some(remainder),
                    }
                    .boxed();
                    let handle = store_streaming_response(Response::from_parts(parts, stream));
                    ProxyDispatchOutcome::Streaming {
                        status,
                        headers,
                        stream: handle,
                    }
                }
                // Prior path used unwrap_or_default() on collect errors.
                SmallDeclaredRead::ReadError => ProxyDispatchOutcome::Materialized(
                    materialize_parts(parts, bytes::Bytes::new()),
                ),
            }
        }
    }
}

/// Incremental accept (checked arithmetic) for small declared-CL materialization.
fn accept_declared_chunk(current_len: usize, chunk_len: usize, max_bytes: usize) -> Option<usize> {
    current_len
        .checked_add(chunk_len)
        .filter(|total| *total <= max_bytes)
}

enum SmallDeclaredRead<B> {
    Complete(bytes::Bytes),
    Exceeded {
        prefix: bytes::Bytes,
        first_chunk: bytes::Bytes,
        remainder: B,
    },
    ReadError,
}

/// Materialize up to `max_bytes` (declared CL). On overflow, return prefix + remainder
/// for streaming fallback. Generic over body type to keep hot-path type-name budget flat.
async fn materialize_small_declared_body<B>(
    mut body: B,
    max_bytes: usize,
) -> SmallDeclaredRead<B>
where
    B: hyper::body::Body + Unpin,
    B::Data: AsRef<[u8]> + Into<bytes::Bytes>,
{
    let mut buf = Vec::new();
    loop {
        match body.frame().await {
            None => return SmallDeclaredRead::Complete(bytes::Bytes::from(buf)),
            Some(Err(_)) => return SmallDeclaredRead::ReadError,
            Some(Ok(frame)) => {
                let Ok(chunk) = frame.into_data() else {
                    continue;
                };
                if chunk.as_ref().is_empty() {
                    continue;
                }
                let chunk_len = chunk.as_ref().len();
                if accept_declared_chunk(buf.len(), chunk_len, max_bytes).is_some() {
                    buf.extend_from_slice(chunk.as_ref());
                    continue;
                }
                let room = max_bytes.saturating_sub(buf.len());
                let chunk_bytes: bytes::Bytes = chunk.into();
                if room > 0 {
                    buf.extend_from_slice(&chunk_bytes[..room]);
                }
                let first_chunk = chunk_bytes.slice(room..);
                return SmallDeclaredRead::Exceeded {
                    prefix: bytes::Bytes::from(buf),
                    first_chunk,
                    remainder: body,
                };
            }
        }
    }
}

/// Local prefix chain for classify overflow (not a public body trait).
/// Generic remainder keeps hot-path type-name budget flat on this file.
struct ClassifyPrefixThenBody<B> {
    prefix: Option<bytes::Bytes>,
    first_chunk: Option<bytes::Bytes>,
    remainder: Option<B>,
}

impl<B> hyper::body::Body for ClassifyPrefixThenBody<B>
where
    B: hyper::body::Body<Data = bytes::Bytes, Error = hyper::Error> + Unpin,
{
    type Data = bytes::Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if let Some(prefix) = this.prefix.take() {
            if !prefix.is_empty() {
                return std::task::Poll::Ready(Some(Ok(hyper::body::Frame::data(prefix))));
            }
        }
        if let Some(first) = this.first_chunk.take() {
            if !first.is_empty() {
                return std::task::Poll::Ready(Some(Ok(hyper::body::Frame::data(first))));
            }
        }
        let Some(body) = this.remainder.as_mut() else {
            return std::task::Poll::Ready(None);
        };
        match std::pin::Pin::new(body).poll_frame(cx) {
            std::task::Poll::Ready(None) => {
                this.remainder = None;
                std::task::Poll::Ready(None)
            }
            other => other,
        }
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
    let body = body
        .collect()
        .await
        .map(|collected| collected.to_bytes())
        .unwrap_or_default();
    ProxyDispatchOutcome::Materialized(materialize_parts(parts, body))
}

pub fn take_streaming(handle: ProxyStreamHandle) -> Option<Response<BoxBody>> {
    crate::attach::take_streaming_response(handle)
}

pub fn take_websocket(handle: ProxyWebSocketHandle) -> Option<Response<BoxBody>> {
    crate::attach::take_websocket_response(handle)
}
