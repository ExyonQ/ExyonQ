//! Plan 12 tranche 5 — FastCGI cache singleflight.

mod fcgi_cache_hooks;
use exyonq_cache::reset_metrics_for_tests;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{
    clear_global_fcgi_dispatch_for_register_once_test, contract_service_registration_test_gate,
    register_fcgi_dispatch_service, AppConfig, FcgiBackendExecutor, FcgiDispatchOutcome,
    FcgiRuntimeRegistration, FcgiSuccessResponse,
};
use exyonq_mod_fastcgi::{
    cache_fcgi_insertions_total, reset_fcgi_cache_metrics_for_tests, FcgiRuntime,
};
use exyonq_module_api::fcgi_dispatch::FcgiDispatchRequest;
use exyonq_module_api::fcgi_script_resolver::{
    clear_fastcgi_script_resolver_for_tests, fastcgi_script_resolver_registration_test_gate,
    register_fastcgi_script_resolver,
};
use fcgi_cache_hooks::ensure_fcgi_cache_hooks;
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Barrier as AsyncBarrier;

fn reset_cache_metrics_for_tests() {
    reset_metrics_for_tests();
    reset_fcgi_cache_metrics_for_tests();
}

struct CountingExecutor {
    hits: Arc<AtomicUsize>,
}

impl FcgiBackendExecutor for CountingExecutor {
    fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        self.hits.fetch_add(1, Ordering::SeqCst);
        FcgiDispatchOutcome::Success(FcgiSuccessResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"once-fcgi".to_vec(),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
#[allow(clippy::await_holding_lock)]
async fn fcgi_singleflight_deduplicates_concurrent_misses() {
    ensure_fcgi_cache_hooks();
    let _gate = contract_service_registration_test_gate();
    let _resolver_gate = fastcgi_script_resolver_registration_test_gate();
    clear_global_fcgi_dispatch_for_register_once_test();
    clear_fastcgi_script_resolver_for_tests();
    register_fastcgi_script_resolver(exyonq_mod_fastcgi::FastcgiScriptResolver::arc())
        .expect("register global script resolver for multi-thread test");
    reset_cache_metrics_for_tests();
    let hits = Arc::new(AtomicUsize::new(0));

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("public")).expect("mkdir");
    std::fs::write(tmp.path().join("public/index.php"), "<?php\n").expect("php");

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
    // Cap048: External CountingExecutor after Module bind from publish.
    register_fcgi_dispatch_service(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(CountingExecutor {
                hits: Arc::clone(&hits),
            }),
            pool_capacities: vec![(0, 32)],
        })
        .expect("fcgi runtime"),
    ))
    .expect("register global fcgi for multi-thread test");
    let ctx = Arc::new(ConnectionContext {
        state,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    });

    let barrier = Arc::new(AsyncBarrier::new(32));
    let uri = "http://127.0.0.1/public/index.php";

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
            assert_eq!(body.to_bytes().as_ref(), b"once-fcgi");
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(cache_fcgi_insertions_total(), 1);
}
