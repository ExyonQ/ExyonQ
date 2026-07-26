//! Plan 11 tranche C — compiled front-controller rewrite (end-to-end).

mod htaccess_runtime_support;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{
    clear_global_fcgi_dispatch_for_register_once_test, contract_service_registration_test_gate,
    fcgi_metrics_assert_guard, register_fcgi_dispatch_service, AppConfig, FcgiDispatchTestGuard,
    FcgiRuntimeRegistration, StaticDispatchTestGuard, DEFAULT_FCGI_MAX_CONCURRENCY,
};
use exyonq_mod_fastcgi::{
    fcgi_responses_502_total, fcgi_responses_503_total, fcgi_responses_504_total, FcgiRuntime,
};
use exyonq_mod_htaccess::{
    compile_vhost_overlay, htaccess_front_controller_bypass_file_total, OverlayPublisher,
};
use exyonq_mod_static::StaticRuntime;
use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchRequest, FcgiSuccessResponse,
};
use exyonq_module_api::static_dispatch::StaticDispatchService;
use http_body_util::BodyExt;
use hyper::header::HeaderValue;
use hyper::{Method, Request, StatusCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const FRONT_CONTROLLER_HTACCESS: &str = r"
RewriteEngine On
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
";

const WORDPRESS_HTACCESS: &str = r"
DirectoryIndex index.php index.html

<IfModule mod_rewrite.c>
RewriteEngine On
RewriteBase /
RewriteRule ^index\.php$ - [L]
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
</IfModule>
";

static FCGI_CAPTURE: Mutex<Option<(String, String)>> = Mutex::new(None);
static HTACCESS_OVERLAY_TEST_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static FRONT_CONTROLLER_E2E_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct CaptureRequestExecutor;
impl FcgiBackendExecutor for CaptureRequestExecutor {
    fn dispatch(&self, request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        *FCGI_CAPTURE.lock().expect("lock") =
            Some((request.script_filename.clone(), request.request_uri.clone()));
        FcgiDispatchOutcome::Success(FcgiSuccessResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"fcgi-ok".to_vec(),
        })
    }
}

struct FcgiHarness {
    ctx: ConnectionContext,
    _root: tempfile::TempDir,
    _static_guard: StaticDispatchTestGuard,
    _fcgi_guard: FcgiDispatchTestGuard,
    _htaccess_install: htaccess_runtime_support::HtaccessRuntimeInstall,
    _overlay_gate: tokio::sync::MutexGuard<'static, ()>,
}

fn install_static_runtime_for_tests() -> StaticDispatchTestGuard {
    let runtime = Arc::new(StaticRuntime::new());
    let service: Arc<dyn StaticDispatchService> = runtime.clone();
    let guard = StaticDispatchTestGuard::install(service);
    exyonq_mod_static::install_kernel_hooks(runtime);
    guard
}

fn install_fcgi_runtime_for_tests(
    executor: Arc<dyn FcgiBackendExecutor>,
    pool_capacities: Vec<(u32, usize)>,
) -> FcgiDispatchTestGuard {
    FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor,
            pool_capacities,
        })
        .expect("fcgi runtime"),
    ))
}

async fn front_controller_ctx(
    htaccess_body: &str,
    seed: impl FnOnce(&std::path::Path),
    executor: Arc<dyn FcgiBackendExecutor>,
    pool_capacities: Vec<(u32, usize)>,
) -> FcgiHarness {
    let _overlay_gate = HTACCESS_OVERLAY_TEST_GATE.lock().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    seed(tmp.path());
    std::fs::write(tmp.path().join(".htaccess"), htaccess_body).expect("htaccess");

    let raw = format!(
        r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[route]]
name = "php"
match = {{ path = "/" }}
fastcgi = "php"
htaccess = "overlay"
[[fcgi_pool]]
name = "php"
address = "unix:/tmp/php.sock"
document_root = "{}"
"#,
        tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let compiled = compile_vhost_overlay("php", tmp.path(), 1).expect("compile");
    let publisher = Arc::new(OverlayPublisher::new(1));
    publisher
        .publish(exyonq_module_api::RuntimePatchVhostOverlay {
            site_id: "php".into(),
            plan_generation: 1,
            overlay: compiled.overlay,
        })
        .expect("publish");
    let getter = Arc::clone(&publisher);
    let _htaccess_install =
        htaccess_runtime_support::install_htaccess_runtime_publisher(Arc::clone(&getter));

    let _static_guard = install_static_runtime_for_tests();
    let _fcgi_guard = install_fcgi_runtime_for_tests(executor, pool_capacities);

    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    FcgiHarness {
        ctx: ConnectionContext {
            state,
            proxy_client,
            x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        _root: tmp,
        _static_guard,
        _fcgi_guard,
        _htaccess_install,
        _overlay_gate,
    }
}

fn seed_fixture(root: &std::path::Path) {
    std::fs::write(root.join("index.php"), b"<?php").expect("php");
    std::fs::write(root.join("existing.txt"), b"plain").expect("txt");
    std::fs::create_dir(root.join("assets")).expect("mkdir");
    std::fs::write(root.join("assets/index.html"), b"asset-index").expect("html");
}

#[tokio::test]
async fn directory_index_root_with_front_controller_serves_index_php() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    *FCGI_CAPTURE.lock().expect("lock") = None;
    let harness = front_controller_ctx(
        WORDPRESS_HTACCESS,
        seed_fixture,
        Arc::new(CaptureRequestExecutor),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
    )
    .await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);

    let (script, request_uri) = FCGI_CAPTURE
        .lock()
        .expect("lock")
        .clone()
        .expect("captured");
    assert!(script.ends_with("index.php"));
    assert_eq!(request_uri, "/");
}

