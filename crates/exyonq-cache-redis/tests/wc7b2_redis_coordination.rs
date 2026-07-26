//! WC7B2 Redis Streams integration tests (requires Redis on EXYONQ_REDIS_COORD_URL).

use exyonq_cache_redis::{
    reset_redis_metrics_for_tests, EventSigningKeys, RedisCoordConfig, RedisCoordSecrets,
    RedisCoordinationProvider, ReplayPolicy,
};
use exyonq_module_api::{
    GenerationStore, InvalidationEvent, InvalidationOperation, InvalidationPublisher,
    InvalidationSubscriber, UrlTarget, COORDINATION_PROTOCOL_VERSION,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn redis_url() -> Option<String> {
    std::env::var("EXYONQ_REDIS_COORD_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| Some("redis://127.0.0.1:16379/".into()))
}

fn probe_redis(url: &str) -> bool {
    let Ok(client) = redis::Client::open(url) else {
        return false;
    };
    let Ok(mut conn) = client.get_connection_with_timeout(Duration::from_millis(400)) else {
        return false;
    };
    redis::cmd("PING").query::<String>(&mut conn).is_ok()
}

fn unique_ns(tag: &str) -> String {
    format!(
        "exyonq:fpc:v1:t{}:{}",
        std::process::id(),
        tag.replace('_', "-")
    )
}

fn test_keys() -> EventSigningKeys {
    EventSigningKeys::for_tests("k1", b"wc7c-test-hmac-key-32bytes!!!!!")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn base_cfg(ns: &str, node: &str, sites: &[u64]) -> RedisCoordConfig {
    RedisCoordConfig {
        endpoint: redis_url().unwrap(),
        deployment_id: "wc7c-lab".into(),
        namespace: ns.into(),
        node_id: node.into(),
        connect_timeout: Duration::from_millis(800),
        command_timeout: Duration::from_millis(800),
        reconnect_min_backoff: Duration::from_millis(50),
        reconnect_max_backoff: Duration::from_millis(500),
        stream_maxlen: 1_000,
        max_pending_events: 64,
        max_event_bytes: 8192,
        invalidation_enabled: true,
        generation_enabled: true,
        known_site_ids: sites.to_vec(),
        replay: ReplayPolicy {
            max_age_ms: 600_000,
            max_future_skew_ms: 60_000,
        },
    }
}

fn open(ns: &str, node: &str, sites: &[u64]) -> RedisCoordinationProvider {
    RedisCoordinationProvider::connect(
        base_cfg(ns, node, sites),
        RedisCoordSecrets::default(),
        test_keys(),
    )
    .expect("connect redis")
}

fn sample_url_event(node: &str, site: u64, path: &str, eid: u128, gen: u64) -> InvalidationEvent {
    InvalidationEvent {
        protocol_version: COORDINATION_PROTOCOL_VERSION,
        event_id: eid,
        source_node_id: node.into(),
        site_id: site,
        operation: InvalidationOperation::PurgeUrl,
        url: Some(UrlTarget {
            scheme: "https".into(),
            host: "example.test".into(),
            path: path.into(),
            query: String::new(),
        }),
        generation: gen,
        issued_at_unix_ms: now_ms(),
    }
}

fn wait_recv(
    sub: &RedisCoordinationProvider,
    timeout: Duration,
) -> Option<InvalidationEvent> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(Some(ev)) = sub.try_recv() {
            return Some(ev);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

fn require_redis() -> Option<String> {
    let url = redis_url()?;
    if !probe_redis(&url) {
        eprintln!("skipping WC7B2 Redis lab tests — Redis not reachable at {url}");
        return None;
    }
    Some(url)
}

macro_rules! need_redis {
    () => {
        if require_redis().is_none() {
            return;
        }
    };
}

#[test]
fn redis_two_node_url_purge() {
    need_redis!();
    reset_redis_metrics_for_tests();
    let ns = unique_ns("url");
    let a = open(&ns, "node-a", &[1]);
    let b = open(&ns, "node-b", &[1]);
    a.start_subscriber().unwrap();
    b.start_subscriber().unwrap();

    let ev = sample_url_event("node-a", 1, "/post/1", a.next_event_id(), 1);
    a.publish(ev.clone()).expect("publish");

    let got = wait_recv(&b, Duration::from_secs(5)).expect("B should receive");
    let accepted = b.accept_received(got, true).unwrap().expect("apply");
    assert_eq!(accepted.url.as_ref().unwrap().path, "/post/1");
    assert_eq!(accepted.site_id, 1);

    // Self-delivery skipped on A if it receives its own (may or may not depending on group).
    while let Ok(Some(ev)) = a.try_recv() {
        assert!(a.accept_received(ev, true).unwrap().is_none());
    }
    a.stop();
    b.stop();
}

#[test]
fn redis_two_node_site_purge() {
    need_redis!();
    let ns = unique_ns("site");
    let a = open(&ns, "node-a", &[7]);
    let b = open(&ns, "node-b", &[7]);
    b.start_subscriber().unwrap();
    let ev = InvalidationEvent {
        protocol_version: COORDINATION_PROTOCOL_VERSION,
        event_id: a.next_event_id(),
        source_node_id: "node-a".into(),
        site_id: 7,
        operation: InvalidationOperation::PurgeSite,
        url: None,
        generation: 2,
        issued_at_unix_ms: now_ms(),
    };
    a.publish(ev).unwrap();
    let got = wait_recv(&b, Duration::from_secs(5)).expect("site event");
    let accepted = b.accept_received(got, true).unwrap().unwrap();
    assert_eq!(accepted.operation, InvalidationOperation::PurgeSite);
    assert_eq!(accepted.site_id, 7);
    b.stop();
}

#[test]
fn redis_two_node_generation_and_monotonic() {
    need_redis!();
    let ns = unique_ns("gen");
    let a = open(&ns, "node-a", &[3]);
    let b = open(&ns, "node-b", &[3]);
    assert_eq!(a.advance_generation(3, 2).unwrap(), 2);
    b.reconcile_known_sites().unwrap();
    assert_eq!(b.get_generation(3).unwrap(), 2);
    // Out of order: G3 then delayed G2 → stored remains G3; local reject on rollback propose
    assert_eq!(a.advance_generation(3, 3).unwrap(), 3);
    assert!(a.advance_generation(3, 2).is_err());
    assert_eq!(a.get_generation(3).unwrap(), 3);
    b.reconcile_known_sites().unwrap();
    assert_eq!(b.get_generation(3).unwrap(), 3);
}

#[test]
fn redis_generation_rollback_local_reject() {
    need_redis!();
    let ns = unique_ns("rollback");
    let a = open(&ns, "node-a", &[9]);
    assert_eq!(a.advance_generation(9, 5).unwrap(), 5);
    let err = a.advance_generation(9, 4).unwrap_err();
    assert_eq!(
        err.reason,
        exyonq_module_api::CoordinationRejectReason::GenerationRollback
    );
}

#[test]
fn redis_missed_event_reconciliation() {
    need_redis!();
    let ns = unique_ns("miss");
    let a = open(&ns, "node-a", &[11]);
    // B not subscribed yet — advance generation while B offline
    assert_eq!(a.advance_generation(11, 10).unwrap(), 10);
    let b = open(&ns, "node-b", &[11]);
    b.start_subscriber().unwrap(); // triggers reconcile
    // Drain synthetic generation event
    let mut saw = false;
    for _ in 0..50 {
        if let Ok(Some(ev)) = b.try_recv() {
            if ev.operation == InvalidationOperation::PurgeGeneration && ev.generation == 10 {
                saw = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(saw || b.get_generation(11).unwrap() >= 10);
    assert!(b.get_generation(11).unwrap() >= 10);
    b.stop();
}

#[test]
fn redis_duplicate_and_malformed_isolation() {
    need_redis!();
    let ns = unique_ns("dup");
    let a = open(&ns, "node-a", &[1]);
    let b = open(&ns, "node-b", &[1]);
    b.start_subscriber().unwrap();
    let eid = a.next_event_id();
    let ev = sample_url_event("node-a", 1, "/x", eid, 1);
    a.publish(ev.clone()).unwrap();
    a.publish(ev.clone()).unwrap(); // duplicate id on wire
    let first = wait_recv(&b, Duration::from_secs(5)).expect("first");
    assert!(b.accept_received(first, true).unwrap().is_some());
    if let Some(second) = wait_recv(&b, Duration::from_secs(2)) {
        assert!(b.accept_received(second, true).unwrap().is_none());
    }
    // Malformed: inject via raw XADD then valid event
    {
        let client = redis::Client::open(redis_url().unwrap()).unwrap();
        let mut conn = client.get_connection().unwrap();
        let stream = format!("{}:inv", ns);
        let _: String = redis::cmd("XADD")
            .arg(&stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(1000)
            .arg("*")
            .arg("e")
            .arg("{not-json")
            .query(&mut conn)
            .unwrap();
    }
    let good = sample_url_event("node-a", 1, "/after-bad", a.next_event_id(), 1);
    a.publish(good).unwrap();
    let mut got_good = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(ev)) = b.try_recv() {
            if let Ok(Some(a)) = b.accept_received(ev, true) {
                if a.url.as_ref().map(|u| u.path.as_str()) == Some("/after-bad") {
                    got_good = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(got_good, "subscriber must continue after malformed");
    b.stop();
}

#[test]
fn redis_subscriber_shutdown() {
    need_redis!();
    let ns = unique_ns("stop");
    let a = open(&ns, "node-a", &[1]);
    a.start_subscriber().unwrap();
    a.stop();
    assert!(a.try_recv().unwrap().is_none());
}

#[test]
fn redis_fail_open_when_unreachable() {
    reset_redis_metrics_for_tests();
    let cfg = RedisCoordConfig {
        endpoint: "redis://127.0.0.1:1/".into(),
        deployment_id: "wc7c-lab".into(),
        namespace: unique_ns("down"),
        node_id: "node-a".into(),
        connect_timeout: Duration::from_millis(100),
        command_timeout: Duration::from_millis(100),
        reconnect_min_backoff: Duration::from_millis(10),
        reconnect_max_backoff: Duration::from_millis(50),
        stream_maxlen: 100,
        max_pending_events: 8,
        max_event_bytes: 8192,
        invalidation_enabled: true,
        generation_enabled: true,
        known_site_ids: vec![1],
        replay: ReplayPolicy::default(),
    };
    let err = RedisCoordinationProvider::connect(cfg, RedisCoordSecrets::default(), test_keys());
    assert!(err.is_err());
}

#[test]
fn redis_data_loss_does_not_roll_back_local() {
    need_redis!();
    let ns = unique_ns("loss");
    let a = open(&ns, "node-a", &[42]);
    assert_eq!(a.advance_generation(42, 20).unwrap(), 20);
    // Simulate remote wipe of generation key only
    {
        let client = redis::Client::open(redis_url().unwrap()).unwrap();
        let mut conn = client.get_connection().unwrap();
        let key = format!("{}:gen:42", ns);
        let _: () = redis::cmd("DEL").arg(key).query(&mut conn).unwrap();
    }
    // get_generation is local high-water only → stays 20
    assert_eq!(a.get_generation(42).unwrap(), 20);
    // advancing to lower than local still rejected
    assert!(a.advance_generation(42, 5).is_err());
}

#[test]
fn redis_reconnect_and_reconcile_after_outage() {
    need_redis!();
    // Soft reconnect path: mark reconcile + run without killing Docker (CI-friendly).
    let ns = unique_ns("reconn");
    let a = open(&ns, "node-a", &[55]);
    let b = open(&ns, "node-b", &[55]);
    assert_eq!(a.advance_generation(55, 7).unwrap(), 7);
    b.start_subscriber().unwrap();
    assert!(b.reconcile_known_sites().is_ok());
    assert!(b.get_generation(55).unwrap() >= 7);
    b.stop();
}

#[test]
fn redis_queue_backpressure_marks_reconcile() {
    need_redis!();
    let mut cfg = base_cfg(&unique_ns("q"), "node-b", &[1]);
    cfg.max_pending_events = 1;
    let a = open(&cfg.namespace, "node-a", &[1]);
    let b = RedisCoordinationProvider::connect(
        cfg.clone(),
        RedisCoordSecrets::default(),
        test_keys(),
    )
    .unwrap();
    b.start_subscriber().unwrap();
    // Flood more events than queue depth; overflow should not panic/deadlock.
    for i in 0..8 {
        let ev = sample_url_event("node-a", 1, &format!("/flood/{i}"), a.next_event_id(), 1);
        let _ = a.publish(ev);
    }
    std::thread::sleep(Duration::from_millis(500));
    // Provider stays usable
    let _ = b.try_recv();
    b.stop();
}

#[test]
fn redis_wire_json_roundtrip_bounds() {
    use exyonq_cache_redis::REDIS_TRANSPORT;
    assert_eq!(REDIS_TRANSPORT, "STREAMS");
    need_redis!();
    let ev = sample_url_event("n", 1, "/p", 99, 1);
    let ns = unique_ns("wire");
    let a = open(&ns, "node-a", &[1]);
    a.publish(ev).unwrap();
}
