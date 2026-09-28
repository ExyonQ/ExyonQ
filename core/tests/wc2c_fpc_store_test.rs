//! WC2C — FPC MISS → static origin → assess → L1 insert → HIT vertical.

use exyonq_cache::{
    fpc_hit_total, fpc_miss_total, fpc_store_attempt_total, fpc_store_rejected_total,
    fpc_store_success_total, reset_metrics_for_tests,
};
use exyonq_core::fpc_lookup::{fpc_gate, FpcGateResult};
use exyonq_core::fpc_store::try_fpc_store_response;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{AppConfig, StaticDispatchTestGuard};
use exyonq_mod_static::StaticRuntime;
use exyonq_module_api::static_dispatch::StaticDispatchService;
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode};
use std::sync::Arc;

static WC2C_FPC_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn install_static_runtime_for_tests() -> StaticDispatchTestGuard {
    let runtime = Arc::new(StaticRuntime::new());
    let service: Arc<dyn StaticDispatchService> = runtime.clone();
    let guard = StaticDispatchTestGuard::install(service);
    exyonq_mod_static::install_kernel_hooks(runtime);
    guard
}

struct FpcCtx {
    ctx: Arc<ConnectionContext>,
    _tmp: tempfile::TempDir,
    _static_guard: StaticDispatchTestGuard,
}

async fn fpc_static_ctx(max_object_bytes: usize) -> FpcCtx {
    reset_metrics_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("ok.txt"), b"fpc-body-v1").expect("write");
    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = "{}"

