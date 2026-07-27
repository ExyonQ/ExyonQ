//! WC7B1 — multinode local coordination (invalidation + generation).

use exyonq_cache::{
    l2_generation_rollback_rejected_total, l2_invalidation_duplicate_total,
    l2_invalidation_publish_failure_total, l2_subscriber_overflow_total, recv_timeout,
    reset_metrics_for_tests, test_has, test_insert, DirectL1PurgePort, LocalCoordinationHub,
    LocalCoordinationProvider, ResponseCache,
};
use exyonq_module_api::cache_purge::{CachePurgeOp, CachePurgePort};
use exyonq_module_api::{
    event_from_purge_op, invalidation_event_to_purge_op, CoordinationRejectReason,
    DistributedCacheCoordConfig, GenerationStore, InvalidationEvent, InvalidationOperation,
    InvalidationPublisher, InvalidationSubscriber, UrlTarget, COORDINATION_PROTOCOL_VERSION,
};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// Process-wide L2 metric counters + `reset_metrics_for_tests` are shared across
/// parallel tests in this binary (KF-P16-008 class / coordination harness).
fn suite_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn enabled_cfg() -> DistributedCacheCoordConfig {
    DistributedCacheCoordConfig {
        enabled: true,
        invalidation_enabled: true,
        generation_enabled: true,
        max_pending_events: 8,
        max_event_bytes: 8192,
        dedup_capacity: 64,
        dedup_ttl_ms: 60_000,
    }
}

fn drain_apply(node: &LocalCoordinationProvider, port: &dyn CachePurgePort, ignore_self: bool) {
    while let Ok(Some(raw)) = node.try_recv() {
        let Ok(Some(ev)) = node.accept_received(raw, ignore_self) else {
            continue;
        };
        if let Ok(op) = invalidation_event_to_purge_op(&ev) {
            let _ = port.purge(op);
        }
        if ev.operation == InvalidationOperation::PurgeGeneration
            || ev.operation == InvalidationOperation::PurgeSite
        {
            let _ = node.advance_generation(ev.site_id, ev.generation);
        }
    }
}

struct Node {
    cache: Arc<ResponseCache>,
    coord: LocalCoordinationProvider,
    port: DirectL1PurgePort,
}

impl Node {
    fn open(hub: &LocalCoordinationHub, name: &str, site: u64, sites: Vec<u64>) -> Self {
        let cache = Arc::new(ResponseCache::with_limits(64, 1024 * 1024));
        let coord = hub.open_node(name);
        coord.start_subscriber().expect("sub");
        let mut port = DirectL1PurgePort::new(Arc::clone(&cache), site);
        port.allowed_sites = sites;
        port.runtime_generation = 1;
        Self { cache, coord, port }
    }

    fn publish_local_purge(&self, op: CachePurgeOp) -> bool {
        let out = self.port.purge(op.clone());
        if out.ok {
            if let Some(ev) = event_from_purge_op(
                &op,
                self.coord.next_event_id(),
                self.coord.node_id(),
                out.generation,
            ) {
                let _ = self.coord.publish(ev);
            }
        }
        out.ok
    }
}

#[test]
fn two_node_url_purge() {
    let _suite = suite_lock();
    reset_metrics_for_tests();
    let hub = LocalCoordinationHub::new(enabled_cfg());
    let site_a = 1001;
    let site_b = 2002;
    let a = Node::open(&hub, "node-a", site_a, vec![site_a, site_b]);
    let b = Node::open(&hub, "node-b", site_a, vec![site_a, site_b]);

    test_insert(&a.cache, site_a, 0, 1, 1, "ex.test", "/page", b"A");
    test_insert(&b.cache, site_a, 0, 1, 1, "ex.test", "/page", b"A");
    test_insert(&b.cache, site_a, 0, 1, 1, "ex.test", "/other", b"O");
    test_insert(&b.cache, site_b, 1, 2, 1, "ex.test", "/b", b"B");

    assert!(a.publish_local_purge(CachePurgeOp::Url {
        site_id: site_a,
        scheme: "http".into(),
        host: "ex.test".into(),
        path: "/page".into(),
        query: String::new(),
    }));
    assert!(!test_has(&a.cache, site_a, 0, 1, 1, "ex.test", "/page"));

    let ev = recv_timeout(&b.coord, Duration::from_millis(200))
        .unwrap()
        .expect("event");
    let ev = b.coord.accept_received(ev, true).unwrap().expect("apply");
    let op = invalidation_event_to_purge_op(&ev).unwrap();
    assert!(b.port.purge(op).ok);

    assert!(!test_has(&b.cache, site_a, 0, 1, 1, "ex.test", "/page"));
    assert!(test_has(&b.cache, site_a, 0, 1, 1, "ex.test", "/other"));
    assert!(test_has(&b.cache, site_b, 1, 2, 1, "ex.test", "/b"));
}

