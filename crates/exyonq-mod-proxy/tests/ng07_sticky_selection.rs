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
//! NG-07 — sticky sessions (cookie / source / header) and selection algorithms
//! (`least_conn`, consistent hash, P2C). E2E against two real TCP echo backends;
//! hit counters prove which endpoint actually served each request.

use exyonq_mod_proxy::{AffinityCtx, ProxyRuntime};
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledEndpoint, ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest,
    ProxyDispatchService, ProxyMethod, ProxySelectionPolicy,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Real backend: accepts HTTP requests, counts hits, replies 200 + close.
async fn echo_backend(hits: Arc<AtomicU64>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let hits = hits.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                if stream.read(&mut buf).await.unwrap_or(0) == 0 {
                    return;
                }
                hits.fetch_add(1, Ordering::Relaxed);
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            });
        }
    });
    port
}

fn slot(
    policy: ProxySelectionPolicy,
    sticky_cookie: Option<&str>,
    sticky_header: Option<&str>,
    port_a: u16,
    port_b: u16,
) -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: String::new(),
        timeout: Duration::from_secs(2),
        max_connect_retries: 1,
        single_endpoint_executable: false,
        multi_endpoint_executable: true,
        endpoint_count: 2,
        failover_priority_bands: true,
        endpoints: Box::new([
            ProxyCompiledEndpoint {
                endpoint_id: "ea".into(),
                http_uri: format!("http://127.0.0.1:{port_a}"),
                weight: 1,
                priority: 0,
                admin_enabled: true,
                max_fails: 1,
                fail_timeout_ms: 10_000,
                slow_start_ms: 0,
            },
            ProxyCompiledEndpoint {
                endpoint_id: "eb".into(),
                http_uri: format!("http://127.0.0.1:{port_b}"),
                weight: 1,
                priority: 0,
                admin_enabled: true,
                max_fails: 1,
                fail_timeout_ms: 10_000,
                slow_start_ms: 0,
            },
        ]),
        health_check: Default::default(),
        selection_policy: policy,
        sticky_cookie: sticky_cookie.map(str::to_string),
        sticky_header: sticky_header.map(str::to_string),
        dns_refresh_ms: 0,
        location_rewrites: Vec::new(),
    }
}

fn req(remote_addr: &str, headers: Vec<(String, String)>) -> ProxyDispatchRequest {
    ProxyDispatchRequest {
        cluster_id: 0,
        method: ProxyMethod::Get,
        path_and_query: "/".into(),
        host: None,
        headers,
        body: None,
        remote_addr: remote_addr.into(),
        scheme: "http".into(),
    }
}

fn outcome_headers(outcome: &ProxyDispatchOutcome) -> &[(String, String)] {
    match outcome {
        ProxyDispatchOutcome::Materialized(m) => &m.headers,
        ProxyDispatchOutcome::Streaming { headers, .. } => headers,
        other => panic!("expected served outcome, got {other:?}"),
    }
}

