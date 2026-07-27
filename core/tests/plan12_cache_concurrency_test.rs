//! Plan 12 v0 tranche 2 — cache concurrency + singleflight.

use bytes::Bytes;
use exyonq_cache::{build_cache_key, CacheKeyParts, NamespaceMetrics, ResponseCache, Singleflight};
use exyonq_cache::{
    cache_hits_total, cache_insertions_total, cache_singleflight_followers_total,
    cache_singleflight_leaders_total, reset_metrics_for_tests,
};
use exyonq_mod_static::{serve_static_with_cache, StaticCacheLoad, STATIC_CACHE_NAMESPACE};
use exyonq_module_api::CompiledCachePolicy;
use http_body_util::BodyExt;
use hyper::{Method, StatusCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Barrier as AsyncBarrier;

/// Global cache metrics are process-wide; serialize this suite under parallel `cargo test`.
static CACHE_CONCURRENCY_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn test_policy() -> CompiledCachePolicy {
    CompiledCachePolicy {
        name: "test".into(),
        ttl: Duration::from_secs(30),
        max_object_bytes: 1024 * 1024,
        policy_generation: 1,
    }
}

fn test_key(path: &str) -> exyonq_cache::CacheKey {
    build_cache_key(CacheKeyParts {
        site_id: 0,
        namespace: 0,
        backend_id: 0,
        runtime_generation: 1,
        policy_generation: 1,
        route_idx: 0,
        method: "GET".into(),
        scheme: "http".into(),
        host: "h".into(),
        path: path.into(),
        query: "".into(),
        content_encoding: "identity".into(),
    })
}

fn ok_load(body: &'static [u8]) -> StaticCacheLoad {
    StaticCacheLoad {
        status: StatusCode::OK.as_u16(),
        headers: vec![("content-type".into(), "text/plain".into())],
        body: Bytes::from_static(body),
        identity: None,
    }
}

fn insert_test(cache: &ResponseCache, key: exyonq_cache::CacheKey, ttl: Duration, body: Bytes) {
    cache.insert_entry(
        key,
        0, // site_id
        0, // route_idx
        1,
        ttl,
        200,
        vec![("content-type".into(), "text/plain".into())],
        body,
        None,
        STATIC_CACHE_NAMESPACE,
        Arc::from([]),
        NamespaceMetrics::NONE,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[allow(clippy::await_holding_lock)]
async fn concurrent_readers_hit_preinserted_entry() {
    let _gate = CACHE_CONCURRENCY_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let cache = Arc::new(ResponseCache::with_limits(10_000, 64 * 1024 * 1024));
    let key = test_key("/shared");
    insert_test(
        &cache,
        key.clone(),
        Duration::from_secs(30),
        Bytes::from_static(b"shared-body"),
    );
    let snap = cache.snapshot();
    let before_hits = cache_hits_total();

    let mut handles = Vec::new();
    for _ in 0..32 {
        let cache = Arc::clone(&cache);
        let key = key.clone();
        handles.push(tokio::spawn(async move {
            let entry = cache.lookup(&key).expect("hit");
            assert_eq!(entry.body.as_ref(), b"shared-body");
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }

    assert_eq!(cache_hits_total(), before_hits + 32);
    assert_eq!(cache.snapshot(), snap);
    assert!(cache.metrics_match_store());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
#[allow(clippy::await_holding_lock)]
async fn singleflight_deduplicates_concurrent_misses() {
    let _gate = CACHE_CONCURRENCY_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let cache = Arc::new(ResponseCache::with_limits(10_000, 64 * 1024 * 1024));
    let singleflight = Arc::new(Singleflight::new());
    let key = test_key("/once");
    let policy = test_policy();
    let loads = Arc::new(AtomicUsize::new(0));
    const N: usize = 32;
    let barrier = Arc::new(AsyncBarrier::new(N));
    let required_followers = (N as u64) - 1;

    // Do not use the debug-only follower hook + blocking Condvar: that deadlocks
    // forever under `--release` (hook compiled out). Wait asynchronously on the
    // always-on follower metric so the leader yields while followers join.
    let scenario = async {
        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let cache = Arc::clone(&cache);
            let singleflight = Arc::clone(&singleflight);
            let key = key.clone();
            let policy = policy.clone();
            let loads = Arc::clone(&loads);
            let barrier = Arc::clone(&barrier);
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
                    move || async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                        while cache_singleflight_followers_total() < required_followers {
                            assert!(
                                tokio::time::Instant::now() < deadline,
                                "timed out waiting for {required_followers} singleflight followers (saw {})",
                                cache_singleflight_followers_total()
                            );
                            tokio::task::yield_now().await;
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                        ok_load(b"once")
                    },
                )
                .await;
                assert_eq!(response.status(), StatusCode::OK);
                let body = response.into_body().collect().await.expect("body");
                assert_eq!(body.to_bytes().as_ref(), b"once");
            }));
        }
        for handle in handles {
            handle.await.expect("join");
        }
    };

    tokio::time::timeout(Duration::from_secs(10), scenario)
        .await
        .expect("singleflight concurrency test timed out");

    assert_eq!(loads.load(Ordering::SeqCst), 1);
    assert_eq!(cache_singleflight_leaders_total(), 1);
    assert_eq!(cache_singleflight_followers_total(), required_followers);
    assert_eq!(cache_insertions_total(), 1);
    assert!(cache.metrics_match_store());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)]
async fn leader_non_cacheable_wakes_followers() {
    let _gate = CACHE_CONCURRENCY_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let cache = Arc::new(ResponseCache::with_limits(10_000, 64 * 1024 * 1024));
    let singleflight = Arc::new(Singleflight::new());
    let key = test_key("/nocache");
    let policy = test_policy();
    let barrier = Arc::new(AsyncBarrier::new(8));

    let mut handles = Vec::new();
    for _ in 0..8 {
        let cache = Arc::clone(&cache);
        let singleflight = Arc::clone(&singleflight);
        let key = key.clone();
        let policy = policy.clone();
        let barrier = Arc::clone(&barrier);
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
                    StaticCacheLoad {
                        status: StatusCode::OK.as_u16(),
                        headers: vec![("set-cookie".into(), "a=b".into())],
                        body: Bytes::from_static(b"x"),
                        identity: None,
                    }
                },
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }

    assert_eq!(cache_insertions_total(), 0);
    assert!(!singleflight.flights_lock_held());
    assert!(cache.metrics_match_store());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)]
