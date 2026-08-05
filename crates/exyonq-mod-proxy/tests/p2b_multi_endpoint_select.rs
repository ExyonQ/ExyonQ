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
//! P2B integration: multi-endpoint bind + selection + pool identity + status mapping.

use exyonq_mod_proxy::{
    endpoint_transport_identity, load_get_for_cache_by_cluster, service_unavailable, wire_conn,
    ProxyCacheLoad, ProxyRuntime,
};
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledEndpoint, ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest,
    ProxyDispatchService, ProxyMethod,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

fn multi_slot() -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: String::new(),
        timeout: Duration::from_millis(50),
        single_endpoint_executable: false,
        multi_endpoint_executable: true,
        endpoint_count: 2,
        failover_priority_bands: true,
        endpoints: Box::new([
            ProxyCompiledEndpoint {
                endpoint_id: "a".into(),
                http_uri: "http://10.0.0.1:8080".into(),
                weight: 1,
                priority: 0,
                admin_enabled: true,
            },
            ProxyCompiledEndpoint {
                endpoint_id: "b".into(),
                http_uri: "http://10.0.0.2:8080".into(),
                weight: 1,
                priority: 0,
                admin_enabled: true,
            },
        ]),
    }
}

fn get_req(cluster_id: u32) -> ProxyDispatchRequest {
    ProxyDispatchRequest {
        cluster_id,
        method: ProxyMethod::Get,
        path_and_query: "/".into(),
        host: None,
        headers: vec![],
        body: None,
        remote_addr: "127.0.0.1".into(),
        scheme: "http".into(),
    }
}

#[tokio::test]
async fn multi_endpoint_selects_without_first_fallback_and_returns_when_unbound() {
    let rt = Arc::new(ProxyRuntime::new());
    let outcome = rt.dispatch(get_req(0)).await;
    assert!(matches!(outcome, ProxyDispatchOutcome::ServiceUnavailable));

    rt.bind_compiled_slots(1, &[multi_slot()]);
    let mut keys = HashSet::new();
    for _ in 0..20 {
        let t = rt.upstream_for_cluster(0).expect("selected");
        let uri = t.uri_for("/");
        let auth = format!(
            "http://{}:{}",
            uri.host().unwrap(),
            uri.port_u16().unwrap_or(80)
        );
        keys.insert(auth);
    }
    assert!(
        keys.len() >= 2,
        "expected both endpoints over repeated selection, got {keys:?}"
    );
}

#[test]
fn pool_key_differs_per_endpoint_same_backend() {
    let a = endpoint_transport_identity("http://10.0.0.1:8080").unwrap();
    let b = endpoint_transport_identity("http://10.0.0.2:8080").unwrap();
    assert_ne!(a, b);
    assert_eq!(a, "http://10.0.0.1:8080");
    // POOL_FUTURE_TLS_IDENTITY_GAP = DOCUMENTED (scheme/port separable; SNI not yet a key dim)
    let tls = endpoint_transport_identity("https://10.0.0.1:8443").unwrap();
    assert_ne!(a, tls);
}

#[tokio::test]
async fn empty_multi_binding_is_service_unavailable() {
    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(
        1,
        &[ProxyCompiledSlot {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: String::new(),
            timeout: Duration::from_millis(50),
            single_endpoint_executable: false,
            multi_endpoint_executable: false,
            endpoint_count: 0,
            failover_priority_bands: true,
            endpoints: Box::new([]),
        }],
    );
    let outcome = rt.dispatch(get_req(0)).await;
    assert!(matches!(outcome, ProxyDispatchOutcome::ServiceUnavailable));
}

#[test]
fn generation_rebuild_drops_removed_endpoint() {
    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(1, &[multi_slot()]);
    let only_b_single = ProxyCompiledSlot::legacy_single(
        0,
        "backend",
        "http://10.0.0.2:8080",
        Duration::from_millis(50),
    );
    rt.bind_compiled_slots(2, &[only_b_single]);
    for _ in 0..10 {
        let t = rt.upstream_for_cluster(0).unwrap();
        assert_eq!(t.uri_for("/").host(), Some("10.0.0.2"));
    }
}

