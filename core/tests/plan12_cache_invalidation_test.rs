//! Plan 12 tranche 3 — static identity revalidation + targeted invalidation.

use bytes::Bytes;
use exyonq_cache::{
    build_storage_cache_key, CacheKeyParts, NamespaceMetrics, ResponseCache, Singleflight,
};
use exyonq_cache::{cache_misses_total, reset_metrics_for_tests};
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{
    clear_global_static_dispatch_for_register_once_test, contract_service_registration_test_gate,
    register_static_dispatch_service, AppConfig, StaticDispatchTestGuard,
};
use exyonq_mod_static::{
    invalidate_static_path, reset_static_cache_metrics_for_tests,
    revalidation_failure_total as cache_static_revalidation_failure_total,
    revalidation_success_total as cache_static_revalidation_success_total, serve_static_with_cache,
    static_cache_invalidations_total as cache_static_invalidations_total, StaticCacheLoad,
    STATIC_CACHE_NAMESPACE,
};
use exyonq_mod_static::{StaticResourceIdentity, StaticRuntime};
use exyonq_module_api::static_dispatch::StaticDispatchService;
use exyonq_module_api::CompiledCachePolicy;
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Barrier as AsyncBarrier;

/// `ServerState` and cache metrics are process-wide; serialize this binary under parallel `cargo test`.
static PLAN12_GLOBAL_CACHE_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn reset_cache_metrics_for_tests() {
    reset_metrics_for_tests();
    reset_static_cache_metrics_for_tests();
}

fn install_static_runtime_for_tests() -> StaticDispatchTestGuard {
    let runtime = Arc::new(StaticRuntime::new());
    let service: Arc<dyn StaticDispatchService> = runtime.clone();
    let guard = StaticDispatchTestGuard::install(service);
    exyonq_mod_static::install_kernel_hooks(runtime);
    guard
}

struct CachedCtx {
    ctx: Arc<ConnectionContext>,
    tmp: tempfile::TempDir,
    _static_guard: StaticDispatchTestGuard,
}

async fn cached_static_ctx(ttl_seconds: u64) -> CachedCtx {
    reset_cache_metrics_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("file.txt"), b"version-one").expect("write");
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
            proxy_client: proxy_client.clone(),
            x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        }),
        tmp,
        _static_guard,
    }
}