async fn leader_error_wakes_followers_and_releases_flight() {
    let _gate = CACHE_CONCURRENCY_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let cache = Arc::new(ResponseCache::with_limits(10_000, 64 * 1024 * 1024));
    let singleflight = Arc::new(Singleflight::new());
    let key = test_key("/404");
    let policy = test_policy();
    let barrier = Arc::new(AsyncBarrier::new(4));

    let mut handles = Vec::new();
    for _ in 0..4 {
        let cache = Arc::clone(&cache);
        let singleflight = Arc::clone(&singleflight);
        let key = key.clone();
        let policy = policy.clone();
        let barrier = Arc::clone(&barrier);
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
                    StaticCacheLoad {
                        status: StatusCode::NOT_FOUND.as_u16(),
                        headers: vec![("content-type".into(), "text/plain".into())],
                        body: Bytes::new(),
                        identity: None,
                    }
                },
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }

    assert_eq!(cache_insertions_total(), 0);
    assert!(!singleflight.flights_lock_held());
}

#[test]
fn ttl_race_never_serves_expired_and_cleanup_is_safe() {
    let _gate = CACHE_CONCURRENCY_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let cache = ResponseCache::with_limits(10, 1024 * 1024);
    let key = test_key("/ttl");
    insert_test(
        &cache,
        key.clone(),
        Duration::from_millis(2),
        Bytes::from_static(b"old"),
    );
    let old = cache.lookup(&key).expect("initial");
    let old_id = old.entry_id();
    std::thread::sleep(Duration::from_millis(5));
    insert_test(
        &cache,
        key.clone(),
        Duration::from_secs(30),
        Bytes::from_static(b"new"),
    );
    cache.try_remove_if_same(&key, old_id);
    let entry = cache.lookup(&key).expect("fresh");
    assert_eq!(entry.body.as_ref(), b"new");
    assert!(cache.metrics_match_store());
}

