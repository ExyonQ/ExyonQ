//! WC2D — FastCGI MISS → origin → assess → L1 insert → HIT vertical.

use exyonq_cache::{
    build_storage_cache_key, fpc_hit_total, fpc_miss_total, fpc_store_attempt_total,
    fpc_store_rejected_total, fpc_store_success_total, reset_metrics_for_tests, CacheKeyParts,
};
use exyonq_core::reload::{active_runtime_generation, publish_runtime_generation};
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{
    AppConfig, FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchTestGuard,
    FcgiRuntimeRegistration, FcgiSuccessResponse,
};
use exyonq_mod_fastcgi::FcgiRuntime;
use exyonq_module_api::fcgi_dispatch::FcgiDispatchRequest;
use exyonq_module_api::fcgi_script_resolver::FastcgiScriptResolverTestGuard;
use exyonq_runtime_plan::stable_fpc_site_id;
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

static WC2D_FPC_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct FcgiHarness {
    ctx: Arc<ConnectionContext>,
    hits: Arc<AtomicUsize>,
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

/// Blocks between "FastCGI started" and "response ready" for generation-race e2e.
/// Pattern matches plan11 capacity tests: mpsc barrier inside spawn_blocking dispatch.
struct LatchExecutor {
    hits: Arc<AtomicUsize>,
    started_tx: mpsc::Sender<()>,
    release_rx: Mutex<mpsc::Receiver<()>>,
    response: FcgiDispatchOutcome,
}

impl FcgiBackendExecutor for LatchExecutor {
    fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        self.hits.fetch_add(1, Ordering::SeqCst);
        let _ = self.started_tx.send(());
        if let Ok(rx) = self.release_rx.lock() {
            let _ = rx.recv();
        }
        self.response.clone()
    }
}

fn success_outcome(body: &'static [u8], headers: Vec<(String, String)>) -> FcgiDispatchOutcome {
    FcgiDispatchOutcome::Success(FcgiSuccessResponse {
        status: 200,
        headers,
        body: body.to_vec(),
    })
}

async fn fpc_fcgi_ctx(
    body: &'static [u8],
    headers: Vec<(String, String)>,
    max_object_bytes: usize,
) -> FcgiHarness {
    reset_metrics_for_tests();
    let hits = Arc::new(AtomicUsize::new(0));
    let script_resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("public")).expect("mkdir");
    std::fs::write(tmp.path().join("public/index.php"), "<?php\n").expect("index.php");

    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[route]]
name = "php"
match = {{ path = "/public/" }}
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = "/tmp/exyonq-fcgi-wc2d.sock"
document_root = "{root}"

[full_page_cache]
enabled = true
namespace = 4
max_entries = 32
max_total_bytes = 1048576
max_object_bytes = {max_object_bytes}
default_ttl_seconds = 60
max_ttl_seconds = 300
"#,
        root = tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(21, config, proxy_client.clone())
        .await
        .expect("state");
    // Cap048: External Capture must be installed after ServerState publish so
    // bind_fcgi_compiled_pools cannot replace it with Module (unix sock → 502).
    let fcgi_guard = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(CountingExecutor {
                hits: Arc::clone(&hits),
                response: success_outcome(body, headers),
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
        hits,
        _root: tmp,
        _fcgi_guard: fcgi_guard,
        _script_resolver: script_resolver,
    }
}

