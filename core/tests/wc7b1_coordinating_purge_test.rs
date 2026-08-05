//! WC7B1 — CoordinatingCachePurgePort: local purge authoritative, publish best-effort.

use exyonq_cache::{
    l2_invalidation_publish_failure_total, reset_metrics_for_tests, test_has, test_insert,
    DirectL1PurgePort, LocalCoordinationHub, ResponseCache,
};
use exyonq_core::coordinating_purge_port::CoordinatingCachePurgePort;
use exyonq_module_api::cache_purge::{CachePurgeOp, CachePurgePort};
use exyonq_module_api::{DistributedCacheCoordConfig, InvalidationPublisher};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn local_purge_ok_when_publish_fails() {
    reset_metrics_for_tests();
    let cfg = DistributedCacheCoordConfig {
        enabled: true,
        invalidation_enabled: true,
        generation_enabled: true,
        max_pending_events: 8,
        max_event_bytes: 8192,
        dedup_capacity: 32,
        dedup_ttl_ms: 60_000,
    };
    let hub = LocalCoordinationHub::new(cfg);
    hub.set_publish_unavailable(true);
    let site = 42u64;
    let cache = Arc::new(ResponseCache::with_limits(32, 1024 * 1024));
    let publisher = hub.open_node("n1");
    let wrap = CoordinatingCachePurgePort::new(
        Arc::new(DirectL1PurgePort::new(Arc::clone(&cache), site)),
        Some(Arc::new(publisher) as Arc<dyn InvalidationPublisher>),
        "n1",
        true,
    );
    test_insert(&cache, site, 0, 1, 1, ("ex.test", "/p"), b"p");
    let out = wrap.purge(CachePurgeOp::Url {
        site_id: site,
        scheme: "http".into(),
        host: "ex.test".into(),
        path: "/p".into(),
        query: String::new(),
    });
    assert!(out.ok, "local purge must succeed");
    assert!(!test_has(&cache, site, 0, 1, 1, "ex.test", "/p"));
    assert!(l2_invalidation_publish_failure_total() >= 1);
}

#[test]
fn publish_reaches_peer_subscriber() {
    reset_metrics_for_tests();
    let cfg = DistributedCacheCoordConfig {
        enabled: true,
        invalidation_enabled: true,
        max_pending_events: 16,
        ..Default::default()
    };
    let hub = LocalCoordinationHub::new(cfg);
    let site = 7u64;
    let cache_a = Arc::new(ResponseCache::with_limits(32, 1024 * 1024));
    let cache_b = Arc::new(ResponseCache::with_limits(32, 1024 * 1024));
    let pub_node = hub.open_node("a");
    let sub_node = hub.open_node("b");
    sub_node.start_subscriber().unwrap();
    test_insert(&cache_b, site, 0, 1, 1, ("ex.test", "/z"), b"z");

    let wrap = CoordinatingCachePurgePort::new(
        Arc::new(DirectL1PurgePort::new(Arc::clone(&cache_a), site)),
        Some(Arc::new(pub_node) as Arc<dyn InvalidationPublisher>),
        "a",
        true,
    );
    test_insert(&cache_a, site, 0, 1, 1, ("ex.test", "/z"), b"z");
    assert!(
        wrap.purge(CachePurgeOp::Url {
            site_id: site,
            scheme: "http".into(),
            host: "ex.test".into(),
            path: "/z".into(),
            query: String::new(),
        })
        .ok
    );

    let ev = exyonq_cache::recv_timeout(&sub_node, Duration::from_millis(500))
        .unwrap()
        .expect("peer event");
    let ev = sub_node.accept_received(ev, true).unwrap().unwrap();
    let port_b = DirectL1PurgePort::new(Arc::clone(&cache_b), site);
    assert!(
        port_b
            .purge(exyonq_module_api::invalidation_event_to_purge_op(&ev).unwrap())
            .ok
    );
    assert!(!test_has(&cache_b, site, 0, 1, 1, "ex.test", "/z"));
}
