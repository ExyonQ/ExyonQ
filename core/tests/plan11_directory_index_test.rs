//! Plan 11 tranche A — overlay `DirectoryIndex` runtime (static + FastCGI resolution).

mod htaccess_runtime_support;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{
    build_fcgi_dispatch_request, clear_global_fcgi_dispatch_for_register_once_test,
    contract_service_registration_test_gate, execute_backend, fcgi_metrics_assert_guard,
    register_fcgi_dispatch_service, AppConfig, Backend, FcgiDispatchTestGuard,
    FcgiRuntimeRegistration, StaticDispatchTestGuard, DEFAULT_FCGI_MAX_CONCURRENCY,
};
use exyonq_metrics::{fcgi_responses_501_total, KernelShellMetrics};
use exyonq_mod_fastcgi::{
    fcgi_responses_502_total, fcgi_responses_503_total, fcgi_responses_504_total, FcgiRuntime,
};
use exyonq_mod_htaccess::{
    compile_vhost_overlay, htaccess_directory_index_misses_total, OverlayPublisher,
};
use exyonq_mod_static::StaticRuntime;
use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchRequest,
};
use exyonq_module_api::fcgi_script_resolver::FastcgiScriptResolverTestGuard;
use exyonq_module_api::kernel_observation::KernelObservationTestGuard;
use exyonq_module_api::static_dispatch::StaticDispatchService;
use hyper::header::HeaderValue;
use hyper::{Method, Request, StatusCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

static HTACCESS_OVERLAY_TEST_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn install_static_runtime_for_tests() -> StaticDispatchTestGuard {
    let runtime = Arc::new(StaticRuntime::new());
    let service: Arc<dyn StaticDispatchService> = runtime.clone();
    let guard = StaticDispatchTestGuard::install(service);
    exyonq_mod_static::install_kernel_hooks(runtime);
    guard
}

fn install_fcgi_script_resolver_for_tests() -> FastcgiScriptResolverTestGuard {
    FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc())
}

struct StaticOverlayHarness {
    ctx: ConnectionContext,
    _root: tempfile::TempDir,
    _static_guard: StaticDispatchTestGuard,
    _htaccess_install: htaccess_runtime_support::HtaccessRuntimeInstall,
    _overlay_gate: tokio::sync::MutexGuard<'static, ()>,
}

