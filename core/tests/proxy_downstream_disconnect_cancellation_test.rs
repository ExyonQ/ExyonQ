//! PROXY_DOWNSTREAM_DISCONNECT_CANCELLATION_TEST
//!
//! Proves (or refutes) that dropping the downstream streaming body after the first
//! frame stops further polling of the instrumented upstream body.
//!
//! Models: downstream disconnect → Hyper / consumer drops response body → upstream
//! `Body` must not continue to be drained unboundedly.
//!
//! Production code is not modified. Test-only instrumented body.

use bytes::Bytes;
use exyonq_mod_proxy::hyper_forward::classify_hyper_response;
use exyonq_mod_proxy::{take_streaming, BENCH_SMALL_UPSTREAM_BODY};
use exyonq_module_api::proxy_dispatch::{ProxyDispatchOutcome, ProxyMethod};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame};
use hyper::Response;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::Notify;

/// Shared counters for the instrumented upstream body.
#[derive(Debug)]
struct BodyProbe {
    polls: AtomicUsize,
    frames: AtomicUsize,
    bytes: AtomicUsize,
    dropped: AtomicBool,
    first_frame: Notify,
}

impl BodyProbe {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            polls: AtomicUsize::new(0),
            frames: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
            dropped: AtomicBool::new(false),
            first_frame: Notify::new(),
        })
    }
}

/// Upstream body that records polls/frames/bytes/drop and can supply many chunks.
struct InstrumentedUpstreamBody {
    probe: Arc<BodyProbe>,
    pending: VecDeque<Bytes>,
    notified_first: bool,
}

impl Drop for InstrumentedUpstreamBody {
    fn drop(&mut self) {
        self.probe.dropped.store(true, Ordering::SeqCst);
    }
}

impl Body for InstrumentedUpstreamBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = &mut *self;
        this.probe.polls.fetch_add(1, Ordering::SeqCst);
        match this.pending.pop_front() {
            None => Poll::Ready(None),
            Some(chunk) => {
                let len = chunk.len();
                this.probe.frames.fetch_add(1, Ordering::SeqCst);
                this.probe.bytes.fetch_add(len, Ordering::SeqCst);
                if !this.notified_first {
                    this.notified_first = true;
                    this.probe.first_frame.notify_waiters();
                }
                Poll::Ready(Some(Ok(Frame::data(chunk))))
            }
        }
    }
}

fn boxed_instrumented(
    probe: Arc<BodyProbe>,
    chunks: Vec<Bytes>,
) -> http_body_util::combinators::BoxBody<Bytes, hyper::Error> {
    InstrumentedUpstreamBody {
        probe,
        pending: chunks.into(),
        notified_first: false,
    }
    .boxed()
}

/// Race allowance: at most one extra frame already in flight when the consumer drops.
const RACE_ALLOWANCE_FRAMES: usize = 1;

/// After drop, wait for drop flag or short deadline; re-check counters for growth.
async fn assert_upstream_stops_after_drop(
    probe: &BodyProbe,
    frames_at_drop: usize,
    total_chunks: usize,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(200);
    while tokio::time::Instant::now() < deadline {
        if probe.dropped.load(Ordering::SeqCst) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        probe.dropped.load(Ordering::SeqCst),
        "upstream body must be dropped after downstream disconnect (drop of response body)"
    );

    // Settle window: ensure no detached drain keeps consuming.
    let polls_after_drop = probe.polls.load(Ordering::SeqCst);
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let polls_final = probe.polls.load(Ordering::SeqCst);
    let frames_final = probe.frames.load(Ordering::SeqCst);

    assert_eq!(
        polls_final, polls_after_drop,
        "polls must not grow after drop (detached drain)"
    );
    assert!(
        frames_final <= frames_at_drop.saturating_add(RACE_ALLOWANCE_FRAMES),
        "frames grew beyond race allowance: at_drop={frames_at_drop} final={frames_final}"
    );
    assert!(
        frames_final < total_chunks,
        "must not drain full body: frames={frames_final} total_chunks={total_chunks}"
    );
}

/// TEST-CANCEL-001 — unknown-length streaming (no Content-Length).
#[tokio::test]
async fn test_cancel_001_unknown_length_disconnect_stops_upstream_polling() {
    let probe = BodyProbe::new();
    let total_chunks = 64usize;
    let mut chunks = Vec::with_capacity(total_chunks);
    chunks.push(Bytes::from_static(b"first"));
    for i in 1..total_chunks {
        chunks.push(Bytes::from(vec![b'x'; 64]));
        let _ = i;
    }

    let response = Response::builder()
        .status(200)
        .body(boxed_instrumented(Arc::clone(&probe), chunks))
        .expect("response");

    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    let ProxyDispatchOutcome::Streaming { stream, .. } = outcome else {
        panic!("expected Streaming for unknown-length, got {outcome:?}");
    };

    let mut resp = take_streaming(stream).expect("streaming handle");
    let first = resp.body_mut().frame().await;
    assert!(matches!(first, Some(Ok(_))), "first frame must deliver");
    let frames_at_first = probe.frames.load(Ordering::SeqCst);
    assert!(frames_at_first >= 1);

    // Downstream disconnect: drop response (headers already taken; body dropped).
    drop(resp);

    let frames_at_drop = probe.frames.load(Ordering::SeqCst);
    assert_upstream_stops_after_drop(&probe, frames_at_drop, total_chunks).await;
}

