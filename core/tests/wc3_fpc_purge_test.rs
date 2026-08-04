//! WC3 — authenticated FPC purge control (CachePurgePort + store).

use exyonq_cache::{
    build_storage_cache_key, fpc_purge_rejected_total, fpc_purge_success_total,
    reset_metrics_for_tests, CacheKeyParts, CacheNamespace, NamespaceMetrics, ResponseCache,
};
use exyonq_core::cache_purge_port::CoreCachePurgePort;
use exyonq_core::reload::{self, SharedServerState};
use exyonq_core::server::state::ServerState;
use exyonq_core::AppConfig;
use exyonq_module_api::cache_purge::{CachePurgeOp, CachePurgePort};
use exyonq_runtime_plan::stable_fpc_site_id;
use std::sync::Arc;
use std::time::Duration;

static WC3_PURGE_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn fpc_state(gen: u64, route_a: &str, route_b: &str) -> (SharedServerState, u64, u64) {
    let tmp = tempfile::tempdir().expect("tmp");
    std::fs::create_dir_all(tmp.path().join("public")).unwrap();
    std::fs::write(tmp.path().join("public/a.txt"), b"a").unwrap();
    std::fs::write(tmp.path().join("public/b.txt"), b"b").unwrap();
    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["{route_a}", "{route_b}"]

[[route]]
name = "{route_a}"
match = {{ path = "/a/" }}
root = "{root}"

[[route]]
name = "{route_b}"
match = {{ path = "/b/" }}
root = "{root}"

[full_page_cache]
enabled = true
namespace = 4
max_entries = 64
max_total_bytes = 1048576
max_object_bytes = 1048576
default_ttl_seconds = 60
max_ttl_seconds = 300
"#,
        root = tmp.path().display(),
        route_a = route_a,
        route_b = route_b,
    );
    let config: AppConfig = raw.parse().expect("config");
    let proxy = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(gen, config, proxy)
        .await
        .expect("state");
    let site_a = stable_fpc_site_id(route_a);
    let site_b = stable_fpc_site_id(route_b);
    // Keep tempdir alive via leak — tests are short-lived.
    std::mem::forget(tmp);
    (reload::wrap_state(state), site_a, site_b)
}

#[allow(clippy::too_many_arguments)] // test helper
fn insert(
    cache: &ResponseCache,
    site_id: u64,
    route_idx: usize,
    backend_id: u32,
    gen: u64,
    host: &str,
    path: &str,
    body: &'static [u8],
) {
    let key = build_storage_cache_key(CacheKeyParts {
        site_id,
        namespace: 4,
        backend_id,
        runtime_generation: gen,
        policy_generation: 0,
        route_idx,
        method: "GET".into(),
        scheme: "http".into(),
        host: host.into(),
        path: path.into(),
        query: String::new(),
        content_encoding: "identity".into(),
    });
    cache.insert_entry(
        key,
        site_id,
        route_idx,
        gen,
        Duration::from_secs(60),
        200,
        vec![("content-type".into(), "text/plain".into())],
        bytes::Bytes::from_static(body),
        None,
        CacheNamespace::new(4),
        Arc::from([]),
        NamespaceMetrics::NONE,
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn purge_site_scoped_and_url() {
    let _gate = WC3_PURGE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let (shared, site_a, site_b) = fpc_state(7, "site-a", "site-b").await;
    let state = reload::read_state(&shared);
    let cache = state.fpc_cache.as_ref().unwrap();
    let be_a = state.snapshot.route_backend_id(0).unwrap().index();
    let be_b = state.snapshot.route_backend_id(1).unwrap().index();

    insert(cache, site_a, 0, be_a, 7, "example.test", "/a/x", b"ax");
    insert(cache, site_a, 0, be_a, 7, "example.test", "/a/y", b"ay");
    insert(cache, site_b, 1, be_b, 7, "example.test", "/b/x", b"bx");
    assert_eq!(cache.snapshot().entries, 3);

    let port = CoreCachePurgePort {
        shared: SharedServerState::clone(&shared),
    };

    // Cross-tenant: unknown site rejected.
    let bad = port.purge(CachePurgeOp::Site { site_id: 999 });
    assert!(!bad.ok);
    assert_eq!(bad.error, Some("unauthorized"));
    assert!(fpc_purge_rejected_total("unauthorized") >= 1);

    // URL purge removes one entry only.
    let url = port.purge(CachePurgeOp::Url {
        site_id: site_a,
        scheme: "http".into(),
        host: "example.test".into(),
        path: "/a/x".into(),
        query: String::new(),
    });
    assert!(url.ok);
    assert_eq!(url.purged_entries, 1);
    assert_eq!(cache.snapshot().entries, 2);

    // Site purge removes remaining site_a; site_b untouched.
    let site = port.purge(CachePurgeOp::Site { site_id: site_a });
    assert!(site.ok);
    assert_eq!(site.purged_entries, 1);
    assert_eq!(cache.snapshot().entries, 1);

    let key_b = build_storage_cache_key(CacheKeyParts {
        site_id: site_b,
        namespace: 4,
        backend_id: be_b,
        runtime_generation: 7,
        policy_generation: 0,
        route_idx: 1,
        method: "GET".into(),
        scheme: "http".into(),
        host: "example.test".into(),
        path: "/b/x".into(),
        query: String::new(),
        content_encoding: "identity".into(),
    });
    assert!(cache.lookup(&key_b).is_some());
    assert!(fpc_purge_success_total() >= 2);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn purge_generation_and_tag_deferred() {
    let _gate = WC3_PURGE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let (shared, site_a, _) = fpc_state(10, "gen-a", "gen-b").await;
    let state = reload::read_state(&shared);
    let cache = state.fpc_cache.as_ref().unwrap();
    let be = state.snapshot.route_backend_id(0).unwrap().index();

    // Manually insert under gen 9 and 10 on the same store (simulates residual).
    insert(cache, site_a, 0, be, 9, "example.test", "/a/old", b"old");
    insert(cache, site_a, 0, be, 10, "example.test", "/a/new", b"new");
    assert_eq!(cache.snapshot().entries, 2);

    let port = CoreCachePurgePort {
        shared: SharedServerState::clone(&shared),
    };
    let out = port.purge(CachePurgeOp::Generation {
        site_id: site_a,
        generation: 9,
    });
    assert!(out.ok);
    assert_eq!(out.purged_entries, 1);
    assert_eq!(cache.snapshot().entries, 1);

    let tag = port.purge(CachePurgeOp::Tag {
        site_id: site_a,
        tag: "post:1".into(),
    });
    assert!(!tag.ok);
    assert_eq!(tag.error, Some("unsupported_operation"));
    assert!(fpc_purge_rejected_total("unsupported_operation") >= 1);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn purge_idempotent_and_invalid_path() {
    let _gate = WC3_PURGE_SUITE_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_metrics_for_tests();
    let (shared, site_a, _) = fpc_state(3, "idem-a", "idem-b").await;
    let port = CoreCachePurgePort {
        shared: SharedServerState::clone(&shared),
    };
    let first = port.purge(CachePurgeOp::Site { site_id: site_a });
    assert!(first.ok);
    assert_eq!(first.purged_entries, 0);
    let second = port.purge(CachePurgeOp::Site { site_id: site_a });
    assert!(second.ok);

    let bad = port.purge(CachePurgeOp::Url {
        site_id: site_a,
        scheme: "http".into(),
        host: "example.test".into(),
        path: "/a/../secret".into(),
        query: String::new(),
    });
    assert!(!bad.ok);
    assert_eq!(bad.error, Some("invalid_key"));
}
