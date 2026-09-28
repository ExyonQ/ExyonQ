//! Plan 12 tranche 4 — bounded proxy microcache.

use bytes::Bytes;
use exyonq_cache::{cache_misses_total, reset_metrics_for_tests};
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{AppConfig, ProxyDispatchTestGuard};
use exyonq_mod_proxy::{
    cache_proxy_hits_total, cache_proxy_insertions_total, cache_proxy_misses_total,
    cache_proxy_rejections_total, reset_proxy_cache_metrics_for_tests, ProxyRuntime,
};
use http_body_util::BodyExt;
use hyper::service::service_fn;
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioExecutor;
use hyper_util::server::conn::auto::Builder;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;

fn reset_cache_metrics_for_tests() {
    reset_metrics_for_tests();
    reset_proxy_cache_metrics_for_tests();
}

static PLAN12_PROXY_CACHE_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct ProxyHarness {
    ctx: Arc<ConnectionContext>,
    _proxy_guard: ProxyDispatchTestGuard,
    _suite_gate: std::sync::MutexGuard<'static, ()>,
}

async fn spawn_upstream(
    hits: Arc<AtomicUsize>,
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: Vec<u8>,
) -> String {
    spawn_upstream_fn(hits, status, headers, Arc::new(move || body.clone())).await
}

async fn spawn_upstream_fn(
    hits: Arc<AtomicUsize>,
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body_fn: Arc<dyn Fn() -> Vec<u8> + Send + Sync>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.expect("accept");
            let hits = Arc::clone(&hits);
            let headers = headers.clone();
            let body_fn = body_fn.clone();
            tokio::spawn(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                let service = service_fn(move |_req| {
                    let hits = Arc::clone(&hits);
                    let headers = headers.clone();
                    let body = body_fn();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        let mut builder = hyper::Response::builder().status(status);
                        for (name, value) in headers {
                            builder = builder.header(name, value);
                        }
                        Ok::<_, hyper::Error>(
                            builder
                                .body(http_body_util::Full::new(Bytes::from(body)))
                                .expect("response"),
                        )
                    }
                });
                let _ = Builder::new(TokioExecutor::new())
                    .serve_connection(io, service)
                    .await;
            });
        }
    });
    format!("http://{addr}")
}

async fn spawn_chunked_upstream(hits: Arc<AtomicUsize>, chunks: Vec<Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let hits = Arc::clone(&hits);
            let chunks = chunks.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                hits.fetch_add(1, Ordering::SeqCst);
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
                    .await;
                for chunk in chunks {
                    let line = format!("{:x}\r\n", chunk.len());
                    let _ = stream.write_all(line.as_bytes()).await;
                    let _ = stream.write_all(&chunk).await;
                    let _ = stream.write_all(b"\r\n").await;
                }
                let _ = stream.write_all(b"0\r\n\r\n").await;
                let _ = stream.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

async fn spawn_sse_upstream(hits: Arc<AtomicUsize>) -> String {
    spawn_upstream_fn(
        hits,
        200,
        vec![("content-type", "text/event-stream")],
        Arc::new(|| b"data: one\n\n".to_vec()),
    )
    .await
}

async fn spawn_raw_upstream(hits: Arc<AtomicUsize>, header: Vec<u8>, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let hits = Arc::clone(&hits);
            let header = header.clone();
            let body = body.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                hits.fetch_add(1, Ordering::SeqCst);
                let _ = stream.write_all(&header).await;
                let _ = stream.write_all(&body).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}
async fn spawn_aborting_upstream(hits: Arc<AtomicUsize>, prefix: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let hits = Arc::clone(&hits);
            let prefix = prefix.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                hits.fetch_add(1, Ordering::SeqCst);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    prefix.len() + 64
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(&prefix).await;
                drop(stream);
            });
        }
    });
    format!("http://{addr}")
}

#[allow(clippy::await_holding_lock)]
async fn proxy_cache_ctx(
    upstream_target: &str,
    ttl_seconds: u64,
    max_object_bytes: usize,
) -> ProxyHarness {
    let suite_gate = PLAN12_PROXY_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_cache_metrics_for_tests();
    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]

[[cache_policy]]
name = "public-api"
ttl_seconds = {ttl_seconds}
max_object_bytes = {max_object_bytes}

[[route]]
name = "api"
match = {{ path = "/public/" }}
upstream = "backend"
cache = "public-api"