/// TEST-CANCEL-002 — known large Content-Length → Streaming path (no collect).
#[tokio::test]
async fn test_cancel_002_known_large_cl_disconnect_stops_upstream_polling() {
    let probe = BodyProbe::new();
    let total_chunks = 64usize;
    let chunk_len = 512usize;
    let mut chunks = Vec::with_capacity(total_chunks);
    chunks.push(Bytes::from(vec![b'a'; chunk_len]));
    for _ in 1..total_chunks {
        chunks.push(Bytes::from(vec![b'b'; chunk_len]));
    }
    let declared = total_chunks * chunk_len;
    assert!(
        declared > BENCH_SMALL_UPSTREAM_BODY,
        "declared={declared} threshold={BENCH_SMALL_UPSTREAM_BODY}"
    );

    let response = Response::builder()
        .status(200)
        .header("content-length", declared.to_string())
        .body(boxed_instrumented(Arc::clone(&probe), chunks))
        .expect("response");

    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    let ProxyDispatchOutcome::Streaming { stream, .. } = outcome else {
        panic!("expected Streaming for large CL, got {outcome:?}");
    };

    let mut resp = take_streaming(stream).expect("handle");
    let _ = resp.body_mut().frame().await.expect("first").expect("ok");
    drop(resp);

    let frames_at_drop = probe.frames.load(Ordering::SeqCst);
    assert_upstream_stops_after_drop(&probe, frames_at_drop, total_chunks).await;
}

/// TEST-CANCEL-003 — small-CL probe exceeds → ClassifyPrefixThenBody streaming.
#[tokio::test]
async fn test_cancel_003_prefix_then_body_disconnect_stops_remainder_polling() {
    let probe = BodyProbe::new();
    // Probe bound = declared CL = 2. First chunk "ab" accepted; second starts overflow.
    let mut chunks = vec![Bytes::from_static(b"ab"), Bytes::from_static(b"cd")];
    let remainder_chunks = 48usize;
    for _ in 0..remainder_chunks {
        chunks.push(Bytes::from(vec![b'z'; 64]));
    }
    let total_upstream_chunks = chunks.len();

    let response = Response::builder()
        .status(200)
        .header("content-length", "2")
        .body(boxed_instrumented(Arc::clone(&probe), chunks))
        .expect("response");

    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    let ProxyDispatchOutcome::Streaming { stream, .. } = outcome else {
        panic!("expected Streaming fallback after false small CL, got {outcome:?}");
    };

    // Probe already consumed "ab"+"cd". Reminder body still holds many chunks.
    let frames_after_classify = probe.frames.load(Ordering::SeqCst);
    assert!(
        frames_after_classify >= 2,
        "probe must have polled overflow chunks"
    );

    let mut resp = take_streaming(stream).expect("handle");
    // Consume prefix / first_chunk from ClassifyPrefixThenBody (no remainder poll required).
    let _ = resp.body_mut().frame().await;
    drop(resp);

    let frames_at_drop = probe.frames.load(Ordering::SeqCst);
    assert_upstream_stops_after_drop(&probe, frames_at_drop, total_upstream_chunks).await;

    // Remainder must not have been fully drained after disconnect.
    assert!(
        probe.frames.load(Ordering::SeqCst) < total_upstream_chunks,
        "remainder drained fully"
    );
}

/// TEST-CANCEL-004 — SSE /api/stream forced streaming path.
#[tokio::test]
async fn test_cancel_004_sse_path_disconnect_stops_upstream_polling() {
    let probe = BodyProbe::new();
    let total_chunks = 32usize;
    let mut chunks = Vec::with_capacity(total_chunks);
    chunks.push(Bytes::from_static(b"data: one\n\n"));
    for _ in 1..total_chunks {
        chunks.push(Bytes::from_static(b"data: more\n\n"));
    }

    let response = Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .body(boxed_instrumented(Arc::clone(&probe), chunks))
        .expect("response");

    let outcome = classify_hyper_response(response, "/api/stream", ProxyMethod::Get).await;
    let ProxyDispatchOutcome::Streaming { stream, .. } = outcome else {
        panic!("expected Streaming for SSE, got {outcome:?}");
    };

    let mut resp = take_streaming(stream).expect("handle");
    let _ = resp.body_mut().frame().await.expect("first").expect("ok");
    drop(resp);

    let frames_at_drop = probe.frames.load(Ordering::SeqCst);
    assert_upstream_stops_after_drop(&probe, frames_at_drop, total_chunks).await;
}