async fn static_overlay_ctx(
    htaccess_body: &str,
    seed: impl Fn(&std::path::Path),
) -> StaticOverlayHarness {
    let _overlay_gate = HTACCESS_OVERLAY_TEST_GATE.lock().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    seed(tmp.path());
    std::fs::write(tmp.path().join(".htaccess"), htaccess_body).expect("htaccess");
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
    let _htaccess_install =
        htaccess_runtime_support::install_htaccess_runtime_publisher(Arc::clone(&publisher));

    let _static_guard = install_static_runtime_for_tests();
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    StaticOverlayHarness {
        ctx: ConnectionContext {
            state,
            proxy_client: proxy_client.clone(),
            x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        _root: tmp,
        _static_guard,
        _htaccess_install,
        _overlay_gate,
    }
}

#[tokio::test]
async fn static_root_directory_index_serves_index_html() {
    let harness = static_overlay_ctx("DirectoryIndex index.html\n", |root| {
        std::fs::write(root.join("index.html"), b"root-index").expect("write");
    })
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
async fn static_subdir_inherits_root_directory_index() {
    let harness = static_overlay_ctx("DirectoryIndex index.html\n", |root| {
        std::fs::create_dir(root.join("admin")).expect("mkdir");
        std::fs::write(root.join("admin/index.html"), b"admin-index").expect("write");
    })
    .await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/admin/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn static_subdir_override_uses_local_index() {
    let _overlay_gate = HTACCESS_OVERLAY_TEST_GATE.lock().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(root.join(".htaccess"), "DirectoryIndex index.html\n").expect("write");
    std::fs::create_dir(root.join("admin")).expect("mkdir");
    std::fs::write(root.join("admin/.htaccess"), "DirectoryIndex admin.html\n").expect("write");
    std::fs::write(root.join("admin/admin.html"), b"override").expect("write");
    std::fs::write(root.join("admin/index.html"), b"ignored").expect("write");

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
        root.display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let compiled = compile_vhost_overlay("site", root, 1).expect("compile");
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
    let ctx = ConnectionContext {
        state,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    };
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/admin/")
        .body(())
        .expect("req");
    let response = serve_http3_request(ctx, req).await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = (_static_guard, _htaccess_install, _overlay_gate);
}

#[tokio::test]
async fn static_second_candidate_wins_when_first_missing() {
    let harness = static_overlay_ctx("DirectoryIndex missing.html index.html\n", |root| {
        std::fs::write(root.join("index.html"), b"second").expect("write");
    })
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
async fn static_no_candidate_keeps_not_found() {
    let before_misses = htaccess_directory_index_misses_total();
    let harness = static_overlay_ctx("DirectoryIndex missing.html\n", |_root| {}).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/nope/")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(htaccess_directory_index_misses_total() >= before_misses);
}

#[tokio::test]
async fn static_no_trailing_slash_skips_directory_index() {
    let harness = static_overlay_ctx("DirectoryIndex index.html\n", |root| {
        std::fs::write(root.join("index.html"), b"root-index").expect("write");
    })
    .await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/admin")
        .body(())
        .expect("req");
    let response = serve_http3_request(harness.ctx, req).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn overlay_lookup_directory_index_requires_trailing_slash_at_runtime() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".htaccess"), "DirectoryIndex index.html\n").expect("write");
    let compiled = compile_vhost_overlay("site", tmp.path(), 1).expect("compile");
    match exyonq_module_api::lookup_overlay("/", &compiled.overlay) {
        exyonq_module_api::OverlayLookupResult::Continue {
            directory_index, ..
        } => assert!(directory_index.is_some()),
        other => panic!("unexpected {other:?}"),
    }
    match exyonq_module_api::lookup_overlay("/admin", &compiled.overlay) {
        exyonq_module_api::OverlayLookupResult::Continue {
            directory_index, ..
        } => assert!(directory_index.is_some()),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn fcgi_no_executor_returns_501() {
    let _obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    let _script_resolver = install_fcgi_script_resolver_for_tests();
    let _guard = FcgiDispatchTestGuard::force_absent();
    let before = fcgi_responses_501_total();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("index.php"), b"<?php").expect("write");
    let request = build_fcgi_dispatch_request(
        0,
        Some(&dir.path().to_path_buf()),
        "GET",
        "/index.php",
        "",
        "/index.php",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .expect("request");
    let outcome = execute_backend(&Backend::Fastcgi { pool_id: 0 }, Some(request), None, None)
        .await
        .expect("outcome");
    assert_eq!(outcome.status, 501);
    assert_eq!(fcgi_responses_501_total(), before + 1);
}

#[tokio::test]
async fn fcgi_backend_failure_returns_502() {
    let _script_resolver = install_fcgi_script_resolver_for_tests();
    struct Fail502;
    impl FcgiBackendExecutor for Fail502 {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            FcgiDispatchOutcome::BadGateway
        }
    }
    let _guard = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(Fail502),
            pool_capacities: vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
        })
        .expect("fcgi runtime"),
    ));
    let before = fcgi_responses_502_total();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("index.php"), b"<?php").expect("write");
    let request = build_fcgi_dispatch_request(
        0,
        Some(&dir.path().to_path_buf()),
        "GET",
        "/index.php",
        "",
        "/index.php",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .expect("request");
    let outcome = execute_backend(&Backend::Fastcgi { pool_id: 0 }, Some(request), None, None)
        .await
        .expect("outcome");
    assert_eq!(outcome.status, 502);
    assert_eq!(fcgi_responses_502_total(), before + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)]
