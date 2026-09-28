//! Plan 12 tranche 4 — proxy cache singleflight.

use bytes::Bytes;
use exyonq_cache::reset_metrics_for_tests;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::AppConfig;
use exyonq_mod_proxy::{cache_proxy_insertions_total, reset_proxy_cache_metrics_for_tests};
use http_body_util::BodyExt;
use hyper::service::service_fn;
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioExecutor;
use hyper_util::server::conn::auto::Builder;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Barrier as AsyncBarrier;

fn reset_cache_metrics_for_tests() {
    reset_metrics_for_tests();
    reset_proxy_cache_metrics_for_tests();
}

static PLAN12_PROXY_CACHE_CONCURRENCY_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Plan12ProxyGuard(#[allow(dead_code)] exyonq_core::execute_backend::ProxyDispatchTestGuard);

fn install_plan12_proxy() -> Plan12ProxyGuard {
    let proxy_runtime = Arc::new(exyonq_mod_proxy::ProxyRuntime::new());
    exyonq_mod_proxy::install_kernel_hooks(Arc::clone(&proxy_runtime));
    let proxy_service: Arc<dyn exyonq_module_api::proxy_dispatch::ProxyDispatchService> =
        proxy_runtime;
    exyonq_core::execute_backend::clear_global_proxy_dispatch_for_register_once_test();
    let _ = exyonq_core::register_proxy_dispatch_service(Arc::clone(&proxy_service));
    Plan12ProxyGuard(exyonq_core::execute_backend::ProxyDispatchTestGuard::install(proxy_service))
}

async fn spawn_counting_upstream(hits: Arc<AtomicUsize>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.expect("accept");
            let hits = Arc::clone(&hits);
            tokio::spawn(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                let service = service_fn(move |_req| {
                    let hits = Arc::clone(&hits);
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, hyper::Error>(
                            hyper::Response::builder()
                                .status(200)
                                .header("content-type", "text/plain")
                                .body(http_body_util::Full::new(Bytes::from_static(
                                    b"once-upstream",
                                )))
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

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
#[allow(clippy::await_holding_lock)]
async fn proxy_singleflight_deduplicates_concurrent_misses() {
    let _gate = PLAN12_PROXY_CACHE_CONCURRENCY_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _proxy = install_plan12_proxy();
    reset_cache_metrics_for_tests();
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_counting_upstream(Arc::clone(&hits)).await;
    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]

[[cache_policy]]
name = "public-api"
ttl_seconds = 30
max_object_bytes = 1048576

[[route]]
name = "api"
match = {{ path = "/public/" }}
upstream = "backend"
cache = "public-api"

[[upstream]]
name = "backend"
target = "{upstream}"
timeout_ms = 5000
"#
    );
    let config: AppConfig = raw.parse().expect("config");
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    let ctx = Arc::new(ConnectionContext {
        state,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    });
    let barrier = Arc::new(AsyncBarrier::new(32));
    let uri = "http://127.0.0.1/public/coalesce.txt";

    let mut handles = Vec::new();
    for _ in 0..32 {
        let ctx = Arc::clone(&ctx);
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let req = Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(())
                .expect("req");
            let response = serve_http3_request(ctx.as_ref().clone(), req).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = response.into_body().collect().await.expect("body");
            assert_eq!(body.to_bytes().as_ref(), b"once-upstream");
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(cache_proxy_insertions_total(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
#[allow(clippy::await_holding_lock)]
async fn proxy_singleflight_oversized_notifies_and_releases_flight() {
    let _gate = PLAN12_PROXY_CACHE_CONCURRENCY_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _proxy = install_plan12_proxy();
    reset_cache_metrics_for_tests();
    let hits = Arc::new(AtomicUsize::new(0));
    let body = vec![b'o'; 2048];
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let hits_for_upstream = Arc::clone(&hits);
    let body_for_spawn = body.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.expect("accept");
            let hits = Arc::clone(&hits_for_upstream);
            let body = body_for_spawn.clone();
            tokio::spawn(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                let service = service_fn(move |_req| {
                    let hits = Arc::clone(&hits);
                    let body = body.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, hyper::Error>(
                            hyper::Response::builder()
                                .status(200)
                                .header("content-type", "application/octet-stream")
                                .header("content-length", body.len())
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
    let upstream = format!("http://{addr}");
    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]

[[cache_policy]]
name = "public-api"
ttl_seconds = 30
max_object_bytes = 512

[[route]]
name = "api"
match = {{ path = "/public/" }}
upstream = "backend"
cache = "public-api"

[[upstream]]
name = "backend"
target = "{upstream}"
timeout_ms = 5000
"#
    );
    let config: AppConfig = raw.parse().expect("config");
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    let ctx = Arc::new(ConnectionContext {
        state,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    });
    let barrier = Arc::new(AsyncBarrier::new(32));
    let uri = "http://127.0.0.1/public/sf-big.bin";

    let mut handles = Vec::new();
    for _ in 0..32 {
        let ctx = Arc::clone(&ctx);
        let barrier = Arc::clone(&barrier);
        let expected = body.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let req = Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(())
                .expect("req");
            let response = serve_http3_request(ctx.as_ref().clone(), req).await;
            assert_eq!(response.status(), StatusCode::OK);
            let got = response.into_body().collect().await.expect("body");
            assert_eq!(got.to_bytes().as_ref(), expected.as_slice());
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }

    assert_eq!(hits.load(Ordering::SeqCst), 32);
    assert_eq!(cache_proxy_insertions_total(), 0);
    assert!(!exyonq_core::GLOBAL_SINGLEFLIGHT().flights_lock_held());
}
