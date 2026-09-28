//! Plan 12 tranche 5 — bounded FastCGI microcache.

mod fcgi_cache_hooks;
use exyonq_cache::reset_metrics_for_tests;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{
    plan10b_cache_headers_enabled, AppConfig, FcgiBackendExecutor, FcgiDispatchOutcome,
    FcgiDispatchTestGuard, FcgiRuntimeRegistration, FcgiSuccessResponse,
};
use exyonq_mod_fastcgi::{
    cache_fcgi_hits_total, cache_fcgi_insertions_total, cache_fcgi_misses_total,
    reset_fcgi_cache_metrics_for_tests, FcgiRuntime,
};
use exyonq_module_api::fcgi_dispatch::FcgiDispatchRequest;
use exyonq_module_api::fcgi_script_resolver::FastcgiScriptResolverTestGuard;
use fcgi_cache_hooks::ensure_fcgi_cache_hooks;
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Global response cache + fcgi metrics are process-wide; serialize this binary under parallel `cargo test`.
static PLAN12_FCGI_CACHE_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn reset_cache_metrics_for_tests() {
    exyonq_cache::reset_global_for_tests();
    reset_metrics_for_tests();
    reset_fcgi_cache_metrics_for_tests();
}

struct FcgiHarness {
    ctx: Arc<ConnectionContext>,
    _root: tempfile::TempDir,
    _fcgi_guard: FcgiDispatchTestGuard,
    _script_resolver: FastcgiScriptResolverTestGuard,
}

struct CountingExecutor {
    hits: Arc<AtomicUsize>,
    response: FcgiDispatchOutcome,
}

impl FcgiBackendExecutor for CountingExecutor {
    fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.response.clone()
    }
}

fn success_body(body: &'static [u8]) -> FcgiDispatchOutcome {
    FcgiDispatchOutcome::Success(FcgiSuccessResponse {
        status: 200,
        headers: vec![("content-type".into(), "text/plain".into())],
        body: body.to_vec(),
    })
}

