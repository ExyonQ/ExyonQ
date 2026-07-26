//! SEC-PROXY-003 — cache passthrough must not second-collect; stream prefix+remainder.
//! Lives under tests/ so hot-path BoxBody budgets on src/ are unchanged.

use bytes::Bytes;
use exyonq_cache::{build_storage_cache_key, CacheKeyParts, ResponseCache, Singleflight};
use exyonq_mod_proxy::{
    prepare_proxy_cache_load, reset_proxy_cache_metrics_for_tests, serve_proxy_with_cache,
    ProxyCacheLoad, PROXY_CACHE_NAMESPACE,
};
use exyonq_module_api::{CacheRejection, CompiledCachePolicy};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame};
use hyper::{Method, Response, StatusCode};
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::Notify;

fn policy(max_object_bytes: usize) -> CompiledCachePolicy {
    CompiledCachePolicy {
        name: "sec-proxy-003".into(),
        ttl: Duration::from_secs(60),
        max_object_bytes,
        policy_generation: 1,
    }
}

fn test_key(path: &str) -> exyonq_cache::CacheKey {
    build_storage_cache_key(CacheKeyParts {
        site_id: 0,
        namespace: 0,
        backend_id: 0,
        runtime_generation: 1,
        policy_generation: 1,
        route_idx: 0,
        method: "GET".into(),
        scheme: "http".into(),
        host: "test".into(),
        path: path.into(),
        query: String::new(),
        content_encoding: "identity".into(),
    })
}

fn full_body(bytes: Bytes) -> http_body_util::combinators::BoxBody<Bytes, hyper::Error> {
    http_body_util::Full::from(bytes)
        .map_err(|never| match never {})
        .boxed()
}

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
                this.probe.frames.fetch_add(1, Ordering::SeqCst);
                this.probe.bytes.fetch_add(chunk.len(), Ordering::SeqCst);
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

async fn serve_load(load: ProxyCacheLoad, path: &str, max: usize) -> Response<
    http_body_util::combinators::BoxBody<Bytes, hyper::Error>,
> {
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let singleflight = Singleflight::new();
    let key = test_key(path);
    let pol = policy(max);
    serve_proxy_with_cache(
        &cache,
        &singleflight,
        key,
        0,
        1,
        &Method::GET,
        &pol,
        || async move { load },
    )
    .await
}

async fn collect_body(
    resp: Response<http_body_util::combinators::BoxBody<Bytes, hyper::Error>>,
) -> Bytes {
    resp.into_body()
        .collect()
        .await
        .expect("collect client body")
        .to_bytes()
}

const RACE_ALLOWANCE_FRAMES: usize = 1;

async fn assert_stops_after_drop(probe: &BodyProbe, frames_at_drop: usize, total_chunks: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
    while tokio::time::Instant::now() < deadline {
        if probe.dropped.load(Ordering::SeqCst) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        probe.dropped.load(Ordering::SeqCst),
        "upstream body must drop after downstream cancel"
    );
    let polls_after = probe.polls.load(Ordering::SeqCst);
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        probe.polls.load(Ordering::SeqCst),
        polls_after,
        "polls must not grow after drop"
    );
    let frames_final = probe.frames.load(Ordering::SeqCst);
    assert!(
        frames_final <= frames_at_drop.saturating_add(RACE_ALLOWANCE_FRAMES),
        "frames grew: at_drop={frames_at_drop} final={frames_final}"
    );
    assert!(
        frames_final < total_chunks,
        "full drain: frames={frames_final} total={total_chunks}"
    );
}

#[tokio::test]
async fn body_under_max_materializes_and_inserts() {
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from_static(b"ok")))
        .unwrap();
    let load = prepare_proxy_cache_load(response, 1024, &[]).await;
    match &load {
        ProxyCacheLoad::Materialized {
            cacheable,
            body,
            ..
        } => {
            assert!(*cacheable);
            assert_eq!(&body[..], b"ok");
        }
        ProxyCacheLoad::Passthrough { .. } => panic!("expected materialized"),
    }
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let singleflight = Singleflight::new();
    let key = test_key("/under");
    let pol = policy(1024);
    let resp = serve_proxy_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::GET,
        &pol,
        || async move { load },
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(collect_body(resp).await.as_ref(), b"ok");
    assert!(cache.lookup(&key).is_some());
}

