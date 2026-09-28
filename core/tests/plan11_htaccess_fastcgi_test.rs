//! Plan 11 tranche B — htaccess overlay orthogonal to FastCGI backend (end-to-end).

mod htaccess_runtime_support;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{
    clear_global_fcgi_dispatch_for_register_once_test, compile_htaccess_site_bindings,
    contract_service_registration_test_gate, fcgi_metrics_assert_guard,
    register_fcgi_dispatch_service, AppConfig, Backend, FcgiDispatchTestGuard,
    FcgiRuntimeRegistration, StaticDispatchTestGuard, DEFAULT_FCGI_MAX_CONCURRENCY,
};
use exyonq_metrics::{fcgi_responses_501_total, KernelShellMetrics};
use exyonq_mod_fastcgi::{
    fcgi_responses_502_total, fcgi_responses_503_total, fcgi_responses_504_total, FcgiRuntime,
};
use exyonq_mod_htaccess::{compile_vhost_overlay, OverlayPublisher};
use exyonq_mod_static::StaticRuntime;
use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchRequest, FcgiSuccessResponse,
};
use exyonq_module_api::kernel_observation::KernelObservationTestGuard;
use exyonq_module_api::static_dispatch::StaticDispatchService;
use http_body_util::BodyExt;
use hyper::header::HeaderValue;
use hyper::{Method, Request, StatusCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static FCGI_SCRIPT_CAPTURE: Mutex<Option<String>> = Mutex::new(None);
static FCGI_501_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static HTACCESS_OVERLAY_TEST_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Keeps overlay document root on disk for FastCGI request-time script resolution.
struct FcgiOverlayHarness {
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

fn clone_ctx(ctx: &ConnectionContext) -> ConnectionContext {
    ConnectionContext {
        state: Arc::clone(&ctx.state),
        proxy_client: ctx.proxy_client.clone(),
        x_forwarded_for: ctx.x_forwarded_for.clone(),
        ops: Arc::clone(&ctx.ops),
    }
}

struct CaptureScriptExecutor;
impl FcgiBackendExecutor for CaptureScriptExecutor {
    fn dispatch(&self, request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        *FCGI_SCRIPT_CAPTURE.lock().expect("lock") = Some(request.script_filename.clone());
        FcgiDispatchOutcome::Success(FcgiSuccessResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"fcgi-ok".to_vec(),
        })
    }
}

async fn fcgi_overlay_ctx(
    htaccess_body: &str,
    seed: impl FnOnce(&std::path::Path),
    executor: Arc<dyn FcgiBackendExecutor>,
) -> FcgiOverlayHarness {
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
    assert_eq!(compile_htaccess_site_bindings(&config).len(), 1);

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
    // Cap048: External Capture must be installed after Module bind from publish.
    let _fcgi_guard =
        install_fcgi_runtime_for_tests(executor, vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)]);
    FcgiOverlayHarness {
        ctx: ConnectionContext {
            state,
            proxy_client: proxy_client.clone(),
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

#[tokio::test]
async fn fcgi_overlay_snapshot_wires_pool_document_root() {
    let harness = fcgi_overlay_ctx(
        "DirectoryIndex index.php\n",
        |root| {
            std::fs::write(root.join("index.php"), b"<?php").expect("write");
        },
        Arc::new(CaptureScriptExecutor),
    )
    .await;
    let snap = &harness.ctx.state.snapshot;
    let (idx, _) = snap
        .route_index()
        .match_route_index_with_host("/", None)
        .expect("route");
    assert_eq!(snap.htaccess_site_id_for_route(idx), Some("php"));
    let pool_id = match snap.resolve_backend(idx).expect("backend") {
        Backend::Fastcgi { pool_id } => pool_id,
        other => panic!("expected fastcgi, got {other:?}"),
    };
    assert!(
        snap.fcgi_pool_document_root(*pool_id).is_some(),
        "pool {pool_id} missing document_root"
    );
}

#[tokio::test]
async fn fcgi_overlay_directory_index_resolves_index_php_end_to_end() {
    *FCGI_SCRIPT_CAPTURE.lock().expect("lock") = None;
    let harness = fcgi_overlay_ctx(
        "DirectoryIndex index.php index.html\n",
        |root| {
            std::fs::write(root.join("index.php"), b"<?php").expect("write php");
        },
        Arc::new(CaptureScriptExecutor),
    )
    .await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);
    let script = FCGI_SCRIPT_CAPTURE
        .lock()
        .expect("lock")
        .clone()
        .expect("script captured");
    assert!(script.ends_with("index.php"));
}

#[tokio::test]
async fn fcgi_overlay_static_fallback_serves_index_html_when_php_missing() {
    let harness = fcgi_overlay_ctx(
        "DirectoryIndex index.php index.html\n",
        |root| {
            std::fs::write(root.join("index.html"), b"static-html").expect("write html");
        },
        Arc::new(CaptureScriptExecutor),
    )
    .await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
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
    assert_eq!(body.as_ref(), b"static-html");
}

#[tokio::test]
async fn fcgi_overlay_redirect_precedes_fastcgi_dispatch() {
    let _overlay_gate = HTACCESS_OVERLAY_TEST_GATE.lock().await;
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct NoCall;
    impl FcgiBackendExecutor for NoCall {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            CALLS.fetch_add(1, Ordering::SeqCst);
            FcgiDispatchOutcome::BadGateway
        }
    }
    let _static_guard = install_static_runtime_for_tests();

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join(".htaccess"),
        "Redirect 301 /legacy/ /elsewhere\nDirectoryIndex index.php\n",
    )
    .expect("write");
    std::fs::write(tmp.path().join("index.php"), b"<?php").expect("write");

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
    let _fcgi_guard = install_fcgi_runtime_for_tests(Arc::new(NoCall), vec![(0, 1)]);
    let ctx = ConnectionContext {
        state,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/legacy/")
        .body(())
        .expect("req");
    let response = serve_http3_request(ctx, req).await;
    assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    let _ = (_static_guard, _fcgi_guard);
}