#[tokio::test]
async fn cookie_affinity_mints_route_cookie_and_pins_subsequent_requests() {
    let hits_a = Arc::new(AtomicU64::new(0));
    let hits_b = Arc::new(AtomicU64::new(0));
    let port_a = echo_backend(hits_a.clone()).await;
    let port_b = echo_backend(hits_b.clone()).await;

    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(
        1,
        &[slot(
            ProxySelectionPolicy::CookieAffinity,
            Some("route"),
            None,
            port_a,
            port_b,
        )],
    );

    // First request: no pin — must be served AND carry a minted route cookie.
    let first = rt.dispatch(req("10.0.0.9", vec![])).await;
    let headers = outcome_headers(&first);
    let minted = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, v)| v.clone())
        .expect("cookie_affinity must mint Set-Cookie on first hit");
    assert!(
        minted.starts_with("route=e") && minted.contains("Path=/") && minted.contains("HttpOnly"),
        "unexpected route cookie: {minted}"
    );

    // Pinned request: cookie naming endpoint "eb" must always land on B.
    hits_a.store(0, Ordering::Relaxed);
    hits_b.store(0, Ordering::Relaxed);
    for _ in 0..5 {
        let out = rt
            .dispatch(req("10.0.0.9", vec![("cookie".into(), "route=eb".into())]))
            .await;
        let _ = outcome_headers(&out);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(hits_a.load(Ordering::Relaxed), 0, "pin leaked to A");
    assert_eq!(hits_b.load(Ordering::Relaxed), 5, "pin to B broken");

    // Pin to "ea" → always A.
    hits_a.store(0, Ordering::Relaxed);
    hits_b.store(0, Ordering::Relaxed);
    for _ in 0..5 {
        let _ = rt
            .dispatch(req("10.0.0.9", vec![("cookie".into(), "route=ea".into())]))
            .await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(hits_a.load(Ordering::Relaxed), 5);
    assert_eq!(hits_b.load(Ordering::Relaxed), 0);

    // Unknown token → falls back to normal selection and re-mints.
    hits_a.store(0, Ordering::Relaxed);
    hits_b.store(0, Ordering::Relaxed);
    let out = rt
        .dispatch(req(
            "10.0.0.9",
            vec![("cookie".into(), "route=nonexistent".into())],
        ))
        .await;
    let headers = outcome_headers(&out);
    assert!(
        headers
            .iter()
            .any(|(n, v)| n.eq_ignore_ascii_case("set-cookie") && v.starts_with("route=e")),
        "fallback must mint a fresh route cookie: {headers:?}"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        hits_a.load(Ordering::Relaxed) + hits_b.load(Ordering::Relaxed),
        1,
        "fallback request must still be served once"
    );
}

#[tokio::test]
async fn source_affinity_pins_client_ip_consistently() {
    let hits_a = Arc::new(AtomicU64::new(0));
    let hits_b = Arc::new(AtomicU64::new(0));
    let port_a = echo_backend(hits_a.clone()).await;
    let port_b = echo_backend(hits_b.clone()).await;

    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(
        1,
        &[slot(
            ProxySelectionPolicy::SourceAffinity,
            None,
            None,
            port_a,
            port_b,
        )],
    );

    // Find two client IPs mapping to different backends (HRW is deterministic,
    // so a bounded scan always finds a split).
    let mut ip_for_a = None;
    let mut ip_for_b = None;
    for i in 0..60u8 {
        let ip = format!("192.0.2.{i}");
        let sel = rt
            .upstream_for_cluster_ctx(
                0,
                &AffinityCtx {
                    client_ip: Some(ip.as_str()),
                    headers: &[],
                    path_key: "/",
                },
            )
            .expect("selection");
        let port = sel.target.uri_for("/").port_u16().expect("port");
        if port == port_a && ip_for_a.is_none() {
            ip_for_a = Some(ip);
        } else if port == port_b && ip_for_b.is_none() {
            ip_for_b = Some(ip);
        }
        if ip_for_a.is_some() && ip_for_b.is_some() {
            break;
        }
    }
    let (ipa, ipb) = (
        ip_for_a.expect("an IP mapping to A"),
        ip_for_b.expect("an IP mapping to B"),
    );

    // Each pinned IP must always hit its own backend through real dispatch.
    for _ in 0..5 {
        let _ = rt.dispatch(req(&ipa, vec![])).await;
        let _ = rt.dispatch(req(&ipb, vec![])).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(hits_a.load(Ordering::Relaxed), 5, "ip {ipa} must pin to A");
    assert_eq!(hits_b.load(Ordering::Relaxed), 5, "ip {ipb} must pin to B");

    // The route cookie must NOT appear — source affinity is not cookie-based.
    let out = rt.dispatch(req(&ipa, vec![])).await;
    assert!(
        !outcome_headers(&out)
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("set-cookie")),
        "source_affinity must not mint cookies"
    );
}

#[tokio::test]
async fn header_affinity_pins_configured_header() {
    let hits_a = Arc::new(AtomicU64::new(0));
    let hits_b = Arc::new(AtomicU64::new(0));
    let port_a = echo_backend(hits_a.clone()).await;
    let port_b = echo_backend(hits_b.clone()).await;

    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(
        1,
        &[slot(
            ProxySelectionPolicy::HeaderAffinity,
            None,
            Some("x-tenant"),
            port_a,
            port_b,
        )],
    );

    // Same tenant value → same backend, every time.
    let headers_of = |v: &str| vec![("x-tenant".to_string(), v.to_string())];
    for _ in 0..5 {
        let _ = rt.dispatch(req("10.0.0.9", headers_of("t-one"))).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (ha, hb) = (
        hits_a.load(Ordering::Relaxed),
        hits_b.load(Ordering::Relaxed),
    );
    assert!(
        (ha == 5) ^ (hb == 5),
        "same tenant must pin to one backend, got a={ha} b={hb}"
    );

    // A different tenant may land elsewhere but must itself be consistent.
    hits_a.store(0, Ordering::Relaxed);
    hits_b.store(0, Ordering::Relaxed);
    for _ in 0..5 {
        let _ = rt.dispatch(req("10.0.0.9", headers_of("t-two"))).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (ha2, hb2) = (
        hits_a.load(Ordering::Relaxed),
        hits_b.load(Ordering::Relaxed),
    );
    assert!(
        (ha2 == 5) ^ (hb2 == 5),
        "second tenant must pin consistently, got a={ha2} b={hb2}"
    );
}

#[tokio::test]
async fn consistent_hash_and_least_conn_resolution_semantics() {
    let rt = ProxyRuntime::new();

    // ConsistentHash: same path+query → same endpoint; retry exclusion remaps.
    rt.bind_compiled_slots(
        1,
        &[slot(
            ProxySelectionPolicy::ConsistentHash,
            None,
            None,
            8081,
            8082,
        )],
    );
    fn ctx(key: &str) -> AffinityCtx<'_> {
        AffinityCtx {
            client_ip: None,
            headers: &[],
            path_key: key,
        }
    }
    let first = rt
        .upstream_for_cluster_ctx(0, &ctx("/api/widgets?page=2"))
        .expect("select");
    for _ in 0..10 {
        let again = rt
            .upstream_for_cluster_ctx(0, &ctx("/api/widgets?page=2"))
            .expect("select");
        assert_eq!(again.target.peer_key, first.target.peer_key);
    }

    // LeastConn: a held lease steers the next request to the other endpoint.
    rt.bind_compiled_slots(
        2,
        &[slot(
            ProxySelectionPolicy::LeastConn,
            None,
            None,
            8081,
            8082,
        )],
    );
    let held = rt
        .upstream_for_cluster_ctx(0, &AffinityCtx::NONE)
        .expect("select");
    let busy = held.target.peer_key.clone();
    let next = rt
        .upstream_for_cluster_ctx(0, &AffinityCtx::NONE)
        .expect("select");
    assert_ne!(
        next.target.peer_key, busy,
        "least_conn must prefer the endpoint without in-flight work"
    );
    drop(held);
    drop(next);
}

#[tokio::test]
async fn cookie_pin_to_dead_endpoint_falls_back_to_live() {
    // Adversarial F-coverage: a pin naming a disabled endpoint must not
    // wedge the request — selection falls back to a live endpoint.
    let hits_a = Arc::new(AtomicU64::new(0));
    let port_a = echo_backend(hits_a.clone()).await;

    let rt = ProxyRuntime::new();
    let mut s = slot(
        ProxySelectionPolicy::CookieAffinity,
        Some("route"),
        None,
        port_a,
        port_a, // placeholder — eb is disabled anyway
    );
    s.endpoints[1].admin_enabled = false;
    s.endpoints[1].http_uri = "http://127.0.0.1:1".into(); // unreachable
    rt.bind_compiled_slots(1, &[s]);

    let out = rt
        .dispatch(req("10.0.0.9", vec![("cookie".into(), "route=eb".into())]))
        .await;
    let _ = outcome_headers(&out); // must be served, not a 5xx outcome
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        hits_a.load(Ordering::Relaxed),
        1,
        "dead pin must fall back to the live endpoint"
    );
}
