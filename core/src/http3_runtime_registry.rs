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
//! Core adapter — HTTP/3 dispatch into generic handler pipeline (KD4.5).
//!
//! H3-SEC-001: response bodies are materialized with an incremental application
//! bound (`H3_MAX_MATERIALIZED_RESPONSE_BODY_BYTES`). Oversized bodies become a
//! stream-local HTTP 500 without releasing the connection lease.

use crate::lifecycle::{ConnectionLifecycleToken, LifecycleState};
use crate::reload::{read_state, SharedServerState};
use crate::server::handler::{self, ConnectionContext};
use async_trait::async_trait;
use bytes::Bytes;
use exyonq_mod_proxy::ProxyClient;
use exyonq_module_api::http3_runtime::{
    Http3ConnectionLifecycle, Http3DispatchError, Http3DispatchService, Http3DrainRejected,
    Http3MaterializedResponse,
};
use exyonq_module_api::static_dispatch::MATERIALIZATION_BUDGET_EXCEEDED_HEADER;
use http::{Request, Response};
use http_body_util::BodyExt;
use hyper::body::Body;
use hyper::header::HeaderValue;
use std::sync::Arc;
use tracing::warn;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

/// Temporary H3 response materialization ceiling.
///
/// Policy: `docs/architecture/security/H3_BODY_LIMIT_POLICY_DECISION.md`.
/// - Temporary private constant (not public config / RuntimePlan).
/// - Future migration: `http3.max_materialized_response_body_bytes`.
/// - Not a global cross-protocol response limit.
/// - Does **not** replace module-pipeline `MODULE_BODY_LIMIT` (4 MiB contract).
const H3_MAX_MATERIALIZED_RESPONSE_BODY_BYTES: usize = 67_108_864;

/// Single-source accessor for the temporary H3 materialization ceiling.
///
/// Returns the numeric budget only — not an H3 type. Used by the H3 entry path to
/// inject `StaticDispatchRequest.materialization_budget_bytes`. Not a public API.
pub(crate) fn h3_materialization_budget_bytes() -> u64 {
    H3_MAX_MATERIALIZED_RESPONSE_BODY_BYTES as u64
}

/// Private materialization error (not a public API).
#[derive(Debug)]
enum H3ResponseMaterializationError {
    Body(hyper::Error),
    TooLarge,
}

/// Whether appending `chunk_len` stays within `max_bytes` (checked arithmetic).
fn h3_response_body_accept_chunk(
    current_len: usize,
    chunk_len: usize,
    max_bytes: usize,
) -> Option<usize> {
    current_len
        .checked_add(chunk_len)
        .filter(|total| *total <= max_bytes)
}

/// Incremental H3 response body materialization with an authoritative byte limit.
///
/// On [`H3ResponseMaterializationError::TooLarge`]: does not append the crossing
/// frame, drops the partial buffer, and stops polling (body dropped on return).
async fn collect_h3_response_body_bounded<B>(
    mut body: B,
    max_bytes: usize,
) -> Result<Bytes, H3ResponseMaterializationError>
where
    B: Body<Data = Bytes, Error = hyper::Error> + Unpin,
{
    let mut buf = Vec::new();
    loop {
        match body.frame().await {
            None => return Ok(Bytes::from(buf)),
            Some(Err(err)) => return Err(H3ResponseMaterializationError::Body(err)),
            Some(Ok(frame)) => {
                let Ok(chunk) = frame.into_data() else {
                    // Trailers / non-data: preserve current ignore semantics.
                    continue;
                };
                if chunk.is_empty() {
                    continue;
                }
                if h3_response_body_accept_chunk(buf.len(), chunk.len(), max_bytes).is_none() {
                    drop(buf);
                    drop(body);
                    return Err(H3ResponseMaterializationError::TooLarge);
                }
                buf.extend_from_slice(&chunk);
            }
        }
    }
}