#[tokio::test]
async fn body_exactly_max_materializes_and_inserts() {
    let body = vec![b'z'; 64];
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from(body.clone())))
        .unwrap();
    let load = prepare_proxy_cache_load(response, 64, &[]).await;
    assert!(matches!(
        load,
        ProxyCacheLoad::Materialized {
            cacheable: true,
            ..
        }
    ));
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let singleflight = Singleflight::new();
    let key = test_key("/exact");
    let pol = policy(64);
    let resp = serve_proxy_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::GET,
        &pol,
        || async move { load },
    )
    .await;
    assert_eq!(collect_body(resp).await.as_ref(), &body);
    assert!(cache.lookup(&key).is_some());
}

#[tokio::test]
async fn body_max_plus_one_exact_eof_materializes_without_insert() {
    // read_bounded cap = max+1; EOF exactly at cap → Complete, then assess rejects store.
    let body = vec![b'x'; 65];
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from(body.clone())))
        .unwrap();
    let load = prepare_proxy_cache_load(response, 64, &[]).await;
    match &load {
        ProxyCacheLoad::Materialized {
            cacheable,
            rejection,
            ..
        } => {
            assert!(!*cacheable);
            assert_eq!(*rejection, Some(CacheRejection::BodyTooLarge));
        }
        ProxyCacheLoad::Passthrough { .. } => panic!("exact max+1 EOF is Complete, not stream"),
    }
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let singleflight = Singleflight::new();
    let key = test_key("/max1");
    let pol = policy(64);
    let resp = serve_proxy_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::GET,
        &pol,
        || async move { load },
    )
    .await;
    assert_eq!(collect_body(resp).await.as_ref(), &body);
    assert!(cache.lookup(&key).is_none());
}

#[tokio::test]
async fn body_beyond_cap_passthrough_streams_without_insert() {
    // One more byte than cap (max+2) forces Oversized → PrefixThenBody passthrough.
    let body = vec![b'x'; 66];
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from(body.clone())))
        .unwrap();
    let load = prepare_proxy_cache_load(response, 64, &[]).await;
    match &load {
        ProxyCacheLoad::Passthrough { rejection, .. } => {
            assert_eq!(*rejection, CacheRejection::BodyTooLarge);
        }
        ProxyCacheLoad::Materialized { .. } => panic!("expected passthrough"),
    }
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let singleflight = Singleflight::new();
    let key = test_key("/max2");
    let pol = policy(64);
    let resp = serve_proxy_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::GET,
        &pol,
        || async move { load },
    )
    .await;
    assert_eq!(collect_body(resp).await.as_ref(), &body);
    assert!(cache.lookup(&key).is_none());
}

#[tokio::test]
async fn large_body_no_second_collect_before_downstream_read() {
    let probe = BodyProbe::new();
    let total_chunks = 40usize;
    let mut chunks = Vec::with_capacity(total_chunks);
    // Many small frames so fill crosses max=32 after several polls.
    for _ in 0..total_chunks {
        chunks.push(Bytes::from(vec![b'a'; 8]));
    }
    let expected_len = total_chunks * 8;
    let response = Response::builder()
        .status(200)
        .body(boxed_instrumented(Arc::clone(&probe), chunks))
        .unwrap();

    let load = prepare_proxy_cache_load(response, 32, &[]).await;
    assert!(matches!(load, ProxyCacheLoad::Passthrough { .. }));
    let frames_after_fill = probe.frames.load(Ordering::SeqCst);
    assert!(
        frames_after_fill < total_chunks,
        "fill must stop at bound: frames={frames_after_fill}"
    );

    let mut resp = serve_load(load, "/nocollect", 32).await;
    let frames_after_serve = probe.frames.load(Ordering::SeqCst);
    // Evidence: serve must not drain remaining frames (second collect removed).
    assert_eq!(
        frames_after_serve, frames_after_fill,
        "second full collect must not run during serve_proxy_with_cache"
    );

    let collected = resp.body_mut().collect().await.expect("stream").to_bytes();
    assert_eq!(collected.len(), expected_len);
    assert_eq!(probe.frames.load(Ordering::SeqCst), total_chunks);
}