#[test]
fn concurrent_replace_same_key_keeps_metrics_coherent() {
    let _gate = CACHE_CONCURRENCY_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let cache = Arc::new(ResponseCache::with_limits(10, 1024 * 1024));
    let key = test_key("/rep");
    let mut handles = Vec::new();
    for i in 0..16 {
        let cache = Arc::clone(&cache);
        let key = key.clone();
        handles.push(std::thread::spawn(move || {
            let body = format!("body-{i}");
            insert_test(&cache, key, Duration::from_secs(30), Bytes::from(body));
        }));
    }
    for handle in handles {
        handle.join().expect("join");
    }
    assert_eq!(cache.snapshot().entries, 1);
    assert!(cache.metrics_match_store());
}

#[test]
fn eviction_under_concurrent_inserts_respects_limits() {
    let _gate = CACHE_CONCURRENCY_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let cache = Arc::new(ResponseCache::with_limits(4, 512));
    let mut handles = Vec::new();
    for i in 0..32 {
        let cache = Arc::clone(&cache);
        handles.push(std::thread::spawn(move || {
            let key = test_key(&format!("/e{i}"));
            insert_test(
                &cache,
                key,
                Duration::from_secs(30),
                Bytes::from_static(b"xx"),
            );
        }));
    }
    for handle in handles {
        handle.join().expect("join");
    }
    assert!(cache.snapshot().entries <= 4);
    assert!(cache.metrics_match_store());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)]
async fn get_stores_head_hits_without_empty_body_pollution() {
    let _gate = CACHE_CONCURRENCY_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let cache = Arc::new(ResponseCache::with_limits(10_000, 64 * 1024 * 1024));
    let singleflight = Singleflight::new();
    let key = test_key("/gh");
    let policy = test_policy();

    let get = serve_static_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::GET,
        &policy,
        || async { ok_load(b"full-body") },
    )
    .await;
    assert_eq!(get.status(), StatusCode::OK);
    let get_body = get.into_body().collect().await.expect("get body");
    assert_eq!(get_body.to_bytes().as_ref(), b"full-body");

    let head = serve_static_with_cache(
        &cache,
        &singleflight,
        key.clone(),
        0,
        1,
        &Method::HEAD,
        &policy,
        || async { ok_load(b"should-not-load") },
    )
    .await;
    assert_eq!(head.status(), StatusCode::OK);
    let head_body = head.into_body().collect().await.expect("head body");
    assert!(head_body.to_bytes().is_empty());
    assert_eq!(cache_insertions_total(), 1);
    assert!(cache_hits_total() >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)]
async fn backend_runs_without_store_or_flights_lock() {
    let _gate = CACHE_CONCURRENCY_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let cache = Arc::new(ResponseCache::with_limits(10_000, 64 * 1024 * 1024));
    let singleflight = Singleflight::new();
    let key = test_key("/lock");
    let policy = test_policy();

    let _ = serve_static_with_cache(
        &cache,
        &singleflight,
        key,
        0,
        1,
        &Method::GET,
        &policy,
        || async {
            assert!(
                !cache.store_read_lock_held() && !cache.store_write_lock_held(),
                "store lock held during backend"
            );
            assert!(
                !singleflight.flights_lock_held(),
                "flights lock held during backend"
            );
            ok_load(b"ok")
        },
    )
    .await;
}