#[tokio::test]
async fn missing_resource_rewrites_to_index_php_preserving_request_uri() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    *FCGI_CAPTURE.lock().expect("lock") = None;
    let harness = front_controller_ctx(
        FRONT_CONTROLLER_HTACCESS,
        seed_fixture,
        Arc::new(CaptureRequestExecutor),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
    )
    .await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/posts/hello?page=2")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);

    let (script, request_uri) = FCGI_CAPTURE
        .lock()
        .expect("lock")
        .clone()
        .expect("captured");
    assert!(script.ends_with("index.php"));
    assert_eq!(request_uri, "/posts/hello?page=2");
}

#[tokio::test]
async fn virtual_permalink_trailing_slash_rewrites_to_index_php() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    *FCGI_CAPTURE.lock().expect("lock") = None;
    let harness = front_controller_ctx(
        WORDPRESS_HTACCESS,
        seed_fixture,
        Arc::new(CaptureRequestExecutor),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
    )
    .await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/posts/hello/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);

    let (script, request_uri) = FCGI_CAPTURE
        .lock()
        .expect("lock")
        .clone()
        .expect("captured");
    assert!(script.ends_with("index.php"));
    assert_eq!(request_uri, "/posts/hello/");
}

#[tokio::test]
async fn existing_file_bypasses_front_controller() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    *FCGI_CAPTURE.lock().expect("lock") = None;
    let before = htaccess_front_controller_bypass_file_total();
    let harness = front_controller_ctx(
        FRONT_CONTROLLER_HTACCESS,
        seed_fixture,
        Arc::new(CaptureRequestExecutor),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
    )
    .await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/existing.txt")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(htaccess_front_controller_bypass_file_total(), before + 1);

    let (script, request_uri) = FCGI_CAPTURE
        .lock()
        .expect("lock")
        .clone()
        .expect("captured");
    assert!(script.ends_with("existing.txt"));
    assert_eq!(request_uri, "/existing.txt");
}

#[tokio::test]
async fn existing_directory_uses_directory_index_not_front_controller() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    let htaccess = format!("{FRONT_CONTROLLER_HTACCESS}\nDirectoryIndex index.html\n");
    let harness = front_controller_ctx(
        &htaccess,
        seed_fixture,
        Arc::new(CaptureRequestExecutor),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
    )
    .await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/assets/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(body.as_ref(), b"asset-index");
}

#[tokio::test]
async fn missing_index_php_returns_controlled_failure_without_loop() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    let harness = front_controller_ctx(
        FRONT_CONTROLLER_HTACCESS,
        |root| {
            std::fs::write(root.join("existing.txt"), b"x").expect("write");
        },
        Arc::new(CaptureRequestExecutor),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
    )
    .await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/missing")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn external_redirect_precedes_front_controller() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    let _overlay_gate = HTACCESS_OVERLAY_TEST_GATE.lock().await;
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct NoCall;
    impl FcgiBackendExecutor for NoCall {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            CALLS.fetch_add(1, Ordering::SeqCst);
            FcgiDispatchOutcome::BadGateway
        }
    }
    let _fcgi_guard = install_fcgi_runtime_for_tests(Arc::new(NoCall), vec![(0, 1)]);
    let _static_guard = install_static_runtime_for_tests();

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join(".htaccess"),
        format!("Redirect 301 /legacy/ /elsewhere\n{FRONT_CONTROLLER_HTACCESS}"),
    )
    .expect("write");
    std::fs::write(tmp.path().join("index.php"), b"<?php").expect("php");

    let raw = format!(
        r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[route]]