#[tokio::test]
async fn many_small_frames_crossing_limit_no_loss_or_dup() {
    let mut chunks = Vec::new();
    for i in 0..20u8 {
        chunks.push(Bytes::from(vec![i; 5]));
    }
    let expected: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
    let response = Response::builder()
        .status(200)
        .body({
            let probe = BodyProbe::new();
            boxed_instrumented(probe, chunks)
        })
        .unwrap();
    let load = prepare_proxy_cache_load(response, 33, &[]).await;
    assert!(matches!(load, ProxyCacheLoad::Passthrough { .. }));
    let got = collect_body(serve_load(load, "/frames", 33).await).await;
    assert_eq!(got.as_ref(), expected.as_slice());
}

#[tokio::test]
async fn single_large_frame_crossing_limit_preserves_bytes() {
    let body = Bytes::from(vec![b'q'; 200]);
    let response = Response::builder()
        .status(200)
        .body(full_body(body.clone()))
        .unwrap();
    let load = prepare_proxy_cache_load(response, 50, &[]).await;
    assert!(matches!(load, ProxyCacheLoad::Passthrough { .. }));
    let got = collect_body(serve_load(load, "/bigframe", 50).await).await;
    assert_eq!(got, body);
}

#[tokio::test]
async fn content_length_over_max_streams_without_fill_collect() {
    let probe = BodyProbe::new();
    let total_chunks = 30usize;
    let mut chunks = Vec::with_capacity(total_chunks);
    for _ in 0..total_chunks {
        chunks.push(Bytes::from(vec![b'c'; 64]));
    }
    let declared = total_chunks * 64;
    let response = Response::builder()
        .status(200)
        .header("content-length", declared.to_string())
        .body(boxed_instrumented(Arc::clone(&probe), chunks))
        .unwrap();

    let load = prepare_proxy_cache_load(response, 128, &[]).await;
    match &load {
        ProxyCacheLoad::Passthrough { rejection, .. } => {
            assert_eq!(*rejection, CacheRejection::BodyTooLarge);
        }
        ProxyCacheLoad::Materialized { .. } => panic!("expected early passthrough"),
    }
    // Early CL bypass must not poll body during prepare.
    assert_eq!(probe.frames.load(Ordering::SeqCst), 0);

    let mut resp = serve_load(load, "/cl-over", 128).await;
    assert_eq!(probe.frames.load(Ordering::SeqCst), 0);

    let got = resp.body_mut().collect().await.expect("stream").to_bytes();
    assert_eq!(got.len(), declared);
}

#[tokio::test]
async fn unknown_length_oversize_passthrough_complete() {
    let body = vec![b'u'; 500];
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from(body.clone())))
        .unwrap();
    let load = prepare_proxy_cache_load(response, 64, &[]).await;
    assert!(matches!(load, ProxyCacheLoad::Passthrough { .. }));
    assert_eq!(
        collect_body(serve_load(load, "/unk", 64).await).await.as_ref(),
        &body
    );
}

#[tokio::test]
async fn false_small_content_length_still_streams_full_body() {
    // Declared CL under max, real body over max → Oversized passthrough.
    let body = vec![b'f'; 200];
    let response = Response::builder()
        .status(200)
        .header("content-length", "10")
        .body(full_body(Bytes::from(body.clone())))
        .unwrap();
    let load = prepare_proxy_cache_load(response, 64, &[]).await;
    assert!(matches!(load, ProxyCacheLoad::Passthrough { .. }));
    assert_eq!(
        collect_body(serve_load(load, "/false-cl", 64).await)
            .await
            .as_ref(),
        &body
    );
}