#[test]
fn concurrent_selection_across_two_backends_no_shared_lock() {
    // SELECTOR_LOCK_SCOPE = SELECTION_STATE_ONLY; CROSS_BACKEND_LOCK_CONTENTION = NO_BY_DESIGN_AND_TEST
    let rt = Arc::new(ProxyRuntime::new());
    let alpha = ProxyCompiledSlot::legacy_single(
        0,
        "alpha",
        "http://10.0.0.1:8080",
        Duration::from_millis(50),
    );
    let mut beta = multi_slot();
    beta.cluster_id = 1;
    beta.upstream_name = "beta".into();
    rt.bind_compiled_slots(1, &[alpha, beta]);

    let mut handles = Vec::new();
    for cluster_id in [0u32, 1u32] {
        for _ in 0..4 {
            let rt = Arc::clone(&rt);
            handles.push(std::thread::spawn(move || {
                for _ in 0..1_000 {
                    assert!(rt.upstream_for_cluster(cluster_id).is_some());
                }
            }));
        }
    }
    for h in handles {
        h.join().expect("thread");
    }
}

#[test]
fn reload_adds_endpoint_and_weight_change() {
    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(
        1,
        &[ProxyCompiledSlot::legacy_single(
            0,
            "backend",
            "http://10.0.0.1:8080",
            Duration::from_millis(50),
        )],
    );
    assert_eq!(
        rt.upstream_for_cluster(0).unwrap().uri_for("/").host(),
        Some("10.0.0.1")
    );

    let mut slot = multi_slot();
    slot.endpoints[1].weight = 10;
    rt.bind_compiled_slots(2, &[slot]);
    let mut hosts = HashSet::new();
    for _ in 0..40 {
        hosts.insert(
            rt.upstream_for_cluster(0)
                .unwrap()
                .uri_for("/")
                .host()
                .unwrap()
                .to_string(),
        );
    }
    assert!(hosts.contains("10.0.0.1") && hosts.contains("10.0.0.2"));
}

#[tokio::test]
async fn open002_configured_multi_one_eligible_binds_single_path() {
    // configured>=2, eligible==1 → Single binding (OPEN-002).
    let slot = ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: "http://10.0.0.1:8080".into(),
        timeout: Duration::from_millis(50),
        single_endpoint_executable: true,
        multi_endpoint_executable: false,
        endpoint_count: 2,
        failover_priority_bands: true,
        endpoints: Box::new([
            ProxyCompiledEndpoint {
                endpoint_id: "a".into(),
                http_uri: "http://10.0.0.1:8080".into(),
                weight: 1,
                priority: 0,
                admin_enabled: true,
            },
            ProxyCompiledEndpoint {
                endpoint_id: "b".into(),
                http_uri: "http://10.0.0.2:8080".into(),
                weight: 0,
                priority: 0,
                admin_enabled: true,
            },
        ]),
    };
    let rt = Arc::new(ProxyRuntime::new());
    rt.bind_compiled_slots(1, &[slot]);
    let mut hosts = HashSet::new();
    for _ in 0..20 {
        let t = rt.upstream_for_cluster(0).expect("selected");
        hosts.insert(t.uri_for("/").host().unwrap().to_string());
    }
    assert_eq!(
        hosts.len(),
        1,
        "effective N=1 must stay on sole eligible host"
    );
    assert!(hosts.contains("10.0.0.1"));
}