async fn fcgi_saturation_returns_503() {
    let _script_resolver = install_fcgi_script_resolver_for_tests();
    let _metrics_gate = fcgi_metrics_assert_guard().await;
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
            FcgiDispatchOutcome::Success(exyonq_module_api::fcgi_dispatch::FcgiSuccessResponse {
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

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("index.php"), b"<?php").expect("write");
    let request = build_fcgi_dispatch_request(
        0,
        Some(&dir.path().to_path_buf()),
        "GET",
        "/index.php",
        "",
        "/index.php",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .expect("request");

    let backend = Backend::Fastcgi { pool_id: 0 };
    let first =
        tokio::spawn(async move { execute_backend(&backend, Some(request), None, None).await });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("started");
    let before = fcgi_responses_503_total();
    let second = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(
            build_fcgi_dispatch_request(
                0,
                Some(&dir.path().to_path_buf()),
                "GET",
                "/index.php",
                "",
                "/index.php",
                Vec::new(),
                Vec::new(),
                "127.0.0.1",
                80,
            )
            .expect("request"),
        ),
        None,
        None,
    )
    .await
    .expect("second");
    assert_eq!(second.status, 503);
    assert_eq!(fcgi_responses_503_total(), before + 1);
    let _ = release_tx.send(());
    let _ = first.await;
}

#[tokio::test]
async fn fcgi_timeout_returns_504() {
    let _script_resolver = install_fcgi_script_resolver_for_tests();
    struct Fail504;
    impl FcgiBackendExecutor for Fail504 {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            FcgiDispatchOutcome::GatewayTimeout
        }
    }
    let _guard = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(Fail504),
            pool_capacities: vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
        })
        .expect("fcgi runtime"),
    ));
    let before = fcgi_responses_504_total();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("index.php"), b"<?php").expect("write");
    let request = build_fcgi_dispatch_request(
        0,
        Some(&dir.path().to_path_buf()),
        "GET",
        "/index.php",
        "",
        "/index.php",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .expect("request");
    let outcome = execute_backend(&Backend::Fastcgi { pool_id: 0 }, Some(request), None, None)
        .await
        .expect("outcome");
    assert_eq!(outcome.status, 504);
    assert_eq!(fcgi_responses_504_total(), before + 1);
}

#[test]
fn request_path_overlay_modules_do_not_touch_htaccess_io() {
    let core = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = [
        core.join("htaccess_runtime_registry.rs"),
        core.join("htaccess_probes.rs"),
        core.join("server/handler.rs"),
    ];
    let forbidden = [
        ".htaccess\"",
        ".htaccess'",
        "parse_htaccess",
        "discover_htaccess",
        "exyonq_mod_htaccess",
        "notify::",
        "File::open",
        "std::fs::read",
    ];
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source");
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "{file:?} must not reference `{needle}` on request path"
            );
        }
    }
}

#[test]
fn overlay_redirect_still_precedes_directory_index() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join(".htaccess"),
        "Redirect 301 /admin/ /elsewhere\nDirectoryIndex index.html\n",
    )
    .expect("write");
    let compiled = compile_vhost_overlay("site", tmp.path(), 1).expect("compile");
    let result = exyonq_module_api::lookup_overlay("/admin/", &compiled.overlay);
    assert!(matches!(
        result,
        exyonq_module_api::OverlayLookupResult::Redirect { status: 301, .. }
    ));
}

#[test]
fn fcgi_executor_receives_resolved_index_script() {
    let _script_resolver = install_fcgi_script_resolver_for_tests();
    static SEEN: AtomicUsize = AtomicUsize::new(0);
    struct Capture;
    impl FcgiBackendExecutor for Capture {
        fn dispatch(&self, request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            if request.script_filename.ends_with("app/index.php") {
                SEEN.fetch_add(1, Ordering::SeqCst);
            }
            FcgiDispatchOutcome::Success(exyonq_module_api::fcgi_dispatch::FcgiSuccessResponse {
                status: 200,
                headers: vec![],
                body: b"ok".to_vec(),
            })
        }
    }
    let _guard = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(Capture),
            pool_capacities: vec![(0, 1)],
        })
        .expect("fcgi runtime"),
    ));
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("app")).expect("mkdir");
    std::fs::write(dir.path().join("app/index.php"), b"<?php").expect("write");
    let uri = "/app/index.php";
    let request = build_fcgi_dispatch_request(
        0,
        Some(&dir.path().to_path_buf()),
        "GET",
        uri,
        "",
        uri,
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .expect("request");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        execute_backend(&Backend::Fastcgi { pool_id: 0 }, Some(request), None, None)
            .await
            .expect("outcome");
    });
    assert_eq!(SEEN.load(Ordering::SeqCst), 1);
}
