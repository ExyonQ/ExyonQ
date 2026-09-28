//! Plan 05B — runtime overlay redirect integration check (no `.htaccess` I/O on request path).

mod htaccess_runtime_support;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::AppConfig;
use exyonq_mod_htaccess::{compile_vhost_overlay, OverlayPublisher};
use hyper::header::HeaderValue;
use hyper::{Method, Request, StatusCode};
use std::sync::Arc;

struct HtaccessRuntimeHarness {
    ctx: ConnectionContext,
    _root: tempfile::TempDir,
    _htaccess_install: htaccess_runtime_support::HtaccessRuntimeInstall,
}

async fn htaccess_ctx() -> HtaccessRuntimeHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".htaccess"), "Redirect 301 /old /new\n").expect("write");
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
htaccess = "overlay"
"#,
        tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let compiled = compile_vhost_overlay("site", tmp.path(), 1).expect("compile");
    let publisher = Arc::new(OverlayPublisher::new(1));
    publisher
        .publish(exyonq_module_api::RuntimePatchVhostOverlay {
            site_id: "site".into(),
            plan_generation: 1,
            overlay: compiled.overlay,
        })
        .expect("publish");
    let getter_publisher: Arc<OverlayPublisher> = Arc::clone(&publisher);
    let _htaccess_install =
        htaccess_runtime_support::install_htaccess_runtime_publisher(getter_publisher);

    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    HtaccessRuntimeHarness {
        ctx: ConnectionContext {
            state,
            proxy_client: proxy_client.clone(),
            x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        _root: tmp,
        _htaccess_install,
    }
}

#[tokio::test]
async fn overlay_redirect_301_on_request_path() {
    let harness = htaccess_ctx().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/old")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok()),
        Some("/new")
    );
}

#[test]
fn without_overlay_publisher_miss() {
    let publisher = Arc::new(OverlayPublisher::new(1));
    assert!(publisher.get("missing").is_none());
}

#[test]
fn htaccess_redirect_plus_overlay_rejected_in_ir() {
    let raw = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
redirect = { status = 301, location = "/x" }
htaccess = "overlay"
"#;
    let err = raw.parse::<AppConfig>().expect_err("must fail");
    assert!(err.to_string().contains("htaccess"));
}

#[test]
fn htaccess_fastcgi_overlay_valid_in_ir() {
    let raw = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"
htaccess = "overlay"
[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
document_root = "/srv/php"
"#;
    raw.parse::<AppConfig>().expect("fcgi overlay valid");
}