fn h3_oversized_materialized_response() -> Http3MaterializedResponse {
    Http3MaterializedResponse {
        status: 500,
        headers: vec![(
            "content-type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )],
        body: Bytes::from_static(b"internal server error"),
    }
}

/// RAII lease — one per QUIC connection (PS1A-H3). Not `Clone`.
pub struct Http3ConnectionLease(#[allow(dead_code)] ConnectionLifecycleToken);

/// Core implementation of kernel QUIC connection admission.
pub struct CoreHttp3Lifecycle {
    ops: Arc<LifecycleState>,
}

impl CoreHttp3Lifecycle {
    pub fn new(ops: Arc<LifecycleState>) -> Self {
        Self { ops }
    }
}

impl Http3ConnectionLifecycle for CoreHttp3Lifecycle {
    type Lease = Http3ConnectionLease;

    fn try_enter_connection(&self) -> Result<Http3ConnectionLease, Http3DrainRejected> {
        self.ops
            .try_enter()
            .map(Http3ConnectionLease)
            .map_err(|_| Http3DrainRejected)
    }
}

pub struct CoreHttp3Dispatcher {
    shared: SharedServerState,
    proxy_client: ProxyClient,
    lifecycle: Arc<LifecycleState>,
}

impl CoreHttp3Dispatcher {
    pub fn new(
        shared: SharedServerState,
        proxy_client: ProxyClient,
        lifecycle: Arc<LifecycleState>,
    ) -> Self {
        Self {
            shared,
            proxy_client,
            lifecycle,
        }
    }
}

#[async_trait]
impl Http3DispatchService for CoreHttp3Dispatcher {
    async fn dispatch(
        &self,
        req: Request<()>,
        peer_ip: &str,
    ) -> Result<Http3MaterializedResponse, Http3DispatchError> {
        let state = read_state(&self.shared);
        let x_forwarded_for =
            HeaderValue::from_str(peer_ip).unwrap_or_else(|_| HeaderValue::from_static("0.0.0.0"));
        let response = handler::serve_http3_request(
            ConnectionContext {
                state,
                proxy_client: self.proxy_client.clone(),
                x_forwarded_for,
                ops: Arc::clone(&self.lifecycle),
            },
            req,
        )
        .await;
        materialize_response(response)
            .await
            .map_err(|err| -> Http3DispatchError {
                Box::new(std::io::Error::other(err.to_string()))
            })
    }
}

async fn materialize_response(
    response: Response<BoxBody>,
) -> anyhow::Result<Http3MaterializedResponse> {
    materialize_response_with_limit(response, H3_MAX_MATERIALIZED_RESPONSE_BODY_BYTES).await
}

async fn materialize_response_with_limit(
    response: Response<BoxBody>,
    max_bytes: usize,
) -> anyhow::Result<Http3MaterializedResponse> {
    // Static (or other owners) signalled budget exceeded before building a Full body.
    // Map to the canonical H3 oversized outcome (500 / stream-local / KEEP).
    if response
        .headers()
        .get(MATERIALIZATION_BUDGET_EXCEEDED_HEADER)
        .is_some_and(|v| v.as_bytes() == b"1")
    {
        drop(response);
        return Ok(h3_oversized_materialized_response());
    }

    let (parts, body) = response.into_parts();

    // Defense for already-materialized bodies (e.g. cache hit) with trustworthy CL.
    if let Some(cl) = parts
        .headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
    {
        if cl > max_bytes {
            drop(body);
            warn!(
                limit_bytes = max_bytes,
                content_length = cl,
                reason = "h3_response_content_length_precheck",
                "h3 response Content-Length exceeds materialization limit"
            );
            return Ok(h3_oversized_materialized_response());
        }
    }

    match collect_h3_response_body_bounded(body, max_bytes).await {
        Ok(collected) => {
            let headers = parts
                .headers
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|v| (name.as_str().to_string(), v.to_string()))
                })
                .collect();
            Ok(Http3MaterializedResponse {
                status: parts.status.as_u16(),
                headers,
                body: collected,
            })
        }
        Err(H3ResponseMaterializationError::TooLarge) => {
            // METRIC_ADDED = NO; METRIC_GAP = YES (no compatible counter infrastructure here).
            warn!(
                limit_bytes = max_bytes,
                reason = "h3_response_body_limit",
                "h3 response body exceeds materialization limit"
            );
            // Stream-local HTTP 500 — Ok so the H3 accept loop keeps the connection.
            Ok(h3_oversized_materialized_response())
        }
        Err(H3ResponseMaterializationError::Body(err)) => Err(err.into()),
    }
}

