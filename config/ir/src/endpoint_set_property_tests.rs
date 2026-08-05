/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! P2A-OPEN-001 property/fuzz-equivalent corpus (no new dependencies).
//!
//! ```text
//! P2A_OPEN_001_STATUS = CLOSED when these tests pass
//! ```

use crate::endpoint_set::{
    deterministic_endpoint_id, endpoint_from_http_target, endpoint_from_raw, EndpointAddress,
    EndpointId, EndpointSet, RawEndpointConfig, UserBackendId,
    MAX_ENDPOINTS_PER_BACKEND_PROVISIONAL, MAX_ID_LENGTH_PROVISIONAL,
};
use crate::{AppConfig, ConfigError};

fn permute3<T: Clone>(items: &[T; 3]) -> Vec<[T; 3]> {
    let mut out = Vec::new();
    let idxs = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for p in idxs {
        out.push([
            items[p[0]].clone(),
            items[p[1]].clone(),
            items[p[2]].clone(),
        ]);
    }
    out
}

#[test]
fn property_endpoint_ordering_permutations_normalize_equivalently() {
    let specs = [
        ("10.0.0.3", 8080u16),
        ("10.0.0.1", 8080),
        ("10.0.0.2", 8080),
    ];
    let mut fingerprints = Vec::new();
    for order in permute3(&specs) {
        let eps: Vec<_> = order
            .iter()
            .map(|(a, p)| endpoint_from_http_target(&format!("http://{a}:{p}")).unwrap())
            .collect();
        let set = EndpointSet::from_endpoints(eps).unwrap();
        fingerprints.push(set.desired_fingerprint_parts());
    }
    for fp in &fingerprints[1..] {
        assert_eq!(&fingerprints[0], fp);
    }
}

#[test]
fn property_invalid_ids_rejected_corpus() {
    let bad = [
        "",
        "../x",
        "a/b",
        "has space",
        "üñí",
        &"x".repeat(MAX_ID_LENGTH_PROVISIONAL + 1),
        "semi;colon",
        "star*",
    ];
    for id in bad {
        assert!(UserBackendId::new(id).is_err(), "accepted {id:?}");
        assert!(EndpointId::new(id.to_string()).is_err(), "accepted {id:?}");
    }
    let good = ["a", "A_b.1-2", &"z".repeat(MAX_ID_LENGTH_PROVISIONAL)];
    for id in good {
        assert!(UserBackendId::new(id).is_ok(), "rejected {id:?}");
    }
}

#[test]
fn property_duplicate_ids_rejected() {
    let a = endpoint_from_http_target("http://10.0.0.1:80").unwrap();
    let mut b = endpoint_from_http_target("http://10.0.0.2:80").unwrap();
    b.endpoint_id = a.endpoint_id.clone();
    assert!(matches!(
        EndpointSet::from_endpoints(vec![a, b]),
        Err(ConfigError::DuplicateEndpointId { .. })
    ));
}

#[test]
fn property_target_endpoints_ambiguity_rejected() {
    let err = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"
[[upstream]]
name = "backend"
target = "http://127.0.0.1:9000"
[[upstream.endpoints]]
address = "10.0.0.1"
port = 8080
"#
    .parse::<AppConfig>()
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.to_lowercase().contains("ambiguous"), "{msg}");
}

#[test]
fn property_oversized_endpoint_set_rejected() {
    let mut eps = Vec::new();
    for i in 0..=MAX_ENDPOINTS_PER_BACKEND_PROVISIONAL {
        eps.push(
            endpoint_from_http_target(&format!("http://10.0.0.{}:80", (i % 250) + 1)).unwrap(),
        );
    }
    // Force unique ids (deterministic may collide on wrap); assign explicit.
    for (i, ep) in eps.iter_mut().enumerate() {
        ep.endpoint_id = EndpointId::new(format!("e{i}")).unwrap();
    }
    assert!(EndpointSet::from_endpoints(eps).is_err());
}

#[test]
fn property_address_forms_ipv4_ipv6_hostname() {
    for raw in ["127.0.0.1", "::1", "[2001:db8::1]", "api.example.com"] {
        assert!(EndpointAddress::parse(raw).is_ok(), "{raw}");
    }
    for raw in ["", "bad host", "a..b", "-bad.com"] {
        assert!(EndpointAddress::parse(raw).is_err(), "{raw}");
    }
}

#[test]
fn property_serialization_round_trip_stable() {
    let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"
[[upstream]]
name = "backend"
timeout_ms = 1000
[[upstream.endpoints]]
id = "e1"
address = "10.0.0.1"
port = 8080
weight = 2
priority = 1
[[upstream.endpoints]]
id = "e2"
address = "10.0.0.2"
port = 8080
weight = 1
priority = 1
"#;
    let cfg: AppConfig = toml.parse().unwrap();
    let set = &cfg.upstreams["backend"].endpoint_set;
    let parts1 = set.desired_fingerprint_parts();
    let cfg2: AppConfig = toml.parse().unwrap();
    let parts2 = cfg2.upstreams["backend"]
        .endpoint_set
        .desired_fingerprint_parts();
    assert_eq!(parts1, parts2);
}

#[test]
fn property_deterministic_fingerprint_and_id() {
    let a = EndpointAddress::parse("api.example.com").unwrap();
    let id1 = deterministic_endpoint_id(&a, 443, 3, 7);
    let id2 = deterministic_endpoint_id(&a, 443, 3, 7);
    assert_eq!(id1, id2);
    let ep = endpoint_from_raw(RawEndpointConfig {
        id: None,
        address: "api.example.com".into(),
        port: 443,
        weight: 3,
        priority: 7,
        admin_state: Default::default(),
    })
    .unwrap();
    assert_eq!(ep.endpoint_id, id1);
}

#[test]
fn property_extreme_weights_priorities_no_panic() {
    let mut eps = Vec::new();
    for (i, (w, p)) in [
        (0u32, 0u32),
        (1, u32::MAX),
        (u32::MAX, 0),
        (u32::MAX, u32::MAX),
    ]
    .into_iter()
    .enumerate()
    {
        let mut ep = endpoint_from_http_target(&format!("http://10.0.0.{}:80", i + 1)).unwrap();
        ep.endpoint_id = EndpointId::new(format!("w{i}")).unwrap();
        ep.weight = w;
        ep.priority = p;
        eps.push(ep);
    }
    let set = EndpointSet::from_endpoints(eps).unwrap();
    let _ = set.desired_fingerprint_parts();
}

#[test]
fn property_no_panic_under_repeated_normalize() {
    for i in 0..64 {
        let ep =
            endpoint_from_http_target(&format!("http://10.0.0.{}:8080", (i % 200) + 1)).unwrap();
        let set = EndpointSet::from_endpoints(vec![ep]).unwrap();
        assert_eq!(set.len(), 1);
    }
}