#[tokio::test]
async fn fcgi_overlay_direct_script_path_skips_directory_index() {
    *FCGI_SCRIPT_CAPTURE.lock().expect("lock") = None;
    let harness = fcgi_overlay_ctx(
        "DirectoryIndex index.php\n",
        |root| {
            std::fs::write(root.join("app.php"), b"<?php").expect("write app");
            std::fs::write(root.join("index.php"), b"<?php").expect("write index");
        },
        Arc::new(CaptureScriptExecutor),
    )
    .await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/app.php")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);
    let script = FCGI_SCRIPT_CAPTURE
        .lock()
        .expect("lock")
        .clone()
        .expect("script");
    assert!(script.ends_with("app.php"));
}

#[tokio::test]
async fn fcgi_overlay_options_minus_indexes_still_serves_directory_index() {
    let harness = fcgi_overlay_ctx(
        "Options -Indexes\nDirectoryIndex index.html\n",
        |root| {
            std::fs::write(root.join("index.html"), b"indexed").expect("write");
        },
        Arc::new(CaptureScriptExecutor),
    )
    .await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn fcgi_overlay_options_minus_indexes_empty_dir_returns_not_found() {
    let harness = fcgi_overlay_ctx(
        "Options -Indexes\nDirectoryIndex missing.html\n",
        |root| {
            std::fs::create_dir(root.join("empty")).expect("mkdir");
        },
        Arc::new(CaptureScriptExecutor),
    )
    .await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/empty/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn fcgi_overlay_site_a_does_not_affect_site_b() {
    let _overlay_gate = HTACCESS_OVERLAY_TEST_GATE.lock().await;
    let dir_a = tempfile::tempdir().expect("tempdir a");
    let dir_b = tempfile::tempdir().expect("tempdir b");
    std::fs::write(
        dir_a.path().join(".htaccess"),
        "Redirect 301 /a/only-a/ /a-new\n",
    )
    .expect("write a");
    std::fs::write(
        dir_b.path().join(".htaccess"),
        "Redirect 301 /b/only-b/ /b-new\n",
    )
    .expect("write b");
    std::fs::write(dir_a.path().join("index.php"), b"<?php").expect("php a");
    std::fs::write(dir_b.path().join("index.php"), b"<?php").expect("php b");

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
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    let _fcgi_guard =
        install_fcgi_runtime_for_tests(Arc::new(CaptureScriptExecutor), vec![(0, 8), (1, 8)]);
    let ctx = ConnectionContext {
        state,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };

    let req_a = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/a/only-a/")
        .body(())
        .expect("req");
    let resp_a = serve_http3_request(clone_ctx(&ctx), req_a).await;
    assert_eq!(resp_a.status(), StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        resp_a
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok()),
        Some("/a-new")
    );

    let req_b = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/b/only-b/")
        .body(())
        .expect("req");
    let resp_b = serve_http3_request(ctx, req_b).await;
    assert_eq!(resp_b.status(), StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        resp_b
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok()),
        Some("/b-new")
    );
    let _ = (_static_guard, _fcgi_guard);
}

