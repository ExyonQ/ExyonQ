//! WC7C — independent L1 stores + signed Redis coordination (lab topology).

use exyonq_cache::direct_purge::{test_has, test_insert, DirectL1PurgePort};
use exyonq_cache::ResponseCache;
use exyonq_cache_redis::{
    EventSigningKeys, RedisCoordConfig, RedisCoordSecrets, RedisCoordinationProvider, ReplayPolicy,
};
use exyonq_module_api::cache_purge::{CachePurgeOp, CachePurgePort};
use exyonq_module_api::{
    event_from_purge_op, invalidation_event_to_purge_op, GenerationStore, InvalidationPublisher,
    InvalidationSubscriber,
};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn redis_ok() -> bool {
    let url = std::env::var("EXYONQ_REDIS_COORD_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:16379/".into());
    let Ok(c) = redis::Client::open(url.as_str()) else {
        return false;
    };
    let Ok(mut conn) = c.get_connection_with_timeout(Duration::from_millis(300)) else {
        return false;
    };
    redis::cmd("PING").query::<String>(&mut conn).is_ok()
}

fn keys() -> EventSigningKeys {
    EventSigningKeys::for_tests("k1", b"wc7c-multinode-l1-key-32bytes!!")
}

fn open(ns: &str, node: &str) -> RedisCoordinationProvider {
    let cfg = RedisCoordConfig {
        endpoint: std::env::var("EXYONQ_REDIS_COORD_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:16379/".into()),
        deployment_id: "wc7c-l1".into(),
        namespace: ns.into(),
        node_id: node.into(),
        connect_timeout: Duration::from_millis(800),
        command_timeout: Duration::from_millis(800),
        reconnect_min_backoff: Duration::from_millis(40),
        reconnect_max_backoff: Duration::from_millis(400),
        stream_maxlen: 2000,
        max_pending_events: 64,
        max_event_bytes: 8192,
        invalidation_enabled: true,
        generation_enabled: true,
        known_site_ids: vec![1],
        replay: ReplayPolicy {
            max_age_ms: 600_000,
            max_future_skew_ms: 60_000,
        },
    };
    RedisCoordinationProvider::connect(cfg, RedisCoordSecrets::default(), keys()).unwrap()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[test]
fn wc7c_independent_l1_url_purge_and_stale_window() {
    if !redis_ok() {
        eprintln!("skip: redis unavailable");
        return;
    }
    let ns = format!("exyonq:fpc:v1:l1:{}", std::process::id());
    let store_a = Arc::new(ResponseCache::new());
    let store_b = Arc::new(ResponseCache::new());
    assert!(!Arc::ptr_eq(&store_a, &store_b));
    // Seed both L1s so URL purge hits (product contract: zero-entry URL purge → not_found).
    // test_insert uses scheme "http" — purge op scheme must match.
    test_insert(&store_a, 1, 0, 1, 1, ("ex.test", "/page"), b"seed-a");
    test_insert(&store_b, 1, 0, 1, 1, ("ex.test", "/page"), b"seed-b");
    let port_a = DirectL1PurgePort::new(store_a.clone(), 1);
    let port_b = DirectL1PurgePort::new(store_b.clone(), 1);

    let a = open(&ns, "node-a");
    let b = open(&ns, "node-b");
    b.start_subscriber().unwrap();

    let op = CachePurgeOp::Url {
        site_id: 1,
        scheme: "http".into(),
        host: "ex.test".into(),
        path: "/page".into(),
        query: String::new(),
    };
    assert!(port_a.purge(op.clone()).ok, "seeded URL must purge on A");
    assert!(!test_has(&store_a, 1, 0, 1, 1, "ex.test", "/page"));
    assert!(test_has(&store_b, 1, 0, 1, 1, "ex.test", "/page"));
    let t0 = Instant::now();
    let mut ev = event_from_purge_op(&op, a.next_event_id(), "node-a", 1).unwrap();
    ev.issued_at_unix_ms = now_ms();
    a.publish(ev).unwrap();

    let mut invalidated = false;
    let mut windows = Vec::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(got)) = b.try_recv() {
            if let Ok(Some(apply)) = b.accept_received(got, true) {
                if let Ok(pop) = invalidation_event_to_purge_op(&apply) {
                    assert!(port_b.purge(pop).ok);
                    windows.push(t0.elapsed().as_millis() as u64);
                    invalidated = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(invalidated, "B must apply signed purge");
    let stale_ms = windows[0];
    eprintln!("WC7C_STALE_WINDOW_MS={stale_ms}");
    assert!(stale_ms < 5_000);

    assert_eq!(a.advance_generation(1, 5).unwrap(), 5);
    b.reconcile_known_sites().unwrap();
    assert!(b.get_generation(1).unwrap() >= 5);
    b.stop();
}

#[test]
fn wc7c_happy_path_repeat_10x() {
    if !redis_ok() {
        return;
    }
    for i in 0..10 {
        let ns = format!("exyonq:fpc:v1:rep{}:{}", i, std::process::id());
        let a = open(&ns, "node-a");
        let b = open(&ns, "node-b");
        b.start_subscriber().unwrap();
        let op = CachePurgeOp::Site { site_id: 1 };
        let mut ev = event_from_purge_op(&op, a.next_event_id(), "node-a", i as u64 + 1).unwrap();
        ev.issued_at_unix_ms = now_ms();
        a.publish(ev).unwrap();
        let mut ok = false;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(4) {
            if let Ok(Some(got)) = b.try_recv() {
                if b.accept_received(got, true).ok().flatten().is_some() {
                    ok = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ok, "repeat {i} failed");
        b.stop();
    }
}