name = "php"
match = {{ path = "/" }}
fastcgi = "php"
htaccess = "overlay"
[[fcgi_pool]]
name = "php"
address = "unix:/tmp/php.sock"
document_root = "{}"
"#,
        tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let compiled = compile_vhost_overlay("php", tmp.path(), 1).expect("compile");
    let publisher = Arc::new(OverlayPublisher::new(1));
    publisher
        .publish(exyonq_module_api::RuntimePatchVhostOverlay {
            site_id: "php".into(),
            plan_generation: 1,
            overlay: compiled.overlay,
        })
        .expect("publish");
    let getter = Arc::clone(&publisher);
    let _htaccess_install =
        htaccess_runtime_support::install_htaccess_runtime_publisher(Arc::clone(&getter));
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    let ctx = ConnectionContext {
        state,
        proxy_client,
        x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };
    let _root = tmp;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/legacy/")
        .body(())
        .expect("req");
    let response = serve_http3_request(ctx, req).await;
    assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    let _ = (_static_guard, _fcgi_guard, _root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)]
async fn saturation_after_rewrite_returns_503() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    let _metrics_gate = fcgi_metrics_assert_guard().await;
    let _overlay_gate = HTACCESS_OVERLAY_TEST_GATE.lock().await;
    let _gate = contract_service_registration_test_gate();
    clear_global_fcgi_dispatch_for_register_once_test();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    struct Gate {
        started_tx: std::sync::mpsc::Sender<()>,
        release_rx: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl FcgiBackendExecutor for Gate {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            let _ = self.started_tx.send(());
            if let Ok(rx) = self.release_rx.lock() {
                let _ = rx.recv();
            }
            FcgiDispatchOutcome::Success(FcgiSuccessResponse {
                status: 200,
                headers: vec![],
                body: b"ok".to_vec(),
            })
        }
    }
    let gate_executor = Arc::new(Gate {
        started_tx,
        release_rx: std::sync::Mutex::new(release_rx),
    });
    register_fcgi_dispatch_service(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: gate_executor,
            pool_capacities: vec![(0, 1)],
        })
        .expect("fcgi runtime"),
    ))
    .expect("register global fcgi for multi-thread test");

    let tmp = tempfile::tempdir().expect("tempdir");
    seed_fixture(tmp.path());
    std::fs::write(tmp.path().join(".htaccess"), FRONT_CONTROLLER_HTACCESS).expect("htaccess");
    let raw = format!(
        r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[route]]
name = "php"
match = {{ path = "/" }}
fastcgi = "php"
htaccess = "overlay"
[[fcgi_pool]]
name = "php"
address = "unix:/tmp/php.sock"
document_root = "{}"
"#,
        tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let compiled = compile_vhost_overlay("php", tmp.path(), 1).expect("compile");
    let publisher = Arc::new(OverlayPublisher::new(1));
    publisher
        .publish(exyonq_module_api::RuntimePatchVhostOverlay {
            site_id: "php".into(),
            plan_generation: 1,
            overlay: compiled.overlay,
        })
        .expect("publish");
    let getter = Arc::clone(&publisher);
    let _htaccess_install =
        htaccess_runtime_support::install_htaccess_runtime_publisher(Arc::clone(&getter));
    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    let harness_ctx = ConnectionContext {
        state,
        proxy_client,
        x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };
    let _root = tmp;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/posts/a")
        .body(())
        .expect("req");
    let ctx_a = ConnectionContext {
        state: Arc::clone(&harness_ctx.state),
        proxy_client: harness_ctx.proxy_client.clone(),
        x_forwarded_for: harness_ctx.x_forwarded_for.clone(),
        ops: Arc::clone(&harness_ctx.ops),
    };
    let publisher_for_spawn = Arc::clone(&publisher);
    let first = tokio::spawn(async move {
        let _htaccess_install =
            htaccess_runtime_support::install_htaccess_runtime_publisher(publisher_for_spawn);
        serve_http3_request(ctx_a, req).await
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("started");
    let before = fcgi_responses_503_total();
    let req2 = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/posts/b")
        .body(())
        .expect("req");
    let second = serve_http3_request(harness_ctx, req2).await;
    assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(fcgi_responses_503_total(), before + 1);
    let _ = release_tx.send(());
    let _ = first.await;
    let _ = (_static_guard, _root);
}

#[tokio::test]
async fn timeout_after_rewrite_returns_504() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    struct Fail504;
    impl FcgiBackendExecutor for Fail504 {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            FcgiDispatchOutcome::GatewayTimeout
        }
    }
    let before = fcgi_responses_504_total();
    let harness = front_controller_ctx(
        FRONT_CONTROLLER_HTACCESS,
        seed_fixture,
        Arc::new(Fail504),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
    )
    .await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/posts/slow")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(fcgi_responses_504_total(), before + 1);
}

