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
//! NG-10 — active health `expect` rules (status/body/header) + probe metrics.
//! E2E against a real TCP backend on loopback whose health response is
//! controllable at runtime: the peer must leave/enter selection as the probe
//! evaluation flips.

use exyonq_mod_proxy::ProxyRuntime;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledEndpoint, ProxyCompiledSlot, ProxyHealthCheckCompiled, ProxySelectionPolicy,
};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Controllable backend: status code, body, and an optional `X-Health` header
/// flipped live via shared state.
#[derive(Clone)]
struct HealthResp {
    status: Arc<AtomicU16>,
    body: Arc<Mutex<String>>,
    header: Arc<Mutex<Option<String>>>,
}

async fn spawn_health_backend(resp: HealthResp, port: u16) {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let resp = resp.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                if stream.read(&mut buf).await.unwrap_or(0) == 0 {
                    return;
                }
                let status = resp.status.load(Ordering::Relaxed);
                let body = resp.body.lock().unwrap().clone();
                let reason = if status == 200 { "OK" } else { "X" };
                let hdr = resp
                    .header
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|v| format!("X-Health: {v}\r\n"))
                    .unwrap_or_default();
                let out = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n{hdr}Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(out.as_bytes()).await;
            });
        }
    });
}

async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.expect("bind :0");
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

fn health_slot(port: u16, hc: ProxyHealthCheckCompiled) -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "hup".into(),
        target: String::new(),
        timeout: Duration::from_secs(2),
        max_connect_retries: 1,
        single_endpoint_executable: false,
        multi_endpoint_executable: true,
        endpoint_count: 1,
        failover_priority_bands: false,
        endpoints: Box::new([ProxyCompiledEndpoint {
            endpoint_id: "ep".into(),
            http_uri: format!("http://127.0.0.1:{port}"),
            weight: 1,
            priority: 0,
            admin_enabled: true,
            max_fails: 0,
            fail_timeout_ms: 0,
            slow_start_ms: 0,
        }]),
        health_check: hc,
        selection_policy: ProxySelectionPolicy::WeightedRoundRobin,
        sticky_cookie: None,
        sticky_header: None,
        dns_refresh_ms: 0,
        location_rewrites: Vec::new(),
    }
}

fn fast_health() -> ProxyHealthCheckCompiled {
    ProxyHealthCheckCompiled {
        enabled: true,
        interval: Duration::from_millis(40),
        timeout: Duration::from_millis(500),
        path: "/health".into(),
        healthy_threshold: 1,
        unhealthy_threshold: 1,
        ..Default::default()
    }
}

// HEALTH_STATS_GEN is process-global: two ProxyRuntime instances racing in one
// binary poison each other's stat writes (shutdown sets epoch to MAX).
async fn runtime_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

async fn wait_selectable(rt: &ProxyRuntime, want: bool) {
    for _ in 0..100 {
        if rt.upstream_for_cluster(0).is_some() == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("peer selectability did not reach {want}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_status_gates_selection() {
    let _guard = runtime_lock().await;
    let port = free_port().await;
    let resp = HealthResp {
        status: Arc::new(AtomicU16::new(200)),
        body: Arc::new(Mutex::new("ok".into())),
        header: Arc::new(Mutex::new(None)),
    };
    spawn_health_backend(resp.clone(), port).await;
    let rt = ProxyRuntime::new();
    let mut hc = fast_health();
    hc.expect_status = vec![204];
    rt.bind_compiled_slots(1, &[health_slot(port, hc)]);
    // 200 not in expect list [204] → probes fail → peer excluded.
    wait_selectable(&rt, false).await;
    resp.status.store(204, Ordering::Relaxed);
    wait_selectable(&rt, true).await;
    rt.shutdown_health();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_body_and_header_gate_selection() {
    let _guard = runtime_lock().await;
    let port = free_port().await;
    let resp = HealthResp {
        status: Arc::new(AtomicU16::new(200)),
        body: Arc::new(Mutex::new("ok".into())),
        header: Arc::new(Mutex::new(None)),
    };
    spawn_health_backend(resp.clone(), port).await;
    let rt = ProxyRuntime::new();
    let mut hc = fast_health();
    hc.expect_body = Some("READY".into());
    rt.bind_compiled_slots(2, &[health_slot(port, hc)]);
    // Body "ok" lacks "READY" → fail.
    wait_selectable(&rt, false).await;
    *resp.body.lock().unwrap() = "READY".into();
    wait_selectable(&rt, true).await;

    // Now require a header that is absent → fail; present → pass.
    let mut hc2 = fast_health();
    hc2.expect_header = Some(("X-Health".into(), Some("pass".into())));
    rt.bind_compiled_slots(3, &[health_slot(port, hc2)]);
    wait_selectable(&rt, false).await;
    *resp.header.lock().unwrap() = Some("pass".into());
    wait_selectable(&rt, true).await;

    // Metrics: probes ran and series exist for this upstream.
    let mut out = String::new();
    exyonq_mod_proxy::append_health_prometheus(&mut out);
    assert!(out.contains("exyonq_upstream_health_probes_total"));
    assert!(out.contains("upstream=\"hup\""));
    assert!(out.contains("exyonq_upstream_health_state"));
    rt.shutdown_health();
}
