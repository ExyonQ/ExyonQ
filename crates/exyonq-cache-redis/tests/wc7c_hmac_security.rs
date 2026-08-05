//! WC7C — HMAC authenticity, rotation, invalid-event isolation (Redis lab).

use exyonq_cache_redis::{
    encode_signed_event, sign_mac, verify_mac, EventSigningKeys, RedisCoordConfig,
    RedisCoordSecrets, RedisCoordinationProvider, ReplayPolicy,
};
use exyonq_module_api::{
    InvalidationEvent, InvalidationOperation, InvalidationPublisher, InvalidationSubscriber,
    UrlTarget, COORDINATION_PROTOCOL_VERSION,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn redis_url() -> Option<String> {
    std::env::var("EXYONQ_REDIS_COORD_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| Some("redis://127.0.0.1:16379/".into()))
}

fn probe(url: &str) -> bool {
    let Ok(c) = redis::Client::open(url) else {
        return false;
    };
    let Ok(mut conn) = c.get_connection_with_timeout(Duration::from_millis(300)) else {
        return false;
    };
    redis::cmd("PING").query::<String>(&mut conn).is_ok()
}

fn need() -> bool {
    let Some(u) = redis_url() else {
        return false;
    };
    if !probe(&u) {
        eprintln!("skip WC7C HMAC redis tests — no Redis at {u}");
        return false;
    }
    true
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn keys() -> EventSigningKeys {
    EventSigningKeys::for_tests("k1", b"wc7c-hmac-suite-key-32bytes!!!!")
}

fn cfg(ns: &str, node: &str) -> RedisCoordConfig {
    RedisCoordConfig {
        endpoint: redis_url().unwrap(),
        deployment_id: "wc7c-secure".into(),
        namespace: ns.into(),
        node_id: node.into(),
        connect_timeout: Duration::from_millis(800),
        command_timeout: Duration::from_millis(800),
        reconnect_min_backoff: Duration::from_millis(50),
        reconnect_max_backoff: Duration::from_millis(400),
        stream_maxlen: 500,
        max_pending_events: 32,
        max_event_bytes: 8192,
        invalidation_enabled: true,
        generation_enabled: true,
        known_site_ids: vec![1],
        replay: ReplayPolicy {
            max_age_ms: 300_000,
            max_future_skew_ms: 60_000,
        },
    }
}

fn open(ns: &str, node: &str, k: EventSigningKeys) -> RedisCoordinationProvider {
    RedisCoordinationProvider::connect(cfg(ns, node), RedisCoordSecrets::default(), k)
        .expect("connect")
}

fn ev(id: u128) -> InvalidationEvent {
    InvalidationEvent {
        protocol_version: COORDINATION_PROTOCOL_VERSION,
        event_id: id,
        source_node_id: "node-a".into(),
        site_id: 1,
        operation: InvalidationOperation::PurgeUrl,
        url: Some(UrlTarget {
            scheme: "https".into(),
            host: "t.example".into(),
            path: "/p".into(),
            query: String::new(),
        }),
        generation: 1,
        issued_at_unix_ms: now_ms(),
    }
}

fn wait(sub: &RedisCoordinationProvider, t: Duration) -> Option<InvalidationEvent> {
    let start = Instant::now();
    while start.elapsed() < t {
        if let Ok(Some(e)) = sub.try_recv() {
            return Some(e);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    None
}

#[test]
fn wc7c_hmac_two_node_accepts_valid() {
    if !need() {
        return;
    }
    let ns = format!("exyonq:fpc:v1:hmac:{}", std::process::id());
    let k = keys();
    let a = open(&ns, "a", k.clone());
    let b = open(&ns, "b", k);
    b.start_subscriber().unwrap();
    a.publish(ev(a.next_event_id())).unwrap();
    assert!(wait(&b, Duration::from_secs(4)).is_some());
    b.stop();
}

#[test]
fn wc7c_invalid_hmac_isolated() {
    if !need() {
        return;
    }
    let ns = format!("exyonq:fpc:v1:badmac:{}", std::process::id());
    let k = keys();
    let a = open(&ns, "a", k.clone());
    let b = open(&ns, "b", k.clone());
    b.start_subscriber().unwrap();

    {
        let bad = ev(999);
        let (kid, _mac) = sign_mac(&bad, "wc7c-secure", &k).unwrap();
        let env = serde_json::json!({
            "key_id": kid,
            "mac": "0000000000000000000000000000000000000000000000000000000000000000",
            "event": bad,
        });
        let client = redis::Client::open(redis_url().unwrap()).unwrap();
        let mut conn = client.get_connection().unwrap();
        let stream = format!("{ns}:inv");
        let _: String = redis::cmd("XADD")
            .arg(&stream)
            .arg("*")
            .arg("e")
            .arg(env.to_string())
            .query(&mut conn)
            .unwrap();
    }

    a.publish(ev(a.next_event_id())).unwrap();
    let got = wait(&b, Duration::from_secs(5)).expect("valid after forge");
    assert_eq!(got.source_node_id, "node-a");
    b.stop();
}

#[test]
fn wc7c_key_rotation_overlap() {
    let old = EventSigningKeys::for_tests("k1", b"old-key-material-32bytes-here!!");
    let overlap = EventSigningKeys::for_tests("k2", b"new-key-material-32bytes-here!!")
        .with_previous("k1", b"old-key-material-32bytes-here!!");
    let e = ev(7);
    let (kid, mac) = sign_mac(&e, "dep", &old).unwrap();
    assert_eq!(kid, "k1");
    verify_mac(&e, "dep", &kid, &mac, &overlap).unwrap();
    let retired = EventSigningKeys::for_tests("k2", b"new-key-material-32bytes-here!!");
    assert!(verify_mac(&e, "dep", &kid, &mac, &retired,).is_err());
}

#[test]
fn wc7c_signed_envelope_contains_mac() {
    let k = keys();
    let e = ev(1);
    let s = encode_signed_event(&e, "dep", &k, 8192).unwrap();
    assert!(s.contains("\"mac\""));
    assert!(s.contains("\"key_id\""));
}