pub fn spawn_http3_listener(
    settings: exyonq_mod_http3::Http3Settings,
    shared: SharedServerState,
    proxy_client: ProxyClient,
    lifecycle: Arc<LifecycleState>,
) {
    let dispatch = Arc::new(CoreHttp3Dispatcher::new(
        SharedServerState::clone(&shared),
        proxy_client.clone(),
        Arc::clone(&lifecycle),
    ));
    let admission = Arc::new(CoreHttp3Lifecycle::new(Arc::clone(&lifecycle)));
    tokio::spawn(async move {
        if let Err(err) = exyonq_mod_http3::serve(settings, dispatch, admission).await {
            tracing::warn!(%err, "http3 listener stopped");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::{BodyExt, Full};
    use hyper::StatusCode;
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::task::{Context, Poll};

    fn full_response(status: StatusCode, body: Bytes) -> Response<BoxBody> {
        Response::builder()
            .status(status)
            .body(Full::from(body).map_err(|never| match never {}).boxed())
            .expect("response")
    }

    struct Probe {
        frames: AtomicUsize,
        polls: AtomicUsize,
        dropped: AtomicBool,
    }

    struct InstrumentedBody {
        probe: Arc<Probe>,
        pending: VecDeque<Bytes>,
    }

    impl Drop for InstrumentedBody {
        fn drop(&mut self) {
            self.probe.dropped.store(true, Ordering::SeqCst);
        }
    }

    impl Body for InstrumentedBody {
        type Data = Bytes;
        type Error = hyper::Error;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
            self.probe.polls.fetch_add(1, Ordering::SeqCst);
            match self.pending.pop_front() {
                None => Poll::Ready(None),
                Some(chunk) => {
                    self.probe.frames.fetch_add(1, Ordering::SeqCst);
                    Poll::Ready(Some(Ok(hyper::body::Frame::data(chunk))))
                }
            }
        }
    }

    fn instrumented(probe: Arc<Probe>, chunks: Vec<Bytes>) -> BoxBody {
        InstrumentedBody {
            probe,
            pending: chunks.into(),
        }
        .boxed()
    }

    #[test]
    fn accept_chunk_boundary_and_overflow() {
        assert_eq!(h3_response_body_accept_chunk(0, 64, 64), Some(64));
        assert_eq!(h3_response_body_accept_chunk(64, 1, 64), None);
        assert_eq!(
            h3_response_body_accept_chunk(usize::MAX, 1, usize::MAX),
            None
        );
        assert_eq!(h3_response_body_accept_chunk(10, 5, 16), Some(15));
    }

    #[test]
    fn productive_limit_is_64_mib() {
        assert_eq!(H3_MAX_MATERIALIZED_RESPONSE_BODY_BYTES, 67_108_864);
        assert_eq!(h3_materialization_budget_bytes(), 67_108_864);
    }

    #[tokio::test]
    async fn static_budget_exceeded_header_maps_to_canonical_500() {
        let response = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(MATERIALIZATION_BUDGET_EXCEEDED_HEADER, "1")
            .body(
                Full::from(Bytes::from_static(b"should-not-appear"))
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .unwrap();
        let out = materialize_response_with_limit(response, 64)
            .await
            .expect("ok");
        assert_eq!(out.status, 500);
        assert_eq!(out.body.as_ref(), b"internal server error");
    }

    #[tokio::test]
    async fn content_length_precheck_rejects_without_collecting_body() {
        let probe = Arc::new(Probe {
            frames: AtomicUsize::new(0),
            polls: AtomicUsize::new(0),
            dropped: AtomicBool::new(false),
        });
        let response = Response::builder()
            .status(200)
            .header(http::header::CONTENT_LENGTH, "1000")
            .body(instrumented(
                Arc::clone(&probe),
                vec![Bytes::from(vec![b'z'; 64])],
            ))
            .unwrap();
        let out = materialize_response_with_limit(response, 64)
            .await
            .expect("ok");
        assert_eq!(out.status, 500);
        assert_eq!(out.body.as_ref(), b"internal server error");
        assert_eq!(probe.frames.load(Ordering::SeqCst), 0);
        assert!(probe.dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn collect_limit_minus_one_ok() {
        let max = 64usize;
        let body = full_response(StatusCode::OK, Bytes::from(vec![b'a'; max - 1]));
        let out = materialize_response_with_limit(body, max)
            .await
            .expect("ok");
        assert_eq!(out.status, 200);
        assert_eq!(out.body.len(), max - 1);
    }

    #[tokio::test]
    async fn collect_exact_limit_ok() {
        let max = 64usize;
        let body = full_response(StatusCode::OK, Bytes::from(vec![b'b'; max]));
        let out = materialize_response_with_limit(body, max)
            .await
            .expect("ok");
        assert_eq!(out.status, 200);
        assert_eq!(out.body.len(), max);
    }

    #[tokio::test]
    async fn collect_limit_plus_one_returns_500_without_original_body() {
        let max = 64usize;
        let body = full_response(StatusCode::OK, Bytes::from(vec![b'c'; max + 1]));
        let out = materialize_response_with_limit(body, max)
            .await
            .expect("stream-local ok");
        assert_eq!(out.status, 500);
        assert_eq!(out.body.as_ref(), b"internal server error");
        assert!(out.body.len() < max);
    }

    #[tokio::test]
    async fn empty_body_ok() {
        let out = materialize_response_with_limit(full_response(StatusCode::OK, Bytes::new()), 64)
            .await
            .expect("ok");
        assert_eq!(out.status, 200);
        assert!(out.body.is_empty());
    }

    #[tokio::test]
    async fn many_small_frames_sum_and_crossing_aborts_before_append() {
        let max = 32usize;
        let probe = Arc::new(Probe {
            frames: AtomicUsize::new(0),
            polls: AtomicUsize::new(0),
            dropped: AtomicBool::new(false),
        });
        // 4×8 = 32 OK if exact; add one more byte frame to cross.
        let chunks = vec![
            Bytes::from(vec![1u8; 8]),
            Bytes::from(vec![2u8; 8]),
            Bytes::from(vec![3u8; 8]),
            Bytes::from(vec![4u8; 8]),
            Bytes::from_static(b"x"),   // crosses
            Bytes::from(vec![9u8; 64]), // must not be fully drained
        ];
        let total_chunks = chunks.len();
        let response = Response::builder()
            .status(200)
            .body(instrumented(Arc::clone(&probe), chunks))
            .unwrap();
        let out = materialize_response_with_limit(response, max)
            .await
            .expect("ok path");
        assert_eq!(out.status, 500);
        assert!(probe.dropped.load(Ordering::SeqCst));
        let frames = probe.frames.load(Ordering::SeqCst);
        // Filled 4 frames (=32), 5th crosses → abort; must not drain remaining.
        assert!(frames < total_chunks);
        assert!(frames <= 5);
    }

    #[tokio::test]
    async fn single_frame_larger_than_limit_rejects_immediately() {
        let max = 16usize;
        let probe = Arc::new(Probe {
            frames: AtomicUsize::new(0),
            polls: AtomicUsize::new(0),
            dropped: AtomicBool::new(false),
        });
        let response = Response::builder()
            .status(200)
            .body(instrumented(
                Arc::clone(&probe),
                vec![Bytes::from(vec![b'z'; 64]), Bytes::from(vec![b'y'; 64])],
            ))
            .unwrap();
        let out = materialize_response_with_limit(response, max)
            .await
            .expect("ok");
        assert_eq!(out.status, 500);
        assert_eq!(probe.frames.load(Ordering::SeqCst), 1);
        assert!(probe.dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn trailers_ignored_like_before() {
        // Full body has no trailers; empty + data frames only path covered.
        // Non-data frames: Body that yields a trailers frame then data.
        use hyper::body::Frame;
        struct TrailersThenData {
            phase: u8,
        }
        impl Body for TrailersThenData {
            type Data = Bytes;
            type Error = hyper::Error;
            fn poll_frame(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
                match self.phase {
                    0 => {
                        self.phase = 1;
                        Poll::Ready(Some(Ok(Frame::trailers(http::HeaderMap::new()))))
                    }
                    1 => {
                        self.phase = 2;
                        Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"ok")))))
                    }
                    _ => Poll::Ready(None),
                }
            }
        }
        let response = Response::builder()
            .status(200)
            .body(TrailersThenData { phase: 0 }.boxed())
            .unwrap();
        let out = materialize_response_with_limit(response, 64)
            .await
            .expect("ok");
        assert_eq!(out.status, 200);
        assert_eq!(out.body.as_ref(), b"ok");
    }

    #[tokio::test]
    async fn sequential_oversized_then_valid_keeps_working() {
        let max = 32usize;
        let over = materialize_response_with_limit(
            full_response(StatusCode::OK, Bytes::from(vec![b'x'; max + 8])),
            max,
        )
        .await
        .expect("ok");
        assert_eq!(over.status, 500);
        let ok = materialize_response_with_limit(
            full_response(StatusCode::OK, Bytes::from_static(b"next")),
            max,
        )
        .await
        .expect("ok");
        assert_eq!(ok.status, 200);
        assert_eq!(ok.body.as_ref(), b"next");
    }

    #[tokio::test]
    async fn dispatch_oversized_health_unaffected_and_no_admission() {
        // /health is small; verify admission invariant still holds after materialize path.
        let ops = LifecycleState::new();
        let raw = include_str!("../../tests/fixtures/minimal.toml");
        let config: crate::config::AppConfig = raw.parse().expect("config");
        let proxy = exyonq_mod_proxy::build_incoming_client();
        let state = crate::server::state::ServerState::new(config, proxy.clone())
            .await
            .expect("state");
        let shared = crate::reload::wrap_state(state);
        let dispatcher = CoreHttp3Dispatcher::new(shared, proxy, Arc::clone(&ops));
        assert_eq!(ops.active_connections(), 0);
        let req = Request::get("/health").body(()).unwrap();
        let resp = dispatcher
            .dispatch(req, "127.0.0.1")
            .await
            .expect("dispatch");
        assert_eq!(resp.status, 200);
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn h3_lease_maps_try_enter_once() {
        let ops = LifecycleState::new();
        let lifecycle = CoreHttp3Lifecycle::new(Arc::clone(&ops));
        let lease = lifecycle.try_enter_connection().unwrap();
        assert_eq!(ops.active_connections(), 1);
        drop(lease);
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn h3_lease_rejects_when_draining_without_increment() {
        let ops = LifecycleState::new();
        ops.start_drain();
        let lifecycle = CoreHttp3Lifecycle::new(Arc::clone(&ops));
        assert!(lifecycle.try_enter_connection().is_err());
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn h3_lease_move_does_not_double_decrement() {
        let ops = LifecycleState::new();
        let lifecycle = CoreHttp3Lifecycle::new(Arc::clone(&ops));
        let lease = lifecycle.try_enter_connection().unwrap();
        let moved = lease;
        drop(moved);
        assert_eq!(ops.active_connections(), 0);
    }

    #[tokio::test]
    async fn dispatch_does_not_call_try_enter() {
        let ops = LifecycleState::new();
        let raw = include_str!("../../tests/fixtures/minimal.toml");
        let config: crate::config::AppConfig = raw.parse().expect("config");
        let proxy = exyonq_mod_proxy::build_incoming_client();
        let state = crate::server::state::ServerState::new(config, proxy.clone())
            .await
            .expect("state");
        let shared = crate::reload::wrap_state(state);
        let dispatcher = CoreHttp3Dispatcher::new(shared, proxy, Arc::clone(&ops));
        assert_eq!(ops.active_connections(), 0);
        let req = Request::get("/health").body(()).unwrap();
        let _ = dispatcher.dispatch(req, "127.0.0.1").await;
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn source_code_has_no_unbounded_collect_on_h3_materialize_path() {
        let src = include_str!("http3_runtime_registry.rs");
        assert!(src.contains("collect_h3_response_body_bounded"));
        assert!(src.contains("H3_MAX_MATERIALIZED_RESPONSE_BODY_BYTES"));
        // Production path must not call BodyExt::collect / .collect() on the response body.
        let prod = src
            .split("#[cfg(test)]")
            .next()
            .expect("production section");
        assert!(
            !prod.contains("BodyExt::collect"),
            "must not use BodyExt::collect"
        );
        assert!(
            !prod.contains("body.collect()"),
            "must not use body.collect()"
        );
    }
}
