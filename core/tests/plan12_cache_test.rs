//! Plan 12 v0 — response cache (static GET/HEAD).
use exyonq_cache::{
    cache_hits_total, cache_insertions_total, cache_misses_total, reset_metrics_for_tests,
};
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{AppConfig, StaticDispatchTestGuard};
use exyonq_mod_static::StaticRuntime;
use exyonq_module_api::assess_cacheability;
use exyonq_module_api::static_dispatch::StaticDispatchService;
use http_body_util::BodyExt;
use hyper::header::{AUTHORIZATION, COOKIE, RANGE};
use hyper::{Method, Request, StatusCode};
use std::sync::Arc;

/// `ServerState` uses process-wide response cache; serialize this binary under parallel `cargo test`.
static PLAN12_GLOBAL_CACHE_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn install_static_runtime_for_tests() -> StaticDispatchTestGuard {
    let runtime = Arc::new(StaticRuntime::new());
    let service: Arc<dyn StaticDispatchService> = runtime.clone();
    let guard = StaticDispatchTestGuard::install(service);
    exyonq_mod_static::install_kernel_hooks(runtime);
    guard
}

struct CachedCtx {
    ctx: Arc<ConnectionContext>,
    _tmp: tempfile::TempDir,
    _static_guard: StaticDispatchTestGuard,
}

async fn cached_static_ctx(ttl_seconds: u64) -> CachedCtx {
    reset_metrics_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("ok.txt"), b"cached-body").expect("write");
    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["assets"]

[[cache_policy]]
name = "default"
ttl_seconds = {ttl_seconds}
max_object_bytes = 1048576

[[route]]
name = "assets"
match = {{ path = "/" }}
root = "{}"
cache = "default"
"#,
        tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    CachedCtx {
        ctx: Arc::new(ConnectionContext {
            state,
            proxy_client,
            x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        }),
        _tmp: tmp,
        _static_guard,
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn static_get_200_second_request_is_cache_hit() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(30).await;
    let ctx = harness.ctx;
    let uri = "http://127.0.0.1/ok.txt";
    let before_miss = cache_misses_total();
    let before_hit = cache_hits_total();

    for _ in 0..2 {
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(())
            .expect("req");
        let response = serve_http3_request(ctx.as_ref().clone(), req).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    assert_eq!(cache_misses_total(), before_miss + 1);
    assert_eq!(cache_hits_total(), before_hit + 1);
    assert_eq!(cache_insertions_total(), 1);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn head_cached_without_body() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(30).await;
    let ctx = harness.ctx;
    let req = Request::builder()
        .method(Method::HEAD)
        .uri("http://127.0.0.1/ok.txt")
        .body(())
        .expect("req");
    let first = serve_http3_request(ctx.as_ref().clone(), req).await;
    assert_eq!(first.status(), StatusCode::OK);
    let req2 = Request::builder()
        .method(Method::HEAD)
        .uri("http://127.0.0.1/ok.txt")
        .body(())
        .expect("req2");
    let second = serve_http3_request(ctx.as_ref().clone(), req2).await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(second.headers().get("content-length").unwrap(), "11");
    let body = second.into_body().collect().await.expect("collect");
    assert!(body.to_bytes().is_empty());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn distinct_query_strings_do_not_share_cache() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(30).await;
    let ctx = harness.ctx;
    let req_a = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/ok.txt?v=1")
        .body(())
        .expect("req");
    let req_b = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/ok.txt?v=2")
        .body(())
        .expect("req");
    serve_http3_request(ctx.as_ref().clone(), req_a).await;
    serve_http3_request(ctx.as_ref().clone(), req_b).await;
    assert_eq!(cache_insertions_total(), 2);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn new_runtime_generation_forces_miss() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(30).await;
    let ctx = harness.ctx;
    let _tmp = harness._tmp;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/ok.txt")
        .body(())
        .expect("req");
    serve_http3_request(ctx.as_ref().clone(), req).await;

    let config = ctx.state.config.clone();
    let proxy_client = ctx.proxy_client.clone();
    let state2 = ServerState::new_with_generation(2, config, proxy_client)
        .await
        .expect("state2");
    let ctx2 = ConnectionContext {
        state: state2,
        proxy_client: ctx.proxy_client.clone(),
        x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };
    let before_miss = cache_misses_total();
    let req2 = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/ok.txt")
        .body(())
        .expect("req2");
    serve_http3_request(ctx2, req2).await;
    assert!(cache_misses_total() > before_miss);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn route_without_cache_policy_always_misses_metrics() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("plain.txt"), b"x").expect("write");
    let raw = format!(
        r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["plain"]
[[route]]
name = "plain"
match = {{ path = "/plain" }}
root = "{}"
"#,
        tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    let ctx = ConnectionContext {
        state,
        proxy_client,
        x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };
    let before_hit = cache_hits_total();
    for _ in 0..2 {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://127.0.0.1/plain/plain.txt")
            .body(())
            .expect("req");
        serve_http3_request(ctx.clone(), req).await;
    }
    assert_eq!(cache_hits_total(), before_hit);
    let _ = _static_guard;
}

#[test]
fn cacheability_rules_fail_closed() {
    use exyonq_module_api::CacheRejection;

    assert_eq!(
        assess_cacheability("POST", &[], 200, &[], 0, 1024).unwrap_err(),
        CacheRejection::Method
    );
    assert_eq!(
        assess_cacheability("GET", &[], 404, &[], 0, 1024).unwrap_err(),
        CacheRejection::Status
    );
    assert_eq!(
        assess_cacheability(
            "GET",
            &[(AUTHORIZATION.as_str().to_string(), "x".into())],
            200,
            &[],
            0,
            1024
        )
        .unwrap_err(),
        CacheRejection::RequestAuthorization
    );
    assert_eq!(
        assess_cacheability(
            "GET",
            &[(COOKIE.as_str().to_string(), "s=1".into())],
            200,
            &[],
            0,
            1024
        )
        .unwrap_err(),
        CacheRejection::RequestCookie
    );
    assert_eq!(
        assess_cacheability(
            "GET",
            &[(RANGE.as_str().to_string(), "bytes=0-1".into())],
            200,
            &[],
            0,
            1024
        )
        .unwrap_err(),
        CacheRejection::RequestRange
    );
    assert_eq!(
        assess_cacheability(
            "GET",
            &[],
            200,
            &[("set-cookie".into(), "a=b".into())],
            0,
            1024
        )
        .unwrap_err(),
        CacheRejection::ResponseSetCookie
    );
    assert_eq!(
        assess_cacheability(
            "GET",
            &[],
            200,
            &[("cache-control".into(), "no-store".into())],
            0,
            1024
        )
        .unwrap_err(),
        CacheRejection::ResponseCacheControl
    );
    assert_eq!(
        assess_cacheability("GET", &[], 200, &[], 2048, 1024).unwrap_err(),
        CacheRejection::BodyTooLarge
    );
}
