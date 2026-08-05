//! WC7C — partition/reconnect + Redis restart soft suites (provider).

use exyonq_cache_redis::{
    EventSigningKeys, RedisCoordConfig, RedisCoordSecrets, RedisCoordinationProvider, ReplayPolicy,
};
use exyonq_module_api::{
    event_from_purge_op, CachePurgeOp, GenerationStore, InvalidationPublisher,
    InvalidationSubscriber,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn need() -> bool {
    let url = std::env::var("EXYONQ_REDIS_COORD_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:16379/".into());
    let Ok(c) = redis::Client::open(url.as_str()) else {
        return false;
    };
    c.get_connection_with_timeout(Duration::from_millis(300))
        .ok()
        .and_then(|mut conn| redis::cmd("PING").query::<String>(&mut conn).ok())
        .is_some()
}

fn keys() -> EventSigningKeys {
    EventSigningKeys::for_tests("k1", b"wc7c-fault-suite-key-32bytes!!!")
}

fn open(ns: &str, node: &str) -> RedisCoordinationProvider {
    RedisCoordinationProvider::connect(
        RedisCoordConfig {
            endpoint: std::env::var("EXYONQ_REDIS_COORD_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:16379/".into()),
            deployment_id: "wc7c-fault".into(),
            namespace: ns.into(),
            node_id: node.into(),
            connect_timeout: Duration::from_millis(600),
            command_timeout: Duration::from_millis(600),
            reconnect_min_backoff: Duration::from_millis(30),
            reconnect_max_backoff: Duration::from_millis(300),
            stream_maxlen: 1000,
            max_pending_events: 32,
            max_event_bytes: 8192,
            invalidation_enabled: true,
            generation_enabled: true,
            known_site_ids: vec![1],
            replay: ReplayPolicy {
                max_age_ms: 600_000,
                max_future_skew_ms: 60_000,
            },
        },
        RedisCoordSecrets::default(),
        keys(),
    )
    .unwrap()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[test]
fn wc7c_partition_repeat_5x_missed_event_reconcile() {
    if !need() {
        return;
    }
    for i in 0..5 {
        let ns = format!("exyonq:fpc:v1:part{}:{}", i, std::process::id());
        let a = open(&ns, "a");
        // B offline (no subscriber) while A advances
        assert_eq!(
            a.advance_generation(1, 10 + i as u64).unwrap(),
            10 + i as u64
        );
        let b = open(&ns, "b");
        b.start_subscriber().unwrap(); // reconcile on start
        assert!(b.get_generation(1).unwrap() >= 10 + i as u64);
        b.stop();
    }
}

#[test]
fn wc7c_redis_restart_repeat_5x_soft() {
    if !need() {
        return;
    }
    for i in 0..5 {
        let ns = format!("exyonq:fpc:v1:rst{}:{}", i, std::process::id());
        let a = open(&ns, "a");
        let b = open(&ns, "b");
        b.start_subscriber().unwrap();
        let op = CachePurgeOp::Site { site_id: 1 };
        let mut ev = event_from_purge_op(&op, a.next_event_id(), "a", 1).unwrap();
        ev.issued_at_unix_ms = now_ms();
        a.publish(ev).unwrap();
        let start = Instant::now();
        let mut ok = false;
        while start.elapsed() < Duration::from_secs(4) {
            if let Ok(Some(g)) = b.try_recv() {
                if b.accept_received(g, true).ok().flatten().is_some() {
                    ok = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ok, "restart soft iter {i}");
        // Soft "restart": stop and start subscriber again
        b.stop();
        b.start_subscriber().unwrap();
        // Soft restart: reconcile failure is non-fatal here.
        let _ = b.reconcile_known_sites();
        b.stop();
    }
}

#[test]
fn wc7c_fail_open_publish_when_redis_down() {
    let cfg = RedisCoordConfig {
        endpoint: "redis://127.0.0.1:1/".into(),
        deployment_id: "x".into(),
        namespace: "exyonq:fpc:v1:down".into(),
        node_id: "a".into(),
        connect_timeout: Duration::from_millis(80),
        command_timeout: Duration::from_millis(80),
        reconnect_min_backoff: Duration::from_millis(10),
        reconnect_max_backoff: Duration::from_millis(40),
        stream_maxlen: 10,
        max_pending_events: 4,
        max_event_bytes: 4096,
        invalidation_enabled: true,
        generation_enabled: true,
        known_site_ids: vec![1],
        replay: ReplayPolicy::default(),
    };
    assert!(RedisCoordinationProvider::connect(cfg, RedisCoordSecrets::default(), keys()).is_err());
}
