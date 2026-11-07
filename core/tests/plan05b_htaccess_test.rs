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
//! Plan 05B — runtime overlay redirect integration check (no `.htaccess` I/O on request path).

mod htaccess_runtime_support;
use base64::Engine as _;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{AppConfig, StaticDispatchTestGuard};
use exyonq_mod_htaccess::{compile_vhost_overlay, OverlayPublisher};
use exyonq_mod_static::StaticRuntime;
use exyonq_module_api::static_dispatch::StaticDispatchService;
use hyper::header::HeaderValue;
use hyper::{Method, Request, StatusCode};
use std::sync::Arc;

struct HtaccessRuntimeHarness {
    ctx: ConnectionContext,
    _root: tempfile::TempDir,
    _static_guard: StaticDispatchTestGuard,
    _htaccess_install: htaccess_runtime_support::HtaccessRuntimeInstall,
}

fn install_static_runtime_for_tests() -> StaticDispatchTestGuard {
    let runtime = Arc::new(StaticRuntime::new());
    let service: Arc<dyn StaticDispatchService> = runtime.clone();
    let guard = StaticDispatchTestGuard::install(service);
    exyonq_mod_static::install_kernel_hooks(runtime);
    guard
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

    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    HtaccessRuntimeHarness {
        ctx: ConnectionContext {
            scheme: "http",
            local_port: None,
            state,
            proxy_client,
            x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        _root: tmp,
        _static_guard,
        _htaccess_install,
    }
}

async fn htaccess_auth_ctx() -> HtaccessRuntimeHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    let htpasswd = tmp.path().join("users.htpasswd");
    std::fs::write(&htpasswd, "u:{PLAIN}pw\n").expect("write htpasswd");
    std::fs::write(tmp.path().join("index.html"), "secret-body").expect("write secret");
    std::fs::write(
        tmp.path().join(".htaccess"),
        format!(
            "AuthType Basic\nAuthName \"Restricted\"\nAuthUserFile {}\nRequire valid-user\nDirectoryIndex index.html\n",
            htpasswd.display()
        ),
    )
    .expect("write htaccess");
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

    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    HtaccessRuntimeHarness {
        ctx: ConnectionContext {
            scheme: "http",
            local_port: None,
            state,
            proxy_client,
            x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        _root: tmp,
        _static_guard,
        _htaccess_install,
    }
}

async fn route_access_ctx(xff: &'static str) -> HtaccessRuntimeHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("index.html"), "route-access-ok").expect("write index");
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
index = "index.html"
access = {{ allow = ["10.0.0.0/8"], methods = ["GET"] }}
"#,
        tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("route access config");
    let publisher = Arc::new(OverlayPublisher::new(1));
    let _htaccess_install =
        htaccess_runtime_support::install_htaccess_runtime_publisher(Arc::clone(&publisher));
    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    HtaccessRuntimeHarness {
        ctx: ConnectionContext {
            scheme: "http",
            local_port: None,
            state,
            proxy_client,
            x_forwarded_for: HeaderValue::from_static(xff),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        _root: tmp,
        _static_guard,
        _htaccess_install,
    }
}

async fn deny_all_ctx() -> HtaccessRuntimeHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    let raw = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["wp-config-deny"]

[[route]]
name = "wp-config-deny"
match = { path = "/wp-config.php" }
access = { deny = ["all"] }
"#;
    let config: AppConfig = raw.parse().expect("deny-all route config");
    let publisher = Arc::new(OverlayPublisher::new(1));
    let _htaccess_install = htaccess_runtime_support::install_htaccess_runtime_publisher(publisher);
    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    HtaccessRuntimeHarness {
        ctx: ConnectionContext {
            scheme: "http",
            local_port: None,
            state,
            proxy_client,
            x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        _root: tmp,
        _static_guard,
        _htaccess_install,
    }
}

async fn htaccess_access_ctx(xff: &'static str) -> HtaccessRuntimeHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("index.html"), "htaccess-access-ok").expect("write index");
    std::fs::write(tmp.path().join(".htaccess"), "Require ip 10.0.0.0/8\n")
        .expect("write htaccess");
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
index = "index.html"
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
    let _htaccess_install =
        htaccess_runtime_support::install_htaccess_runtime_publisher(Arc::clone(&publisher));
    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    HtaccessRuntimeHarness {
        ctx: ConnectionContext {
            scheme: "http",
            local_port: None,
            state,
            proxy_client,
            x_forwarded_for: HeaderValue::from_static(xff),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        _root: tmp,
        _static_guard,
        _htaccess_install,
    }
}

fn basic_auth(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
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

#[tokio::test]
async fn overlay_basic_auth_challenges_and_accepts_credentials() {
    let harness = htaccess_auth_ctx().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok()),
        Some("Basic realm=\"Restricted\"")
    );

    let harness = htaccess_auth_ctx().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .header("authorization", basic_auth("u", "wrong"))
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let harness = htaccess_auth_ctx().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .header("authorization", basic_auth("u", "pw"))
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn route_access_denies_ip_and_method_before_static() {
    let harness = route_access_ctx("127.0.0.1").await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    assert_eq!(
        serve_http3_request(harness.ctx, req).await.status(),
        StatusCode::FORBIDDEN
    );

    let harness = route_access_ctx("10.1.2.3").await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    assert_eq!(
        serve_http3_request(harness.ctx, req).await.status(),
        StatusCode::FORBIDDEN
    );

    let harness = route_access_ctx("10.1.2.3").await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    assert_eq!(
        serve_http3_request(harness.ctx, req).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn deny_all_route_rejects_path_boundary_without_backend() {
    for path in [
        "/wp-config.php",
        "/wp-config.php?probe=1",
        "/wp-config.php/",
        "/wp-config.php/nested",
    ] {
        let harness = deny_all_ctx().await;
        let req = Request::builder()
            .method(Method::GET)
            .uri(format!("http://127.0.0.1{path}"))
            .body(())
            .expect("req");
        assert_eq!(
            serve_http3_request(harness.ctx, req).await.status(),
            StatusCode::FORBIDDEN,
            "boundary deny route must reject {path}"
        );
    }

    // A segmented prefix must not become a raw string prefix.
    for path in ["/wp-config.php.bak", "/wp-config.phpx"] {
        let harness = deny_all_ctx().await;
        let req = Request::builder()
            .method(Method::GET)
            .uri(format!("http://127.0.0.1{path}"))
            .body(())
            .expect("req");
        assert_eq!(
            serve_http3_request(harness.ctx, req).await.status(),
            StatusCode::NOT_FOUND,
            "boundary deny route must not match {path}"
        );
    }
}

#[tokio::test]
async fn htaccess_require_ip_is_enforced_before_static() {
    let harness = htaccess_access_ctx("127.0.0.1").await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    assert_eq!(
        serve_http3_request(harness.ctx, req).await.status(),
        StatusCode::FORBIDDEN
    );

    let harness = htaccess_access_ctx("10.1.2.3").await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    assert_eq!(
        serve_http3_request(harness.ctx, req).await.status(),
        StatusCode::OK
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