async fn fcgi_cache_ctx(
    hits: Arc<AtomicUsize>,
    body: &'static [u8],
    with_cache: bool,
) -> FcgiHarness {
    ensure_fcgi_cache_hooks();
    reset_cache_metrics_for_tests();
    let script_resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("public")).expect("mkdir public");
    std::fs::write(tmp.path().join("public/index.php"), "<?php\n").expect("index.php");
    std::fs::write(tmp.path().join("public/page.php"), "<?php\n").expect("page.php");
    std::fs::write(tmp.path().join("public/setcookie.php"), "<?php\n").expect("setcookie.php");

    let cache_block = if with_cache {
        r#"
[[cache_policy]]
name = "public-php"
ttl_seconds = 30
max_object_bytes = 1048576
"#
    } else {
        ""
    };
    let cache_route = if with_cache {
        r#"
cache = "public-php"
"#
    } else {
        ""
    };

    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
{cache_block}
[[route]]
name = "php"
match = {{ path = "/public/" }}
fastcgi = "php"
{cache_route}
[[fcgi_pool]]
name = "php"
address = "/tmp/exyonq-fcgi.sock"
document_root = "{root}"
"#,
        root = tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    // Cap048: External CountingExecutor after Module bind from publish.
    let fcgi_guard = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(CountingExecutor {
                hits: Arc::clone(&hits),
                response: success_body(body),
            }),
            pool_capacities: vec![(0, 16)],
        })
        .expect("fcgi runtime"),
    ));
    FcgiHarness {
        ctx: Arc::new(ConnectionContext {
            state,
            proxy_client: proxy_client.clone(),
            x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        }),
        _root: tmp,
        _fcgi_guard: fcgi_guard,
        _script_resolver: script_resolver,
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

async fn head_status(ctx: &ConnectionContext, uri: &str) -> StatusCode {
    let req = Request::builder()
        .method(Method::HEAD)
        .uri(uri)
        .body(())
        .expect("req");
    serve_http3_request(ctx.clone(), req).await.status()
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_get_second_request_is_cache_hit() {
    let _gate = PLAN12_FCGI_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let hits = Arc::new(AtomicUsize::new(0));
    let harness = fcgi_cache_ctx(Arc::clone(&hits), b"fcgi-cache-body", true).await;
    let uri = "http://127.0.0.1/public/index.php";
    let before_miss = cache_fcgi_misses_total();
    let before_hit = cache_fcgi_hits_total();

    for _ in 0..2 {
        let (status, body) = get_status_body(harness.ctx.as_ref(), uri).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"fcgi-cache-body");
    }

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(cache_fcgi_misses_total(), before_miss + 1);
    assert_eq!(cache_fcgi_hits_total(), before_hit + 1);
    assert_eq!(cache_fcgi_insertions_total(), 1);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_head_after_get_is_hit_with_empty_body() {
    let _gate = PLAN12_FCGI_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let hits = Arc::new(AtomicUsize::new(0));
    let harness = fcgi_cache_ctx(Arc::clone(&hits), b"fcgi-head-body", true).await;
    let uri = "http://127.0.0.1/public/page.php";
    let (status, body) = get_status_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"fcgi-head-body");
    assert_eq!(head_status(harness.ctx.as_ref(), uri).await, StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_cookie_request_bypasses_cache() {
    let _gate = PLAN12_FCGI_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let hits = Arc::new(AtomicUsize::new(0));
    let harness = fcgi_cache_ctx(Arc::clone(&hits), b"fcgi-cookie", true).await;
    let uri = "http://127.0.0.1/public/index.php";
    let _ = get_status_body(harness.ctx.as_ref(), uri).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("cookie", "session=abc")
        .body(())
        .expect("req");
    let _ = serve_http3_request(harness.ctx.as_ref().clone(), req).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_set_cookie_response_not_stored() {
    ensure_fcgi_cache_hooks();
    let _gate = PLAN12_FCGI_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_cache_metrics_for_tests();
    let hits = Arc::new(AtomicUsize::new(0));
    let _script_resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("public")).expect("mkdir");
    std::fs::write(tmp.path().join("public/setcookie.php"), "<?php\n").expect("php");

    let raw = format!(
        r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[cache_policy]]
name = "public-php"
ttl_seconds = 30
max_object_bytes = 1048576
[[route]]
name = "php"
match = {{ path = "/public/" }}
fastcgi = "php"
cache = "public-php"
[[fcgi_pool]]
name = "php"
address = "/tmp/exyonq-fcgi.sock"
document_root = "{}"
"#,
        tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    let _fcgi_guard = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(CountingExecutor {
                hits: Arc::clone(&hits),
                response: FcgiDispatchOutcome::Success(FcgiSuccessResponse {
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/plain".into()),
                        ("set-cookie".into(), "x=1".into()),
                    ],
                    body: b"setcookie".to_vec(),
                }),
            }),
            pool_capacities: vec![(0, 16)],
        })
        .expect("fcgi runtime"),
    ));
    let ctx = Arc::new(ConnectionContext {
        state,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    });

    let uri = "http://127.0.0.1/public/setcookie.php";
    let _ = get_status_body(ctx.as_ref(), uri).await;
    let _ = get_status_body(ctx.as_ref(), uri).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_without_cache_policy_no_regression() {
    let _gate = PLAN12_FCGI_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let hits = Arc::new(AtomicUsize::new(0));
    let harness = fcgi_cache_ctx(Arc::clone(&hits), b"no-cache-policy", false).await;
    let uri = "http://127.0.0.1/public/index.php";
    for _ in 0..2 {
        let (status, body) = get_status_body(harness.ctx.as_ref(), uri).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"no-cache-policy");
    }
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(cache_fcgi_insertions_total(), 0);
}

#[test]
fn plan10b_cache_headers_flag_defaults_off() {
    std::env::remove_var("EXYONQ_BENCH_CACHE_HEADERS");
    assert!(!plan10b_cache_headers_enabled());
}

#[test]
fn plan10b_cache_headers_retired_always_disabled() {
    // Integrity phase 1: env-gated header injection removed; flag is permanently off.
    assert!(!plan10b_cache_headers_enabled());
}