[full_page_cache]
enabled = true
namespace = 4
max_entries = 32
max_total_bytes = 1048576
max_object_bytes = {max_object_bytes}
default_ttl_seconds = 60
max_ttl_seconds = 300
"#,
        tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(11, config, proxy_client.clone())
        .await
        .expect("state");
    FpcCtx {
        ctx: Arc::new(ConnectionContext {
            state,
            proxy_client: proxy_client.clone(),
            x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        }),
        _tmp: tmp,
        _static_guard,
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn miss_fill_hit_vertical_static() {
    let _gate = WC2C_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_static_ctx(1024 * 1024).await;
    let ctx = harness.ctx;
    let uri = "http://example.test/ok.txt";
    let before_miss = fpc_miss_total();
    let before_hit = fpc_hit_total();
    let before_store = fpc_store_success_total();

    let req1 = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    let r1 = serve_http3_request(ctx.as_ref().clone(), req1).await;
    assert_eq!(r1.status(), StatusCode::OK);
    let body1 = r1.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body1[..], b"fpc-body-v1");
    assert!(fpc_miss_total() > before_miss);
    assert!(fpc_store_success_total() > before_store);

    let req2 = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    let r2 = serve_http3_request(ctx.as_ref().clone(), req2).await;
    assert_eq!(r2.status(), StatusCode::OK);
    let body2 = r2.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body2[..], b"fpc-body-v1");
    assert!(fpc_hit_total() > before_hit);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn head_reuses_get_entry_without_poisoning() {
    let _gate = WC2C_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_static_ctx(1024 * 1024).await;
    let ctx = harness.ctx;
    let uri = "http://example.test/ok.txt";

    // HEAD miss first must not store empty body over GET representation.
    let head_first = Request::builder()
        .method(Method::HEAD)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    let hr = serve_http3_request(ctx.as_ref().clone(), head_first).await;
    assert_eq!(hr.status(), StatusCode::OK);
    let hb = hr.into_body().collect().await.unwrap().to_bytes();
    assert!(hb.is_empty());
    let stores_after_head = fpc_store_success_total();

    let get = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    let gr = serve_http3_request(ctx.as_ref().clone(), get).await;
    assert_eq!(gr.status(), StatusCode::OK);
    assert!(fpc_store_success_total() > stores_after_head);

    let head2 = Request::builder()
        .method(Method::HEAD)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    let hr2 = serve_http3_request(ctx.as_ref().clone(), head2).await;
    assert_eq!(hr2.status(), StatusCode::OK);
    let hb2 = hr2.into_body().collect().await.unwrap().to_bytes();
    assert!(hb2.is_empty());

    let get2 = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    let gr2 = serve_http3_request(ctx.as_ref().clone(), get2).await;
    let body = gr2.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"fpc-body-v1");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn private_cookie_bypasses_and_does_not_hit() {
    let _gate = WC2C_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_static_ctx(1024 * 1024).await;
    let ctx = harness.ctx;
    let uri = "http://example.test/ok.txt";

    let warm = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    serve_http3_request(ctx.as_ref().clone(), warm).await;

    let before_hit = fpc_hit_total();
    let private = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("host", "example.test")
        .header("cookie", "wordpress_logged_in_abc=1")
        .body(())
        .expect("req");
    let r = serve_http3_request(ctx.as_ref().clone(), private).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(fpc_hit_total(), before_hit);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn body_too_large_fail_open_no_store() {
    let _gate = WC2C_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_static_ctx(4).await; // body is longer than 4 bytes
    let ctx = harness.ctx;
    let uri = "http://example.test/ok.txt";
    let before_reject = fpc_store_rejected_total("body_too_large");
    let before_success = fpc_store_success_total();

    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    let r = serve_http3_request(ctx.as_ref().clone(), req).await;
    assert_eq!(r.status(), StatusCode::OK);
    let body = r.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"fpc-body-v1");
    assert!(fpc_store_attempt_total() >= 1);
    assert!(fpc_store_rejected_total("body_too_large") > before_reject);
    assert_eq!(fpc_store_success_total(), before_success);

    // Second request still MISS (nothing stored).
    let before_hit = fpc_hit_total();
    let req2 = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    serve_http3_request(ctx.as_ref().clone(), req2).await;
    assert_eq!(fpc_hit_total(), before_hit);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn set_cookie_response_never_stored() {
    let _gate = WC2C_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let harness = fpc_static_ctx(1024 * 1024).await;
    let state = &harness.ctx.state;
    let uri: hyper::Uri = "http://example.test/ok.txt".parse().unwrap();
    let gate = fpc_gate(state, 0, &Method::GET, &uri, Some("example.test"), &[]);
    let FpcGateResult::Miss(ctx) = gate else {
        panic!("expected miss");
    };
    let before = fpc_store_rejected_total("set_cookie");
    try_fpc_store_response(
        &ctx,
        "GET",
        200,
        &[
            ("content-type".into(), "text/plain".into()),
            ("set-cookie".into(), "session=1".into()),
        ],
        bytes::Bytes::from_static(b"nope"),
    );
    assert!(fpc_store_rejected_total("set_cookie") > before);
    assert_eq!(fpc_store_success_total(), 0);
    assert!(matches!(
        fpc_gate(state, 0, &Method::GET, &uri, Some("example.test"), &[]),
        FpcGateResult::Miss(_)
    ));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn parallel_miss_inserts_remain_bounded() {
    let _gate = WC2C_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_static_ctx(1024 * 1024).await;
    let ctx = harness.ctx;
    let mut handles = Vec::new();
    for _ in 0..8 {
        let c = ctx.clone();
        handles.push(tokio::spawn(async move {
            let req = Request::builder()
                .method(Method::GET)
                .uri("http://example.test/ok.txt")
                .header("host", "example.test")
                .body(())
                .expect("req");
            serve_http3_request(c.as_ref().clone(), req).await
        }));
    }
    for h in handles {
        let r = h.await.expect("join");
        assert_eq!(r.status(), StatusCode::OK);
    }
    let snap = ctx.state.fpc_cache.as_ref().expect("cache").snapshot();
    assert!(snap.entries <= 32);
    assert!(snap.bytes <= 1_048_576);
}
