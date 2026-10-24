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
//! NG-08 — passive upstream health (`max_fails`/`fail_timeout`/`slow_start`).
//! E2E against real TCP backends on loopback: a member that dies must stop
//! receiving traffic, must rejoin after `fail_timeout`, and must ramp
//! (`slow_start`) its share after recovery.

use exyonq_mod_proxy::{PassivePeer, ProxyRuntime};
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledEndpoint, ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest,
    ProxyDispatchService, ProxyMethod, ProxySelectionPolicy,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Real backend on an explicit port: accepts HTTP, counts hits, replies 200.
/// Returns only after the listener is bound — callers must not race dispatches
/// against an unbound socket (a refused connect is a real passive-health fail).
async fn spawn_backend(hits: Arc<AtomicU64>, port: u16) -> JoinHandle<()> {
    try_spawn_backend(hits, port).await.expect("bind backend")
}

/// Same as [`spawn_backend`] but returns `None` when the port is still busy
/// (revive-after-kill races TIME_WAIT).
async fn try_spawn_backend(hits: Arc<AtomicU64>, port: u16) -> Option<JoinHandle<()>> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.ok()?;
    Some(tokio::spawn(async move {
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
    }))
}

async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.expect("bind :0");
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

fn endpoint(
    id: &str,
    port: u16,
    max_fails: u32,
    fail_timeout_ms: u64,
    slow_start_ms: u64,
) -> ProxyCompiledEndpoint {
    ProxyCompiledEndpoint {
        endpoint_id: id.into(),
        http_uri: format!("http://127.0.0.1:{port}"),
        weight: 1,
        priority: 0,
        admin_enabled: true,
        max_fails,
        fail_timeout_ms,
        slow_start_ms,
    }
}

fn multi_slot(a: ProxyCompiledEndpoint, b: ProxyCompiledEndpoint) -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: String::new(),
        timeout: Duration::from_secs(2),
        max_connect_retries: 1,
        single_endpoint_executable: false,
        multi_endpoint_executable: true,
        endpoint_count: 2,
        failover_priority_bands: false,
        endpoints: Box::new([a, b]),
        health_check: Default::default(),
        selection_policy: ProxySelectionPolicy::WeightedRoundRobin,
        sticky_cookie: None,
        sticky_header: None,
        dns_refresh_ms: 0,
        location_rewrites: Vec::new(),
    }
}

fn single_slot(target: ProxyCompiledEndpoint) -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: target.http_uri.clone(),
        timeout: Duration::from_millis(300),
        max_connect_retries: 0,
        single_endpoint_executable: true,
        multi_endpoint_executable: false,
        endpoint_count: 1,
        failover_priority_bands: true,
        endpoints: Box::new([target]),
        health_check: Default::default(),
        selection_policy: ProxySelectionPolicy::WeightedRoundRobin,
        sticky_cookie: None,
        sticky_header: None,
        dns_refresh_ms: 0,
        location_rewrites: Vec::new(),
    }
}

fn get() -> ProxyDispatchRequest {
    ProxyDispatchRequest {
        cluster_id: 0,
        method: ProxyMethod::Get,
        path_and_query: "/".into(),
        host: None,
        headers: vec![],
        body: None,
        remote_addr: "10.0.0.9".into(),
        scheme: "http".into(),
    }
}

fn served(outcome: &ProxyDispatchOutcome) -> bool {
    matches!(
        outcome,
        ProxyDispatchOutcome::Materialized(_) | ProxyDispatchOutcome::Streaming { .. }
    )
}

