//! WC2B — FPC eligibility + L1 lookup (no insert) dataplane tests.

use bytes::Bytes;
use exyonq_cache::{
    build_storage_cache_key, fpc_bypass_total, fpc_hit_total, fpc_lookup_total, fpc_miss_total,
    reset_metrics_for_tests, CacheKeyParts, CacheNamespace, NamespaceMetrics, ResponseCache,
};
use exyonq_core::fpc_lookup::try_fpc_lookup_hit;
use exyonq_core::server::state::ServerState;
use exyonq_core::AppConfig;
use exyonq_runtime_plan::stable_fpc_site_id;
use http_body_util::BodyExt;
use hyper::Method;
use std::sync::Arc;
use std::time::Duration;

/// Process-wide FPC metrics — serialize this binary under parallel `cargo test`.
static WC2B_FPC_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn minimal_static_config(fpc_enabled: bool) -> AppConfig {
    let toml = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:0"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = "/tmp"

[full_page_cache]
enabled = {fpc_enabled}
namespace = 4
max_entries = 100
max_total_bytes = 1048576
default_ttl_seconds = 30
"#
    );
    toml.parse().expect("config")
}

async fn state(fpc_enabled: bool) -> Arc<ServerState> {
    let config = minimal_static_config(fpc_enabled);
    let proxy = exyonq_mod_proxy::build_incoming_client();
    ServerState::new_with_generation(7, config, proxy)
        .await
        .expect("state")
}

fn insert_hit(cache: &ResponseCache, parts: CacheKeyParts, body: &'static [u8]) {
    let site_id = parts.site_id;
    let ns = CacheNamespace::new(parts.namespace);
    let key = build_storage_cache_key(parts);
    cache.insert_entry(
        key,
        site_id,
        0,
        7,
        Duration::from_secs(60),
        200,
        vec![("content-type".into(), "text/plain".into())],
        Bytes::from_static(body),
        None,
        ns,
        Arc::from([]),
        NamespaceMetrics::NONE,
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn fpc_disabled_skips_lookup() {
    let _gate = WC2B_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let state = state(false).await;
    assert!(state.fpc_cache.is_none());
    let uri: hyper::Uri = "http://example.test/".parse().unwrap();
    let hit = try_fpc_lookup_hit(&state, 0, &Method::GET, &uri, Some("example.test"), &[]);
    assert!(hit.is_none());
    assert_eq!(fpc_lookup_total(), 0);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn eligible_miss_continues_without_hit() {
    let _gate = WC2B_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let state = state(true).await;
    assert!(state.fpc_cache.is_some());
    let uri: hyper::Uri = "http://example.test/".parse().unwrap();
    let before_lookup = fpc_lookup_total();
    let before_miss = fpc_miss_total();
    let hit = try_fpc_lookup_hit(&state, 0, &Method::GET, &uri, Some("example.test"), &[]);
    assert!(hit.is_none());
    assert!(fpc_lookup_total() > before_lookup);
    assert!(fpc_miss_total() > before_miss);
    assert_eq!(fpc_hit_total(), 0);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn eligible_hit_serves_without_backend_and_head_has_no_body() {
    let _gate = WC2B_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let state = state(true).await;
    let cache = state.fpc_cache.as_ref().expect("fpc cache");
    let site_id = stable_fpc_site_id("site");
    assert_ne!(site_id, 0);
    let backend_id = state.snapshot.route_backend_id(0).expect("backend").index();
    let parts = CacheKeyParts {
        site_id,
        namespace: 4,
        backend_id,
        runtime_generation: 7,
        policy_generation: 0,
        route_idx: 0,
        method: "GET".into(),
        scheme: "http".into(),
        host: "example.test".into(),
        path: "/".into(),
        query: String::new(),
        content_encoding: "identity".into(),
    };
    insert_hit(cache, parts.clone(), b"cached-body");

    let uri: hyper::Uri = "http://example.test/".parse().unwrap();
    let before_hit = fpc_hit_total();
    let get =
        try_fpc_lookup_hit(&state, 0, &Method::GET, &uri, Some("example.test"), &[]).expect("hit");
    assert_eq!(get.status(), 200);
    let body = get.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"cached-body");
    assert!(fpc_hit_total() > before_hit);

    let head = try_fpc_lookup_hit(&state, 0, &Method::HEAD, &uri, Some("example.test"), &[])
        .expect("head hit");
    assert_eq!(head.status(), 200);
    let head_body = head.into_body().collect().await.unwrap().to_bytes();
    assert!(head_body.is_empty());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn cookie_bypass_does_not_hit_anonymous_entry() {
    let _gate = WC2B_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let state = state(true).await;
    let cache = state.fpc_cache.as_ref().expect("fpc cache");
    let site_id = stable_fpc_site_id("site");
    let backend_id = state.snapshot.route_backend_id(0).unwrap().index();
    insert_hit(
        cache,
        CacheKeyParts {
            site_id,
            namespace: 4,
            backend_id,
            runtime_generation: 7,
            policy_generation: 0,
            route_idx: 0,
            method: "GET".into(),
            scheme: "http".into(),
            host: "example.test".into(),
            path: "/".into(),
            query: String::new(),
            content_encoding: "identity".into(),
        },
        b"secret",
    );
    let uri: hyper::Uri = "http://example.test/".parse().unwrap();
    let before_bypass = fpc_bypass_total();
    let bypassed = try_fpc_lookup_hit(
        &state,
        0,
        &Method::GET,
        &uri,
        Some("example.test"),
        &[("Cookie".into(), "wordpress_logged_in_x=1".into())],
    );
    assert!(bypassed.is_none());
    assert!(fpc_bypass_total() > before_bypass);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn unknown_query_bypasses() {
    let _gate = WC2B_FPC_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let state = state(true).await;
    let uri: hyper::Uri = "http://example.test/?p=1".parse().unwrap();
    let before_bypass = fpc_bypass_total();
    assert!(try_fpc_lookup_hit(&state, 0, &Method::GET, &uri, Some("example.test"), &[]).is_none());
    assert!(fpc_bypass_total() > before_bypass);
}

#[tokio::test]
async fn distinct_site_ids_isolate_keys() {
    let a = stable_fpc_site_id("site-a");
    let b = stable_fpc_site_id("site-b");
    assert_ne!(a, 0);
    assert_ne!(b, 0);
    assert_ne!(a, b);
    let ka = build_storage_cache_key(CacheKeyParts {
        site_id: a,
        namespace: 4,
        backend_id: 1,
        runtime_generation: 1,
        policy_generation: 0,
        route_idx: 0,
        method: "GET".into(),
        scheme: "http".into(),
        host: "h".into(),
        path: "/".into(),
        query: String::new(),
        content_encoding: "identity".into(),
    });
    let kb = build_storage_cache_key(CacheKeyParts {
        site_id: b,
        namespace: 4,
        backend_id: 1,
        runtime_generation: 1,
        policy_generation: 0,
        route_idx: 0,
        method: "GET".into(),
        scheme: "http".into(),
        host: "h".into(),
        path: "/".into(),
        query: String::new(),
        content_encoding: "identity".into(),
    });
    assert_ne!(ka, kb);
}