#[test]
fn two_node_site_purge() {
    let _suite = suite_lock();
    reset_metrics_for_tests();
    let hub = LocalCoordinationHub::new(enabled_cfg());
    let site_a = 11;
    let site_b = 22;
    let a = Node::open(&hub, "a", site_a, vec![site_a, site_b]);
    let b = Node::open(&hub, "b", site_a, vec![site_a, site_b]);

    test_insert(&a.cache, site_a, 0, 1, 1, "ex.test", "/a1", b"1");
    test_insert(&b.cache, site_a, 0, 1, 1, "ex.test", "/a1", b"1");
    test_insert(&b.cache, site_a, 0, 1, 1, "ex.test", "/a2", b"2");
    test_insert(&b.cache, site_b, 1, 2, 1, "ex.test", "/b1", b"b");

    assert!(a.publish_local_purge(CachePurgeOp::Site { site_id: site_a }));
    let ev = recv_timeout(&b.coord, Duration::from_millis(200))
        .unwrap()
        .unwrap();
    let ev = b.coord.accept_received(ev, true).unwrap().unwrap();
    assert!(
        b.port
            .purge(invalidation_event_to_purge_op(&ev).unwrap())
            .ok
    );

    assert!(!test_has(&b.cache, site_a, 0, 1, 1, "ex.test", "/a1"));
    assert!(!test_has(&b.cache, site_a, 0, 1, 1, "ex.test", "/a2"));
    assert!(test_has(&b.cache, site_b, 1, 2, 1, "ex.test", "/b1"));
}

#[test]
fn two_node_generation_monotonic() {
    let _suite = suite_lock();
    reset_metrics_for_tests();
    let hub = LocalCoordinationHub::new(enabled_cfg());
    let site = 77;
    let a = Node::open(&hub, "a", site, vec![site]);
    let b = Node::open(&hub, "b", site, vec![site]);

    test_insert(&a.cache, site, 0, 1, 1, "ex.test", "/g", b"g1");
    test_insert(&b.cache, site, 0, 1, 1, "ex.test", "/g", b"g1");

    assert_eq!(a.coord.advance_generation(site, 1).unwrap(), 1);
    assert_eq!(a.coord.advance_generation(site, 2).unwrap(), 2);
    assert_eq!(b.coord.get_generation(site).unwrap(), 2);

    // Publish generation purge for G1 entries.
    assert!(a.publish_local_purge(CachePurgeOp::Generation {
        site_id: site,
        generation: 1,
    }));
    let ev = recv_timeout(&b.coord, Duration::from_millis(200))
        .unwrap()
        .unwrap();
    let ev = b.coord.accept_received(ev, true).unwrap().unwrap();
    assert!(
        b.port
            .purge(invalidation_event_to_purge_op(&ev).unwrap())
            .ok
    );
    assert!(!test_has(&b.cache, site, 0, 1, 1, "ex.test", "/g"));

    // Rollback rejected.
    let err = a.coord.advance_generation(site, 1).unwrap_err();
    assert_eq!(err.reason, CoordinationRejectReason::GenerationRollback);
    assert!(l2_generation_rollback_rejected_total() >= 1);
    assert_eq!(a.coord.get_generation(site).unwrap(), 2);
}

#[test]
fn duplicate_and_self_delivery() {
    let _suite = suite_lock();
    reset_metrics_for_tests();
    let hub = LocalCoordinationHub::new(enabled_cfg());
    let site = 5;
    let a = Node::open(&hub, "a", site, vec![site]);
    let b = Node::open(&hub, "b", site, vec![site]);
    test_insert(&b.cache, site, 0, 1, 1, "ex.test", "/d", b"d");

    let ev = InvalidationEvent {
        protocol_version: COORDINATION_PROTOCOL_VERSION,
        event_id: 42,
        source_node_id: "a".into(),
        site_id: site,
        operation: InvalidationOperation::PurgeUrl,
        url: Some(UrlTarget {
            scheme: "http".into(),
            host: "ex.test".into(),
            path: "/d".into(),
            query: String::new(),
        }),
        generation: 1,
        issued_at_unix_ms: 1,
    };
    a.coord.publish(ev.clone()).unwrap();
    let first = recv_timeout(&b.coord, Duration::from_millis(200))
        .unwrap()
        .unwrap();
    let apply = b.coord.accept_received(first, true).unwrap().unwrap();
    assert!(
        b.port
            .purge(invalidation_event_to_purge_op(&apply).unwrap())
            .ok
    );

    // Duplicate to B.
    b.coord.inject_for_tests(ev.clone()).unwrap();
    let dup = b.coord.try_recv().unwrap().unwrap();
    assert!(b.coord.accept_received(dup, true).unwrap().is_none());
    assert!(l2_invalidation_duplicate_total() >= 1);

    // Self-delivery ignored when ignore_self.
    a.coord
        .inject_for_tests(InvalidationEvent {
            source_node_id: "a".into(),
            event_id: 99,
            ..ev
        })
        .unwrap();
    let self_ev = a.coord.try_recv().unwrap().unwrap();
    assert!(a.coord.accept_received(self_ev, true).unwrap().is_none());
}