#[tokio::test]
async fn site_a_does_not_share_front_controller_with_site_b() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    let _overlay_gate = HTACCESS_OVERLAY_TEST_GATE.lock().await;
    let dir_a = tempfile::tempdir().expect("a");
    let dir_b = tempfile::tempdir().expect("b");
    for (dir, target) in [
        (dir_a.path(), "/index-a.php"),
        (dir_b.path(), "/index-b.php"),
    ] {
        std::fs::write(
            dir.join(".htaccess"),
            format!(
                "RewriteEngine On\nRewriteCond %{{REQUEST_FILENAME}} !-f\nRewriteCond %{{REQUEST_FILENAME}} !-d\nRewriteRule . {target} [L]\n"
            ),
        )
        .expect("htaccess");
        std::fs::write(dir.join(target.trim_start_matches('/')), b"<?php").expect("php");
    }

    let raw = format!(
        r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site-a", "site-b"]
[[route]]
name = "site-a"
match = {{ path = "/a" }}
fastcgi = "pool-a"
htaccess = "overlay"
[[route]]
name = "site-b"
match = {{ path = "/b" }}
fastcgi = "pool-b"
htaccess = "overlay"
[[fcgi_pool]]
name = "pool-a"
address = "unix:/tmp/a.sock"
document_root = "{}"
[[fcgi_pool]]
name = "pool-b"
address = "unix:/tmp/b.sock"
document_root = "{}"
"#,
        dir_a.path().display(),
        dir_b.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let publisher = Arc::new(OverlayPublisher::new(1));
    for (site, root) in [("site-a", dir_a.path()), ("site-b", dir_b.path())] {
        let compiled = compile_vhost_overlay(site, root, 1).expect("compile");
        publisher
            .publish(exyonq_module_api::RuntimePatchVhostOverlay {
                site_id: site.into(),
                plan_generation: 1,
                overlay: compiled.overlay,
            })
            .expect("publish");
    }
    let getter = Arc::clone(&publisher);
    let _htaccess_install =
        htaccess_runtime_support::install_htaccess_runtime_publisher(Arc::clone(&getter));
    let _static_guard = install_static_runtime_for_tests();
    let _fcgi_guard =
        install_fcgi_runtime_for_tests(Arc::new(CaptureRequestExecutor), vec![(0, 8), (1, 8)]);
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    let ctx = ConnectionContext {
        state: Arc::clone(&state),
        proxy_client: proxy_client.clone(),
        x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };
    let _keep = (dir_a, dir_b);

    *FCGI_CAPTURE.lock().expect("lock") = None;
    let req_a = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/a/x")
        .body(())
        .expect("req");
    let _ = serve_http3_request(
        ConnectionContext {
            state: Arc::clone(&ctx.state),
            proxy_client: ctx.proxy_client.clone(),
            x_forwarded_for: ctx.x_forwarded_for.clone(),
            ops: Arc::clone(&ctx.ops),
        },
        req_a,
    )
    .await;
    let script_a = FCGI_CAPTURE.lock().expect("lock").clone().expect("a").0;
    assert!(script_a.ends_with("index-a.php"));

    *FCGI_CAPTURE.lock().expect("lock") = None;
    let req_b = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/b/y")
        .body(())
        .expect("req");
    let _ = serve_http3_request(
        ConnectionContext {
            state: ctx.state,
            proxy_client: ctx.proxy_client,
            x_forwarded_for: ctx.x_forwarded_for,
            ops: ctx.ops,
        },
        req_b,
    )
    .await;
    let script_b = FCGI_CAPTURE.lock().expect("lock").clone().expect("b").0;
    assert!(script_b.ends_with("index-b.php"));
    let _ = (_static_guard, _fcgi_guard);
}

#[test]
fn request_path_avoids_htaccess_io() {
    let core = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in [
        core.join("htaccess_runtime_registry.rs"),
        core.join("server/handler.rs"),
    ] {
        let text = std::fs::read_to_string(&file).expect("read");
        for needle in [
            ".htaccess\"",
            "parse_htaccess",
            "discover_htaccess",
            "exyonq_mod_htaccess",
            "notify::",
        ] {
            assert!(
                !text.contains(needle),
                "{file:?} must not reference `{needle}` on request path"
            );
        }
    }
}

#[tokio::test]
async fn backend_failure_after_rewrite_returns_502() {
    let _e2e_gate = FRONT_CONTROLLER_E2E_GATE.lock().await;
    struct Fail502;
    impl FcgiBackendExecutor for Fail502 {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            FcgiDispatchOutcome::BadGateway
        }
    }
    let before = fcgi_responses_502_total();
    let harness = front_controller_ctx(
        FRONT_CONTROLLER_HTACCESS,
        seed_fixture,
        Arc::new(Fail502),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
    )
    .await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/posts/x")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(fcgi_responses_502_total(), before + 1);
}