[[upstream]]
name = "backend"
target = "{upstream_target}"
timeout_ms = 5000
"#
    );
    let config: AppConfig = raw.parse().expect("config");
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let proxy_runtime = Arc::new(ProxyRuntime::new());
    exyonq_mod_proxy::install_kernel_hooks(Arc::clone(&proxy_runtime));
    let proxy_service: Arc<dyn exyonq_module_api::proxy_dispatch::ProxyDispatchService> =
        proxy_runtime;
    exyonq_core::execute_backend::clear_global_proxy_dispatch_for_register_once_test();
    let _ = exyonq_core::register_proxy_dispatch_service(Arc::clone(&proxy_service));
    let proxy_guard = ProxyDispatchTestGuard::install(proxy_service);
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    ProxyHarness {
        ctx: Arc::new(ConnectionContext {
            state,
            proxy_client: proxy_client.clone(),
            x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        }),
        _proxy_guard: proxy_guard,
        _suite_gate: suite_gate,
    }
}

async fn get_status_body(ctx: &ConnectionContext, uri: &str) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(())
        .expect("req");
    let response = serve_http3_request(ctx.clone(), req).await;
    let status = response.status();
    let body = response.into_body().collect().await.expect("collect");
    (status, body.to_bytes().to_vec())
}

#[tokio::test]
async fn proxy_get_second_request_is_cache_hit() {
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![("content-type", "text/plain")],
        b"proxy-body".to_vec(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 1024 * 1024).await;
    let uri = "http://127.0.0.1/public/data.txt";
    let before_proxy_miss = cache_proxy_misses_total();
    let before_proxy_hit = cache_proxy_hits_total();

    for _ in 0..2 {
        let (status, body) = get_status_body(harness.ctx.as_ref(), uri).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"proxy-body");
    }

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(cache_proxy_misses_total(), before_proxy_miss + 1);
    assert_eq!(cache_proxy_hits_total(), before_proxy_hit + 1);
    assert_eq!(cache_proxy_insertions_total(), 1);
}

#[tokio::test]
async fn distinct_query_strings_do_not_share_proxy_cache() {
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![("content-type", "text/plain")],
        b"x".to_vec(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 1024).await;
    get_status_body(harness.ctx.as_ref(), "http://127.0.0.1/public/x?a=1").await;
    get_status_body(harness.ctx.as_ref(), "http://127.0.0.1/public/x?a=2").await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn head_hits_after_get_without_body() {
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![("content-type", "text/plain")],
        b"head-body".to_vec(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 1024).await;
    get_status_body(harness.ctx.as_ref(), "http://127.0.0.1/public/h.txt").await;
    let req = Request::builder()
        .method(Method::HEAD)
        .uri("http://127.0.0.1/public/h.txt")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx.as_ref().clone(), req).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("content-length").unwrap(), "9");
    let body = response.into_body().collect().await.expect("collect");
    assert!(body.to_bytes().is_empty());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn set_cookie_response_is_never_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![("content-type", "text/plain"), ("set-cookie", "s=v")],
        b"c".to_vec(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 1024).await;
    let uri = "http://127.0.0.1/public/cookie.txt";
    get_status_body(harness.ctx.as_ref(), uri).await;
    get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_proxy_insertions_total(), 0);
    assert!(cache_proxy_rejections_total() >= 1);
}

#[tokio::test]
async fn authorization_bypasses_proxy_cache() {
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![("content-type", "text/plain")],
        b"z".to_vec(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 1024).await;
    let uri = "http://127.0.0.1/public/auth.txt";
    for _ in 0..2 {
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header("Authorization", "Bearer x")
            .body(())
            .expect("req");
        let response = serve_http3_request(harness.ctx.as_ref().clone(), req).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_proxy_hits_total(), 0);
}

#[tokio::test]
async fn oversized_upstream_response_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let big = vec![b'x'; 2048];
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![("content-type", "application/octet-stream")],
        big.clone(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 512).await;
    let uri = "http://127.0.0.1/public/big.bin";
    let (status, body) = get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.len(), 2048);
    get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_proxy_insertions_total(), 0);
}

#[tokio::test]
async fn known_content_length_over_max_bypasses_materialization() {
    let hits = Arc::new(AtomicUsize::new(0));
    let body = vec![b'a'; 2048];
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![
            ("content-type", "application/octet-stream"),
            ("content-length", "2048"),
        ],
        body.clone(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 512).await;
    let uri = "http://127.0.0.1/public/cl-big.bin";
    let before_reject = cache_proxy_rejections_total();
    let (status, got) = get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, body);
    get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_proxy_insertions_total(), 0);
    assert!(cache_proxy_rejections_total() > before_reject);
}