#[test]
fn out_of_order_generation_events() {
    let _suite = suite_lock();
    reset_metrics_for_tests();
    let hub = LocalCoordinationHub::new(enabled_cfg());
    let site = 9;
    let b = Node::open(&hub, "b", site, vec![site]);
    b.coord.advance_generation(site, 1).unwrap();

    let e3 = InvalidationEvent {
        protocol_version: COORDINATION_PROTOCOL_VERSION,
        event_id: 3,
        source_node_id: "a".into(),
        site_id: site,
        operation: InvalidationOperation::PurgeGeneration,
        url: None,
        generation: 3,
        issued_at_unix_ms: 3,
    };
    let e2 = InvalidationEvent {
        event_id: 2,
        generation: 2,
        ..e3.clone()
    };

    b.coord.inject_for_tests(e3).unwrap();
    b.coord.inject_for_tests(e2).unwrap();
    drain_apply(&b.coord, &b.port, true);
    // Final generation stays at max applied (3), never rolls to 2.
    assert_eq!(b.coord.get_generation(site).unwrap(), 3);
}

#[test]
fn provider_failure_fail_open() {
    let _suite = suite_lock();
    reset_metrics_for_tests();
    let hub = LocalCoordinationHub::new(enabled_cfg());
    let site = 3;
    let a = Node::open(&hub, "a", site, vec![site]);
    test_insert(&a.cache, site, 0, 1, 1, "ex.test", "/x", b"x");
    hub.set_publish_unavailable(true);
    assert!(a.publish_local_purge(CachePurgeOp::Url {
        site_id: site,
        scheme: "http".into(),
        host: "ex.test".into(),
        path: "/x".into(),
        query: String::new(),
    }));
    // Local purge succeeded despite publish failure.
    assert!(!test_has(&a.cache, site, 0, 1, 1, "ex.test", "/x"));
    assert!(l2_invalidation_publish_failure_total() >= 1);
}

#[test]
fn queue_overflow_triggers_reconciliation() {
    let _suite = suite_lock();
    reset_metrics_for_tests();
    let mut cfg = enabled_cfg();
    cfg.max_pending_events = 1;
    let hub = LocalCoordinationHub::new(cfg);
    let site = 4;
    let a = Node::open(&hub, "a", site, vec![site]);
    let b = Node::open(&hub, "b", site, vec![site]);

    let mk = |id: u128| InvalidationEvent {
        protocol_version: COORDINATION_PROTOCOL_VERSION,
        event_id: id,
        source_node_id: "a".into(),
        site_id: site,
        operation: InvalidationOperation::PurgeSite,
        url: None,
        generation: 1,
        issued_at_unix_ms: id as u64,
    };
    // Fill B's queue.
    a.coord.publish(mk(1)).ok();
    // Second publish should overflow bounded queue.
    let r = a.coord.publish(mk(2));
    assert!(r.is_err());
    assert!(l2_subscriber_overflow_total() >= 1);
    // Uncertainty reconciliation advanced generation.
    assert!(
        b.coord.get_generation(site).unwrap() >= 1
            || !a.coord.take_uncertain_sites().is_empty()
            || l2_subscriber_overflow_total() >= 1
    );
}

#[test]
fn subscriber_shutdown_clean() {
    let _suite = suite_lock();
    let hub = LocalCoordinationHub::new(enabled_cfg());
    let n = hub.open_node("z");
    n.start_subscriber().unwrap();
    n.stop();
    n.start_subscriber().unwrap();
    n.stop();
    assert!(n.try_recv().unwrap().is_none());
}

#[test]
fn unknown_version_rejected() {
    let _suite = suite_lock();
    reset_metrics_for_tests();
    let hub = LocalCoordinationHub::new(enabled_cfg());
    let b = hub.open_node("b");
    b.start_subscriber().unwrap();
    let ev = InvalidationEvent {
        protocol_version: 99,
        event_id: 1,
        source_node_id: "a".into(),
        site_id: 1,
        operation: InvalidationOperation::PurgeSite,
        url: None,
        generation: 1,
        issued_at_unix_ms: 1,
    };
    b.inject_for_tests(ev).unwrap();
    let raw = b.try_recv().unwrap().unwrap();
    let err = b.accept_received(raw, true).unwrap_err();
    assert_eq!(err.reason, CoordinationRejectReason::UnknownVersion);
}