/// max_fails=1 + fail_timeout: a dead member is banned out of rotation and
/// rejoins once the window passes and the backend is reachable again.
#[tokio::test]
async fn dead_backend_banned_then_recovers_after_fail_timeout() {
    let hits_a = Arc::new(AtomicU64::new(0));
    let hits_b = Arc::new(AtomicU64::new(0));
    let port_a = free_port().await;
    let port_b = free_port().await;
    let b_task = spawn_backend(hits_b.clone(), port_b).await;

    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(
        1,
        &[multi_slot(
            endpoint("ea", port_a, 1, 300, 0),
            endpoint("eb", port_b, 1, 300, 0),
        )],
    );
    let _a = spawn_backend(hits_a.clone(), port_a).await;

    // Warm: WRR reaches both members.
    for _ in 0..20 {
        rt.dispatch(get()).await;
    }
    assert!(hits_a.load(Ordering::Relaxed) > 0, "warm hits on a");
    assert!(hits_b.load(Ordering::Relaxed) > 0, "warm hits on b");

    // Kill B, then keep dispatching — the failing attempt(s) ban it and every
    // later request must be served by A (retries may hide the first failure).
    b_task.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;
    for _ in 0..10 {
        rt.dispatch(get()).await;
    }
    let frozen_b = hits_b.load(Ordering::Relaxed);
    for _ in 0..10 {
        assert!(served(&rt.dispatch(get()).await), "live member must serve");
    }
    assert_eq!(
        hits_b.load(Ordering::Relaxed),
        frozen_b,
        "banned peer must receive zero traffic during fail_timeout"
    );

    // Past fail_timeout: revive B on the same port — it must rejoin rotation.
    tokio::time::sleep(Duration::from_millis(350)).await;
    let mut revived = None;
    for _ in 0..30 {
        if let Some(t) = try_spawn_backend(hits_b.clone(), port_b).await {
            revived = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _keep = revived.expect("backend must rebind its port");
    for _ in 0..20 {
        rt.dispatch(get()).await;
    }
    assert!(
        hits_b.load(Ordering::Relaxed) > frozen_b,
        "recovered peer must rejoin rotation after fail_timeout"
    );
}

/// slow_start: after the ban expires the recovered member ramps — it must
/// receive a strictly smaller share than the healthy equal-weight peer while
/// inside the ramp window.
#[tokio::test]
async fn recovered_peer_gets_reduced_share_during_slow_start() {
    let hits_a = Arc::new(AtomicU64::new(0));
    let hits_b = Arc::new(AtomicU64::new(0));
    let port_a = free_port().await;
    let port_b = free_port().await;
    let b_task = spawn_backend(hits_b.clone(), port_b).await;

    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(
        1,
        &[multi_slot(
            endpoint("ea", port_a, 1, 200, 0),
            // B: slow 3 s ramp after recovery.
            endpoint("eb", port_b, 1, 200, 3_000),
        )],
    );
    let _a = spawn_backend(hits_a.clone(), port_a).await;
    for _ in 0..20 {
        rt.dispatch(get()).await;
    }

    // Kill → ban. Wait out fail_timeout so the next attempt starts the ramp.
    b_task.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;
    for _ in 0..10 {
        rt.dispatch(get()).await;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;

    // Revive B, then hammer requests — inside the 3 s ramp B must stay well
    // below its 50% equal-weight share.
    let mut revived = None;
    for _ in 0..30 {
        if let Some(t) = try_spawn_backend(hits_b.clone(), port_b).await {
            revived = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _keep = revived.expect("backend must rebind its port");

    let base_a = hits_a.load(Ordering::Relaxed);
    let base_b = hits_b.load(Ordering::Relaxed);
    for _ in 0..100 {
        rt.dispatch(get()).await;
    }
    let a_share = hits_a.load(Ordering::Relaxed) - base_a;
    let b_share = hits_b.load(Ordering::Relaxed) - base_b;
    assert!(
        b_share * 3 < a_share,
        "ramping peer must stay under ~25% share: a={a_share} b={b_share}"
    );
}

/// nginx: `max_fails`/`fail_timeout` are ignored when a group has a single
/// server — a dead sole endpoint is still *attempted* (502), never banned
/// into 503.
#[tokio::test]
async fn single_dead_endpoint_is_attempted_not_banned() {
    let port = free_port().await; // nothing listening — connect refused
    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(1, &[single_slot(endpoint("sole", port, 1, 60_000, 0))]);
    for _ in 0..3 {
        assert!(
            matches!(rt.dispatch(get()).await, ProxyDispatchOutcome::BadGateway),
            "single-server group must keep attempting (nginx ignores max_fails)"
        );
    }
}

/// Same nginx single-server rule when a *multi* slot carries one eligible
/// endpoint: the defense-in-depth collapse to `Single` must also drop the
/// passive-health gate — a dead sole member is attempted (502), not banned
/// into 503.
#[tokio::test]
async fn multi_with_one_eligible_endpoint_is_attempted_not_banned() {
    let port = free_port().await; // nothing listening — connect refused
    let mut slot = multi_slot(
        endpoint("sole", port, 1, 60_000, 0),
        endpoint("sole2", port, 1, 60_000, 0),
    );
    slot.endpoints = Box::new([endpoint("sole", port, 1, 60_000, 0)]);
    slot.endpoint_count = 1;
    let rt = ProxyRuntime::new();
    rt.bind_compiled_slots(1, &[slot]);
    for _ in 0..3 {
        assert!(
            matches!(rt.dispatch(get()).await, ProxyDispatchOutcome::BadGateway),
            "one-eligible multi group must collapse to attempted single (nginx ignores max_fails)"
        );
    }
}

/// F1 regression: the raw-path dial is inside the upstream timeout budget and
/// its failure is recorded. TEST-NET-1 192.0.0.1 is unroutable — the connect
/// hangs (or returns unroutable) instead of being refused instantly, so a
/// regression here hangs the test for the kernel TCP timeout (~75s).
#[tokio::test]
async fn raw_dial_bounded_and_recorded() {
    let desc = exyonq_mod_proxy::UpstreamDescriptor {
        cluster_id: 0,
        upstream_name: "blackhole".into(),
        target: "http://192.0.0.1:80".into(),
        timeout: Duration::from_millis(100),
        host: None,
        max_connect_retries: 0,
    };
    let mut target =
        exyonq_mod_proxy::UpstreamTarget::from_descriptor(&desc).expect("blackhole target parses");
    let peer = Arc::new(PassivePeer::new(5, 60_000, 0));
    target.passive = Arc::clone(&peer);

    let mut sink = tokio::io::sink();
    let started = std::time::Instant::now();
    let result = exyonq_mod_proxy::raw_upstream::stream_get_to_client(
        &target, "/", None, None, &mut sink, false,
    )
    .await;
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "dial must be bounded by target.timeout (100ms), got {:?}",
        started.elapsed()
    );
    assert!(result.is_err(), "blackholed upstream must fail");
    assert_eq!(
        peer.fails(),
        1,
        "dial failure must record a passive outcome"
    );
}