async fn get_body(ctx: &ConnectionContext, uri: &str) -> (StatusCode, Vec<u8>) {
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

async fn head_len(ctx: &ConnectionContext, uri: &str) -> (StatusCode, Option<String>) {
    let req = Request::builder()
        .method(Method::HEAD)
        .uri(uri)
        .body(())
        .expect("req");
    let response = serve_http3_request(ctx.clone(), req).await;
    let status = response.status();
    let len = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    (status, len)
}

fn test_policy() -> CompiledCachePolicy {
    CompiledCachePolicy {
        name: "test".into(),
        ttl: Duration::from_secs(300),
        max_object_bytes: 1024 * 1024,
        policy_generation: 1,
    }
}

fn test_key(path: &str, query: &str, route: usize, gen: u64) -> exyonq_cache::CacheKey {
    build_storage_cache_key(CacheKeyParts {
        site_id: 0,
        namespace: 0,
        backend_id: 0,
        runtime_generation: gen,
        policy_generation: 1,
        route_idx: route,
        method: "GET".into(),
        scheme: "http".into(),
        host: "h".into(),
        path: path.into(),
        query: query.into(),
        content_encoding: "identity".into(),
    })
}

fn snapshot_for_path(
    path: &std::path::Path,
) -> exyonq_module_api::static_dispatch::StaticResourceIdentitySnapshot {
    let identity = StaticResourceIdentity::capture(path).expect("capture");
    exyonq_mod_static::snapshot_from_identity(identity)
}

fn identity_load(path: &std::path::Path, body: &'static [u8]) -> StaticCacheLoad {
    let snapshot = StaticResourceIdentity::capture(path)
        .ok()
        .map(exyonq_mod_static::snapshot_from_identity);
    StaticCacheLoad {
        status: StatusCode::OK.as_u16(),
        headers: vec![("content-type".into(), "text/plain".into())],
        body: Bytes::from_static(body),
        identity: snapshot,
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_static_entry(
    cache: &ResponseCache,
    key: exyonq_cache::CacheKey,
    route_idx: usize,
    runtime_gen: u64,
    ttl: Duration,
    body: Bytes,
    static_identity: Option<exyonq_module_api::static_dispatch::StaticResourceIdentitySnapshot>,
    invalidation_path: Option<&std::path::Path>,
) {
    let tags: Arc<[String]> = invalidation_path
        .map(|p| {
            Arc::from([exyonq_mod_static::canonical_path_for_invalidation(p)
                .to_string_lossy()
                .into_owned()])
        })
        .unwrap_or_else(|| Arc::from([]));
    cache.insert_entry(
        key,
        0, // site_id
        route_idx,
        runtime_gen,
        ttl,
        200,
        vec![],
        body,
        static_identity,
        STATIC_CACHE_NAMESPACE,
        tags,
        NamespaceMetrics::NONE,
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn intact_file_is_cache_hit_with_revalidation() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(300).await;
    let uri = "http://127.0.0.1/file.txt";
    let (_, b1) = get_body(harness.ctx.as_ref(), uri).await;
    let before_success = cache_static_revalidation_success_total();
    let (_, b2) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(b1, b2);
    assert!(cache_static_revalidation_success_total() > before_success);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn modified_file_invalidates_and_serves_new_body() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(300).await;
    let uri = "http://127.0.0.1/file.txt";
    let (_, b1) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(b1, b"version-one");
    let before_inv = cache_static_invalidations_total();
    std::fs::write(harness.tmp.path().join("file.txt"), b"version-two-longer").expect("write");
    let (_, b2) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(b2, b"version-two-longer");
    assert!(cache_static_invalidations_total() > before_inv);
    assert!(cache_static_revalidation_failure_total() >= 1);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn atomic_rename_invalidates_cached_body() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(300).await;
    let target = harness.tmp.path().join("index.html");
    std::fs::write(&target, b"old").expect("write old");
    let uri = "http://127.0.0.1/index.html";
    let (_, b1) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(b1, b"old");
    let new_file = harness.tmp.path().join("index.new");
    std::fs::write(&new_file, b"new-content").expect("write new");
    std::fs::rename(&new_file, &target).expect("rename");
    let (_, b2) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(b2, b"new-content");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn deleted_file_does_not_hit_cache() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(300).await;
    let path = harness.tmp.path().join("file.txt");
    let uri = "http://127.0.0.1/file.txt";
    let (s1, _) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(s1, StatusCode::OK);
    std::fs::remove_file(&path).expect("delete");
    let (s2, _) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(s2, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn file_becoming_directory_is_not_hit() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(300).await;
    let path = harness.tmp.path().join("file.txt");
    let uri = "http://127.0.0.1/file.txt";
    let (s1, _) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(s1, StatusCode::OK);
    std::fs::remove_file(&path).expect("remove file");
    std::fs::create_dir(&path).expect("mkdir");
    let (s2, body) = get_body(harness.ctx.as_ref(), uri).await;
    assert_ne!(s2, StatusCode::OK);
    let _ = body;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn head_revalidates_content_length_after_file_change() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(300).await;
    let uri = "http://127.0.0.1/file.txt";
    let (_, b) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(b.len(), 11);
    let (s1, len1) = head_len(harness.ctx.as_ref(), uri).await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(len1.as_deref(), Some("11"));
    std::fs::write(harness.tmp.path().join("file.txt"), b"much-longer-body").expect("write");
    let (s2, len2) = head_len(harness.ctx.as_ref(), uri).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(len2.as_deref(), Some("16"));
    let (_, b2) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(b2, b"much-longer-body");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
#[allow(clippy::await_holding_lock)]
async fn singleflight_after_invalidation_coalesces_reload() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // `tokio::spawn` workers must see the same static service for identity revalidation.
    let _reg_gate = contract_service_registration_test_gate();
    clear_global_static_dispatch_for_register_once_test();
    let runtime = Arc::new(StaticRuntime::new());
    register_static_dispatch_service(runtime.clone()).expect("register static");
    exyonq_mod_static::install_kernel_hooks(runtime);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("coalesce.txt");
    std::fs::write(&path, b"v1").expect("write");
    reset_cache_metrics_for_tests();
    let cache = Arc::new(ResponseCache::with_limits(10_000, 64 * 1024 * 1024));
    let singleflight = Arc::new(Singleflight::new());
    let key = test_key("/coalesce.txt", "", 0, 1);
    let policy = test_policy();
    let loads = Arc::new(AtomicUsize::new(0));
    let file_path = path.clone();

    let _ = serve_static_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::GET,
        &policy,
        || async {
            loads.fetch_add(1, Ordering::SeqCst);
            identity_load(&file_path, b"v1")
        },
    )
    .await;
    assert_eq!(loads.load(Ordering::SeqCst), 1);

    std::fs::write(&path, b"v2-longer").expect("modify");
    let barrier = Arc::new(AsyncBarrier::new(32));
    let mut handles = Vec::new();
    for _ in 0..32 {
        let cache = Arc::clone(&cache);
        let singleflight = Arc::clone(&singleflight);
        let key = key.clone();
        let policy = policy.clone();
        let loads = Arc::clone(&loads);
        let barrier = Arc::clone(&barrier);
        let file_path = file_path.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let response = serve_static_with_cache(
                &cache,
                &singleflight,
                key,
                0,
                1,
                &Method::GET,
                &policy,
                || async {
                    loads.fetch_add(1, Ordering::SeqCst);
                    identity_load(&file_path, b"v2-longer")
                },
            )
            .await;
            let body = response.into_body().collect().await.expect("body");
            assert_eq!(body.to_bytes().as_ref(), b"v2-longer");
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }
    assert_eq!(loads.load(Ordering::SeqCst), 2);
}

#[test]
fn stale_cleanup_does_not_remove_replaced_entry_by_entry_id() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _static_guard = install_static_runtime_for_tests();
    reset_cache_metrics_for_tests();
    let cache = ResponseCache::with_limits(10, 1024 * 1024);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("race.txt");
    std::fs::write(&path, b"old").expect("write");
    let key = test_key("/race.txt", "", 0, 1);
    let snap = snapshot_for_path(&path);
    insert_static_entry(
        &cache,
        key.clone(),
        0,
        1,
        Duration::from_secs(300),
        Bytes::from_static(b"old"),
        Some(snap),
        Some(&path),
    );
    let old = cache.lookup(&key).expect("old");
    let old_id = old.entry_id();
    std::fs::write(&path, b"new").expect("modify");
    insert_static_entry(
        &cache,
        key.clone(),
        0,
        1,
        Duration::from_secs(300),
        Bytes::from_static(b"new"),
        StaticResourceIdentity::capture(&path)
            .ok()
            .map(exyonq_mod_static::snapshot_from_identity),
        Some(&path),
    );
    cache.try_remove_if_same(&key, old_id);
    let entry = cache.lookup(&key).expect("fresh");
    assert_eq!(entry.body.as_ref(), b"new");
}

#[test]
fn invalidate_static_path_removes_all_query_variants() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _static_guard = install_static_runtime_for_tests();
    reset_cache_metrics_for_tests();
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("shared.txt");
    std::fs::write(&path, b"x").expect("write");
    let snap = snapshot_for_path(&path);
    let k1 = test_key("/shared.txt", "", 0, 1);
    let k2 = test_key("/shared.txt", "v=1", 0, 1);
    for key in [&k1, &k2] {
        insert_static_entry(
            &cache,
            key.clone(),
            0,
            1,
            Duration::from_secs(300),
            Bytes::from_static(b"x"),
            Some(snap.clone()),
            Some(&path),
        );
    }
    assert!(cache.lookup(&k1).is_some());
    assert!(cache.lookup(&k2).is_some());
    invalidate_static_path(&cache, &path);
    assert!(cache.lookup(&k1).is_none());
    assert!(cache.lookup(&k2).is_none());
    assert!(cache.metrics_match_store());
}

#[test]
fn distinct_routes_same_textual_path_do_not_collide() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _static_guard = install_static_runtime_for_tests();
    reset_cache_metrics_for_tests();
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let dir_a = tempfile::tempdir().expect("tempdir a");
    let dir_b = tempfile::tempdir().expect("tempdir b");
    let path_a = dir_a.path().join("same.txt");
    let path_b = dir_b.path().join("same.txt");
    std::fs::write(&path_a, b"route-a").expect("write a");
    std::fs::write(&path_b, b"route-b").expect("write b");
    let snap_a = snapshot_for_path(&path_a);
    let snap_b = snapshot_for_path(&path_b);
    let k_a = test_key("/same.txt", "", 0, 1);
    let k_b = test_key("/same.txt", "", 1, 1);
    insert_static_entry(
        &cache,
        k_a.clone(),
        0,
        1,
        Duration::from_secs(300),
        Bytes::from_static(b"route-a"),
        Some(snap_a),
        Some(&path_a),
    );
    insert_static_entry(
        &cache,
        k_b.clone(),
        1,
        1,
        Duration::from_secs(300),
        Bytes::from_static(b"route-b"),
        Some(snap_b),
        Some(&path_b),
    );
    invalidate_static_path(&cache, &path_a);
    assert!(cache.lookup(&k_a).is_none());
    assert!(cache.lookup(&k_b).is_some());
}

#[test]
fn runtime_generation_isolation_after_invalidate() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _static_guard = install_static_runtime_for_tests();
    reset_cache_metrics_for_tests();
    let cache = ResponseCache::with_limits(100, 1024 * 1024);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gen.txt");
    std::fs::write(&path, b"g1").expect("write");
    let snap = snapshot_for_path(&path);
    let k_old = test_key("/gen.txt", "", 0, 1);
    let k_new = test_key("/gen.txt", "", 0, 2);
    insert_static_entry(
        &cache,
        k_old.clone(),
        0,
        1,
        Duration::from_secs(300),
        Bytes::from_static(b"g1"),
        Some(snap),
        Some(&path),
    );
    cache.invalidate_runtime_generation(1);
    assert!(cache.lookup(&k_old).is_none());
    insert_static_entry(
        &cache,
        k_new.clone(),
        0,
        2,
        Duration::from_secs(300),
        Bytes::from_static(b"g2"),
        StaticResourceIdentity::capture(&path)
            .ok()
            .map(exyonq_mod_static::snapshot_from_identity),
        Some(&path),
    );
    assert!(cache.lookup(&k_new).is_some());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn revalidation_without_watcher_guarantees_coherence() {
    let _gate = PLAN12_GLOBAL_CACHE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let harness = cached_static_ctx(300).await;
    let uri = "http://127.0.0.1/file.txt";
    get_body(harness.ctx.as_ref(), uri).await;
    std::fs::write(harness.tmp.path().join("file.txt"), b"no-watcher").expect("write");
    let before_miss = cache_misses_total();
    let (_, body) = get_body(harness.ctx.as_ref(), uri).await;
    assert_eq!(body, b"no-watcher");
    assert!(cache_misses_total() > before_miss);
}