#[tokio::test]
async fn sse_passthrough_streams_without_insert() {
    let response = Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .body(full_body(Bytes::from_static(b"data: one\n\n")))
        .unwrap();
    let load = prepare_proxy_cache_load(response, 1024, &[]).await;
    match &load {
        ProxyCacheLoad::Passthrough { rejection, .. } => {
            assert_eq!(*rejection, CacheRejection::ResponseStreaming);
        }
        ProxyCacheLoad::Materialized { .. } => panic!("SSE must passthrough"),
    }
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let singleflight = Singleflight::new();
    let key = test_key("/sse");
    let pol = policy(1024);
    let resp = serve_proxy_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::GET,
        &pol,
        || async move { load },
    )
    .await;
    assert_eq!(collect_body(resp).await.as_ref(), b"data: one\n\n");
    assert!(cache.lookup(&key).is_none());
}

#[tokio::test]
async fn non_cacheable_auth_materialized_no_insert() {
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from_static(b"secret")))
        .unwrap();
    let req_headers = vec![("authorization".into(), "Bearer x".into())];
    let load = prepare_proxy_cache_load(response, 1024, &req_headers).await;
    match &load {
        ProxyCacheLoad::Materialized {
            cacheable,
            rejection,
            ..
        } => {
            assert!(!*cacheable);
            assert_eq!(*rejection, Some(CacheRejection::RequestAuthorization));
        }
        ProxyCacheLoad::Passthrough { .. } => panic!("expected materialized bypass"),
    }
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let singleflight = Singleflight::new();
    let key = test_key("/auth");
    let pol = policy(1024);
    let resp = serve_proxy_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::GET,
        &pol,
        || async move { load },
    )
    .await;
    assert_eq!(collect_body(resp).await.as_ref(), b"secret");
    assert!(cache.lookup(&key).is_none());
}

#[tokio::test]
async fn cache_passthrough_disconnect_stops_upstream_polling() {
    let probe = BodyProbe::new();
    let total_chunks = 48usize;
    let mut chunks = Vec::with_capacity(total_chunks);
    chunks.push(Bytes::from_static(b"first"));
    for _ in 1..total_chunks {
        chunks.push(Bytes::from(vec![b'z'; 32]));
    }
    let declared = 10_000usize;
    let response = Response::builder()
        .status(200)
        .header("content-length", declared.to_string())
        .body(boxed_instrumented(Arc::clone(&probe), chunks))
        .unwrap();

    let load = prepare_proxy_cache_load(response, 64, &[]).await;
    assert!(matches!(load, ProxyCacheLoad::Passthrough { .. }));
    assert_eq!(probe.frames.load(Ordering::SeqCst), 0);

    let mut resp = serve_load(load, "/cancel", 64).await;
    let first = resp.body_mut().frame().await;
    assert!(matches!(first, Some(Ok(_))));
    let frames_at_first = probe.frames.load(Ordering::SeqCst);
    assert!(frames_at_first >= 1);

    drop(resp);
    let frames_at_drop = probe.frames.load(Ordering::SeqCst);
    assert_stops_after_drop(&probe, frames_at_drop, total_chunks).await;
}

#[tokio::test]
async fn cache_hit_still_serves_buffered_entry() {
    reset_proxy_cache_metrics_for_tests();
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let singleflight = Singleflight::new();
    let key = test_key("/hit");
    let pol = policy(1024);
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from_static(b"cached")))
        .unwrap();
    let load = prepare_proxy_cache_load(response, 1024, &[]).await;
    let resp1 = serve_proxy_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::GET,
        &pol,
        || async move { load },
    )
    .await;
    assert_eq!(collect_body(resp1).await.as_ref(), b"cached");

    let resp2 = serve_proxy_with_cache(
        &cache,
        &singleflight,
        key,
        0,
        1,
        &Method::GET,
        &pol,
        || async {
            panic!("load must not run on cache hit");
        },
    )
    .await;
    assert_eq!(collect_body(resp2).await.as_ref(), b"cached");
    let _ = PROXY_CACHE_NAMESPACE;
}