#[tokio::test]
async fn chunked_body_over_limit_delivers_full_without_cache() {
    let hits = Arc::new(AtomicUsize::new(0));
    let chunk_a = vec![0u8; 300];
    let chunk_b = vec![1u8; 300];
    let chunk_c = vec![2u8; 300];
    let expected: Vec<u8> = chunk_a
        .iter()
        .chain(chunk_b.iter())
        .chain(chunk_c.iter())
        .copied()
        .collect();
    let upstream = spawn_chunked_upstream(Arc::clone(&hits), vec![chunk_a, chunk_b, chunk_c]).await;
    let harness = proxy_cache_ctx(&upstream, 30, 512).await;
    let uri = "http://127.0.0.1/public/chunked.bin";
    let (status, got) = get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, expected);
    get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_proxy_insertions_total(), 0);
}

#[tokio::test]
async fn body_exactly_at_max_object_bytes_is_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let body = vec![b'z'; 512];
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![("content-type", "application/octet-stream")],
        body.clone(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 512).await;
    let uri = "http://127.0.0.1/public/exact.bin";
    get_status_body(harness.ctx.as_ref(), uri).await;
    get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(cache_proxy_insertions_total(), 1);
}

#[tokio::test]
async fn body_one_byte_over_max_is_not_cached_but_complete() {
    let hits = Arc::new(AtomicUsize::new(0));
    let body = vec![b'y'; 513];
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![("content-type", "application/octet-stream")],
        body.clone(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 512).await;
    let uri = "http://127.0.0.1/public/plusone.bin";
    let (status, got) = get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got.len(), 513);
    assert_eq!(got, body);
    get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_proxy_insertions_total(), 0);
}

#[tokio::test]
async fn lying_content_length_never_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let body = vec![b'm'; 10];
    let header = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 100\r\nConnection: close\r\n\r\n".to_vec();
    let upstream = spawn_raw_upstream(Arc::clone(&hits), header, body.clone()).await;
    let harness = proxy_cache_ctx(&upstream, 30, 1024).await;
    let uri = "http://127.0.0.1/public/lie.bin";
    get_status_body(harness.ctx.as_ref(), uri).await;
    get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_proxy_insertions_total(), 0);
}

#[tokio::test]
async fn sse_stream_bypasses_materialization() {
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_sse_upstream(Arc::clone(&hits)).await;
    let harness = proxy_cache_ctx(&upstream, 30, 1024).await;
    let uri = "http://127.0.0.1/public/sse.txt";
    let (status, body) = get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"data: one\n\n");
    get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_proxy_insertions_total(), 0);
}

#[tokio::test]
async fn read_error_after_prefix_does_not_insert() {
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_aborting_upstream(Arc::clone(&hits), vec![b'p'; 64]).await;
    let harness = proxy_cache_ctx(&upstream, 30, 1024).await;
    let uri = "http://127.0.0.1/public/abort.bin";
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx.as_ref().clone(), req).await;
    let _ = response.into_body().collect().await;
    assert_eq!(cache_proxy_insertions_total(), 0);
}

#[tokio::test]
async fn head_oversized_returns_empty_body_and_does_not_cache() {
    let hits = Arc::new(AtomicUsize::new(0));
    let body = vec![b'h'; 2048];
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![
            ("content-type", "application/octet-stream"),
            ("content-length", "2048"),
        ],
        body,
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 512).await;
    let uri = "http://127.0.0.1/public/head-big.bin";
    let req = Request::builder()
        .method(Method::HEAD)
        .uri(uri)
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx.as_ref().clone(), req).await;
    assert_eq!(response.status(), StatusCode::OK);
    let collected = response.into_body().collect().await.expect("collect");
    assert!(collected.to_bytes().is_empty());
    let (status, got) = get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got.len(), 2048);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_proxy_insertions_total(), 0);
}

#[tokio::test]
async fn new_runtime_generation_forces_proxy_miss() {
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_upstream(
        Arc::clone(&hits),
        200,
        vec![("content-type", "text/plain")],
        b"g".to_vec(),
    )
    .await;
    let harness = proxy_cache_ctx(&upstream, 30, 1024).await;
    get_status_body(harness.ctx.as_ref(), "http://127.0.0.1/public/g.txt").await;
    let config = harness.ctx.state.config.clone();
    let proxy_client = harness.ctx.proxy_client.clone();
    let state = ServerState::new_with_generation(2, config, proxy_client.clone())
        .await
        .expect("state gen2");
    let ctx = Arc::new(ConnectionContext {
        state,
        proxy_client,
        x_forwarded_for: harness.ctx.x_forwarded_for.clone(),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    });
    get_status_body(ctx.as_ref(), "http://127.0.0.1/public/g.txt").await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert!(cache_misses_total() >= 2);
}