#[tokio::test]
async fn open002_multi_flag_with_one_eligible_collapses_defense_in_depth() {
    // Even if multi_endpoint_executable is wrongly set, bind collapses to Single.
    let slot = ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: String::new(),
        timeout: Duration::from_millis(50),
        single_endpoint_executable: false,
        multi_endpoint_executable: true,
        endpoint_count: 2,
        failover_priority_bands: true,
        endpoints: Box::new([
            ProxyCompiledEndpoint {
                endpoint_id: "a".into(),
                http_uri: "http://10.0.0.1:8080".into(),
                weight: 1,
                priority: 0,
                admin_enabled: true,
            },
            ProxyCompiledEndpoint {
                endpoint_id: "b".into(),
                http_uri: "http://10.0.0.2:8080".into(),
                weight: 1,
                priority: 0,
                admin_enabled: false,
            },
        ]),
    };
    let rt = Arc::new(ProxyRuntime::new());
    rt.bind_compiled_slots(1, &[slot]);
    let mut hosts = HashSet::new();
    for _ in 0..20 {
        let t = rt.upstream_for_cluster(0).expect("selected");
        hosts.insert(t.uri_for("/").host().unwrap().to_string());
    }
    assert_eq!(hosts, HashSet::from(["10.0.0.1".to_string()]));
}

#[tokio::test]
async fn selected_endpoint_connection_refused_is_bad_gateway_not_503() {
    // After selection, connect failure keeps existing 502 contract.
    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(
        1,
        &[ProxyCompiledSlot::legacy_single(
            0,
            "backend",
            "http://127.0.0.1:1",
            Duration::from_millis(200),
        )],
    );
    let outcome = rt.dispatch(get_req(0)).await;
    assert!(
        matches!(
            outcome,
            ProxyDispatchOutcome::BadGateway | ProxyDispatchOutcome::GatewayTimeout
        ),
        "expected 502/504 after selection, got {outcome:?}"
    );
    assert!(!matches!(outcome, ProxyDispatchOutcome::ServiceUnavailable));
}

#[tokio::test]
async fn selected_endpoint_timeout_is_gateway_timeout_not_503() {
    let rt = ProxyRuntime::new();
    // Blackhole / non-routable with very short timeout → 504 class.
    rt.bind_compiled_slots(
        1,
        &[ProxyCompiledSlot::legacy_single(
            0,
            "backend",
            "http://172.16.0.1:9",
            Duration::from_millis(1),
        )],
    );
    let outcome = rt.dispatch(get_req(0)).await;
    assert!(
        matches!(
            outcome,
            ProxyDispatchOutcome::GatewayTimeout | ProxyDispatchOutcome::BadGateway
        ),
        "expected timeout/connect failure after selection, got {outcome:?}"
    );
    assert!(!matches!(outcome, ProxyDispatchOutcome::ServiceUnavailable));
}

#[test]
fn websocket_and_helper_no_eligible_is_503() {
    // forward_websocket_by_cluster uses service_unavailable() when resolve yields None.
    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(
        1,
        &[ProxyCompiledSlot {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: String::new(),
            timeout: Duration::from_millis(50),
            single_endpoint_executable: false,
            multi_endpoint_executable: false,
            endpoint_count: 0,
            failover_priority_bands: true,
            endpoints: Box::new([]),
        }],
    );
    assert!(rt.upstream_for_cluster(0).is_none());
    assert_eq!(service_unavailable().status(), 503);
}

#[tokio::test]
async fn cache_load_no_eligible_passthrough_is_503() {
    let rt = Arc::new(ProxyRuntime::new());
    rt.bind_compiled_slots(
        1,
        &[ProxyCompiledSlot {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: String::new(),
            timeout: Duration::from_millis(50),
            single_endpoint_executable: false,
            multi_endpoint_executable: false,
            endpoint_count: 0,
            failover_priority_bands: true,
            endpoints: Box::new([]),
        }],
    );
    wire_conn::pin_runtime(Arc::clone(&rt));
    let load = load_get_for_cache_by_cluster(0, "/", None, 1024, &[]).await;
    match load {
        ProxyCacheLoad::Passthrough { response, .. } => {
            assert_eq!(response.status(), 503);
        }
        _ => panic!("expected passthrough 503 for no-eligible cache load"),
    }
}