#[tokio::test]
async fn fcgi_overlay_no_executor_returns_501() {
    let _obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    let _overlay_gate = HTACCESS_OVERLAY_TEST_GATE.lock().await;
    let _gate = FCGI_501_GATE.lock().await;
    let before = fcgi_responses_501_total();
    let _static_guard = install_static_runtime_for_tests();
    let _fcgi_absent = FcgiDispatchTestGuard::force_absent();

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".htaccess"), "DirectoryIndex index.php\n").expect("write");
    std::fs::write(tmp.path().join("index.php"), b"<?php").expect("write");
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
        proxy_client: proxy_client.clone(),
        x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    let response = serve_http3_request(ctx, req).await;
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(fcgi_responses_501_total(), before + 1);
    let _ = (_static_guard, _fcgi_absent);
}

async fn fcgi_overlay_with_executor(
    executor: Arc<dyn FcgiBackendExecutor>,
    capacities: Vec<(u32, usize)>,
    htaccess_body: &str,
    seed: impl FnOnce(&std::path::Path),
) -> FcgiOverlayHarness {
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
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    // Cap048: External after publish.
    let _fcgi_guard = install_fcgi_runtime_for_tests(executor, capacities);
    FcgiOverlayHarness {
        ctx: ConnectionContext {
            state,
            proxy_client: proxy_client.clone(),
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

#[tokio::test]
async fn fcgi_overlay_backend_failure_returns_502() {
    struct Fail502;
    impl FcgiBackendExecutor for Fail502 {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            FcgiDispatchOutcome::BadGateway
        }
    }
    let before = fcgi_responses_502_total();
    let harness = fcgi_overlay_with_executor(
        Arc::new(Fail502),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
        "DirectoryIndex index.php\n",
        |root| {
            std::fs::write(root.join("index.php"), b"<?php").expect("write");
        },
    )
    .await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(fcgi_responses_502_total(), before + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)]
async fn fcgi_overlay_saturation_returns_503() {
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

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".htaccess"), "DirectoryIndex index.php\n").expect("htaccess");
    std::fs::write(tmp.path().join("index.php"), b"<?php").expect("write");
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
    register_fcgi_dispatch_service(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: gate_executor,
            pool_capacities: vec![(0, 1)],
        })
        .expect("fcgi runtime"),
    ))
    .expect("register global fcgi for multi-thread test");
    let harness_ctx = ConnectionContext {
        state,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };
    let _root = tmp;

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    let publisher_for_spawn = Arc::clone(&publisher);
    let first = tokio::spawn({
        let ctx = clone_ctx(&harness_ctx);
        let req = req;
        async move {
            let _htaccess_install =
                htaccess_runtime_support::install_htaccess_runtime_publisher(publisher_for_spawn);
            serve_http3_request(ctx, req).await
        }
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("started");
    let before = fcgi_responses_503_total();
    let req2 = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
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
async fn fcgi_overlay_timeout_returns_504() {
    struct Fail504;
    impl FcgiBackendExecutor for Fail504 {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            FcgiDispatchOutcome::GatewayTimeout
        }
    }
    let before = fcgi_responses_504_total();
    let harness = fcgi_overlay_with_executor(
        Arc::new(Fail504),
        vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
        "DirectoryIndex index.php\n",
        |root| {
            std::fs::write(root.join("index.php"), b"<?php").expect("write");
        },
    )
    .await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(fcgi_responses_504_total(), before + 1);
}

#[test]
fn request_path_modules_avoid_htaccess_io() {
    let core = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in [
        core.join("htaccess_runtime_registry.rs"),
        core.join("htaccess_probes.rs"),
        core.join("server/handler.rs"),
    ] {
        let text = std::fs::read_to_string(&file).expect("read");
        for needle in [
            ".htaccess\"",
            "parse_htaccess",
            "discover_htaccess",
            "exyonq_mod_htaccess",
            "notify::",
            "File::open",
        ] {
            assert!(
                !text.contains(needle),
                "{file:?} must not reference `{needle}` on request path"
            );
        }
    }
}