async fn get(ctx: &ConnectionContext, uri: &str, cookie: Option<&str>) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("host", "example.test");
    if let Some(c) = cookie {
        builder = builder.header("cookie", c);
    }
    let req = builder.body(()).expect("req");
    let response = serve_http3_request(ctx.clone(), req).await;
    let status = response.status();
    let body = response.into_body().collect().await.expect("collect");
    (status, body.to_bytes().to_vec())
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_miss_fill_hit_vertical() {
    let _gate = WC2D_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_fcgi_ctx(
        b"hello-php",
        vec![("content-type".into(), "text/html".into())],
        1024 * 1024,
    )
    .await;
    let uri = "http://example.test/public/index.php";
    let before_miss = fpc_miss_total();
    let before_hit = fpc_hit_total();
    let before_store = fpc_store_success_total();

    let (s1, b1) = get(&harness.ctx, uri, None).await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(&b1[..], b"hello-php");
    assert_eq!(harness.hits.load(Ordering::SeqCst), 1);
    assert!(fpc_miss_total() > before_miss);
    assert!(fpc_store_success_total() > before_store);

    let (s2, b2) = get(&harness.ctx, uri, None).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(&b2[..], b"hello-php");
    assert_eq!(
        harness.hits.load(Ordering::SeqCst),
        1,
        "PHP-FPM must not run on HIT"
    );
    assert!(fpc_hit_total() > before_hit);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_head_reuses_get_without_poisoning_or_origin_on_hit() {
    let _gate = WC2D_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_fcgi_ctx(
        b"head-body",
        vec![("content-type".into(), "text/plain".into())],
        1024 * 1024,
    )
    .await;
    let uri = "http://example.test/public/index.php";

    let head_req = Request::builder()
        .method(Method::HEAD)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    let hr = serve_http3_request(harness.ctx.as_ref().clone(), head_req).await;
    assert_eq!(hr.status(), StatusCode::OK);
    let hb = hr.into_body().collect().await.unwrap().to_bytes();
    assert!(hb.is_empty());
    let stores_after_head = fpc_store_success_total();
    let origins_after_head = harness.hits.load(Ordering::SeqCst);
    assert!(origins_after_head >= 1);

    let (gs, gb) = get(&harness.ctx, uri, None).await;
    assert_eq!(gs, StatusCode::OK);
    assert_eq!(&gb[..], b"head-body");
    assert!(fpc_store_success_total() > stores_after_head);

    let origins_before_head_hit = harness.hits.load(Ordering::SeqCst);
    let head2 = Request::builder()
        .method(Method::HEAD)
        .uri(uri)
        .header("host", "example.test")
        .body(())
        .expect("req");
    let hr2 = serve_http3_request(harness.ctx.as_ref().clone(), head2).await;
    assert_eq!(hr2.status(), StatusCode::OK);
    let hb2 = hr2.into_body().collect().await.unwrap().to_bytes();
    assert!(hb2.is_empty());
    assert_eq!(
        harness.hits.load(Ordering::SeqCst),
        origins_before_head_hit,
        "HEAD HIT must not invoke PHP-FPM"
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_set_cookie_never_stored() {
    let _gate = WC2D_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_fcgi_ctx(
        b"session",
        vec![
            ("content-type".into(), "text/html".into()),
            ("set-cookie".into(), "PHPSESSID=abc".into()),
        ],
        1024 * 1024,
    )
    .await;
    let uri = "http://example.test/public/index.php";
    let before = fpc_store_rejected_total("set_cookie");
    let (s, b) = get(&harness.ctx, uri, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(&b[..], b"session");
    assert!(fpc_store_rejected_total("set_cookie") > before);
    assert_eq!(fpc_store_success_total(), 0);

    let before_hit = fpc_hit_total();
    let _ = get(&harness.ctx, uri, None).await;
    assert_eq!(fpc_hit_total(), before_hit);
    assert_eq!(harness.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_private_and_nocache_headers_never_stored() {
    let _gate = WC2D_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for (label, headers) in [
        (
            "private",
            vec![
                ("content-type".into(), "text/html".into()),
                ("cache-control".into(), "private, max-age=0".into()),
            ],
        ),
        (
            "no_store",
            vec![
                ("content-type".into(), "text/html".into()),
                ("cache-control".into(), "no-store".into()),
            ],
        ),
        (
            "no_cache",
            vec![
                ("content-type".into(), "text/html".into()),
                (
                    "cache-control".into(),
                    "no-cache, must-revalidate, max-age=0".into(),
                ),
            ],
        ),
    ] {
        let harness = fpc_fcgi_ctx(b"x", headers, 1024 * 1024).await;
        let before = fpc_store_rejected_total(label);
        let (s, _) = get(&harness.ctx, "http://example.test/public/index.php", None).await;
        assert_eq!(s, StatusCode::OK);
        assert!(
            fpc_store_rejected_total(label) > before,
            "expected reject {label}"
        );
        assert_eq!(fpc_store_success_total(), 0);
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_logged_in_and_woo_session_bypass() {
    let _gate = WC2D_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_fcgi_ctx(
        b"secret",
        vec![("content-type".into(), "text/html".into())],
        1024 * 1024,
    )
    .await;
    let uri = "http://example.test/public/index.php";
    let _ = get(&harness.ctx, uri, None).await;
    assert!(fpc_store_success_total() >= 1);

    let before_hit = fpc_hit_total();
    let (s, _) = get(&harness.ctx, uri, Some("wordpress_logged_in_abc=1")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(fpc_hit_total(), before_hit);

    let before_hit2 = fpc_hit_total();
    let (s2, _) = get(&harness.ctx, uri, Some("wp_woocommerce_session_deadbeef=1")).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(fpc_hit_total(), before_hit2);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_body_too_large_fail_open() {
    let _gate = WC2D_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = fpc_fcgi_ctx(
        b"oversized-body-content",
        vec![("content-type".into(), "text/plain".into())],
        4,
    )
    .await;
    let uri = "http://example.test/public/index.php";
    let before = fpc_store_rejected_total("body_too_large");
    let (s, b) = get(&harness.ctx, uri, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(&b[..], b"oversized-body-content");
    assert!(fpc_store_attempt_total() >= 1);
    assert!(fpc_store_rejected_total("body_too_large") > before);
    assert_eq!(fpc_store_success_total(), 0);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fcgi_generation_mismatch_e2e_mid_fastcgi() {
    let _gate = WC2D_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();

    const G1: u64 = 31;
    const G2: u64 = 32;
    let body = b"gen-race-body";
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let hits = Arc::new(AtomicUsize::new(0));

    let script_resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("public")).expect("mkdir");
    std::fs::write(tmp.path().join("public/index.php"), "<?php\n").expect("index.php");
    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[route]]
name = "php"
match = {{ path = "/public/" }}
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = "/tmp/exyonq-fcgi-wc2d-gen.sock"
document_root = "{root}"

[full_page_cache]
enabled = true
namespace = 4
max_entries = 32
max_total_bytes = 1048576
max_object_bytes = 1048576
default_ttl_seconds = 60
max_ttl_seconds = 300
"#,
        root = tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state_g1 = ServerState::new_with_generation(G1, config.clone(), proxy_client.clone())
        .await
        .expect("state g1");
    assert_eq!(state_g1.generation, G1);
    assert_eq!(active_runtime_generation(), G1);
    // Cap048: install Latch External after publish (Module bind would replace Capture).
    let fcgi_guard = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(LatchExecutor {
                hits: Arc::clone(&hits),
                started_tx,
                release_rx: Mutex::new(release_rx),
                response: success_outcome(body, vec![("content-type".into(), "text/html".into())]),
            }),
            pool_capacities: vec![(0, 16)],
        })
        .expect("fcgi runtime"),
    ));

    let ctx_g1 = Arc::new(ConnectionContext {
        state: Arc::clone(&state_g1),
        proxy_client: proxy_client.clone(),
        x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    });

    let uri = "http://example.test/public/index.php";
    let site_id = stable_fpc_site_id("php");
    let backend_id = state_g1.snapshot.route_backend_id(0).unwrap().index();
    let key_g1 = build_storage_cache_key(CacheKeyParts {
        site_id,
        namespace: 4,
        backend_id,
        runtime_generation: G1,
        policy_generation: 0,
        route_idx: 0,
        method: "GET".into(),
        scheme: "http".into(),
        host: "example.test".into(),
        path: "/public/index.php".into(),
        query: String::new(),
        content_encoding: "identity".into(),
    });
    let key_g2 = build_storage_cache_key(CacheKeyParts {
        site_id,
        namespace: 4,
        backend_id,
        runtime_generation: G2,
        policy_generation: 0,
        route_idx: 0,
        method: "GET".into(),
        scheme: "http".into(),
        host: "example.test".into(),
        path: "/public/index.php".into(),
        query: String::new(),
        content_encoding: "identity".into(),
    });

    let before_reject = fpc_store_rejected_total("generation_mismatch");
    let before_success = fpc_store_success_total();
    let before_miss = fpc_miss_total();

    // Keep TLS FastCGI test override visible: drive on current-thread runtime
    // while LatchExecutor blocks inside FcgiRuntime's spawn_blocking pool.
    let req_ctx = ctx_g1.as_ref().clone();
    let req_fut = async move {
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header("host", "example.test")
            .body(())
            .expect("req");
        serve_http3_request(req_ctx, req).await
    };
    tokio::pin!(req_fut);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match started_rx.try_recv() {
            Ok(()) => break,
            Err(mpsc::TryRecvError::Empty) => {
                if tokio::time::Instant::now() >= deadline {
                    panic!("FastCGI dispatch should start");
                }
                tokio::select! {
                    biased;
                    resp = &mut req_fut => {
                        panic!(
                            "request finished before FastCGI started: status={}",
                            resp.status()
                        );
                    }
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {}
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                panic!("FastCGI latch dropped before start");
            }
        }
    }
    assert!(fpc_miss_total() > before_miss);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(active_runtime_generation(), G1);

    // Advance the process-visible generation while FastCGI is in flight (reload race).
    publish_runtime_generation(G2);
    assert_eq!(active_runtime_generation(), G2);
    assert_eq!(state_g1.generation, G1, "in-flight Arc remains G1");

    release_tx.send(()).expect("release FastCGI");
    let response = req_fut.await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/html")
    );
    let got = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&got[..], body);

    assert!(fpc_store_rejected_total("generation_mismatch") > before_reject);
    assert_eq!(fpc_store_success_total(), before_success);
    let cache_g1 = state_g1.fpc_cache.as_ref().expect("fpc");
    assert_eq!(cache_g1.snapshot().entries, 0, "stale fill must not insert");
    assert!(
        cache_g1.lookup(&key_g1).is_none(),
        "no G1 insert from stale fill"
    );
    assert!(
        cache_g1.lookup(&key_g2).is_none(),
        "no G2 rewrite insert on G1 store"
    );

    // Same URL under G2: new plan generation → MISS → FastCGI runs again.
    let state_g2 = ServerState::new_with_generation(G2, config, proxy_client.clone())
        .await
        .expect("state g2");
    assert_eq!(active_runtime_generation(), G2);
    let ctx_g2 = Arc::new(ConnectionContext {
        state: Arc::clone(&state_g2),
        proxy_client: proxy_client.clone(),
        x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    });
    // Reinstall a non-latching executor for the follow-up request.
    drop(fcgi_guard);
    let hits2 = Arc::new(AtomicUsize::new(0));
    let _fcgi_guard2 = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(CountingExecutor {
                hits: Arc::clone(&hits2),
                response: success_outcome(body, vec![("content-type".into(), "text/html".into())]),
            }),
            pool_capacities: vec![(0, 16)],
        })
        .expect("fcgi runtime"),
    ));
    let _script_resolver = script_resolver;

    let before_hit = fpc_hit_total();
    let (s2, b2) = get(&ctx_g2, uri, None).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(&b2[..], body);
    assert_eq!(
        hits2.load(Ordering::SeqCst),
        1,
        "G2 must MISS and execute FastCGI"
    );
    assert_eq!(
        fpc_hit_total(),
        before_hit,
        "stale G1 fill must not produce a G2 HIT"
    );
    assert!(state_g2
        .fpc_cache
        .as_ref()
        .unwrap()
        .lookup(&key_g1)
        .is_none());
}
