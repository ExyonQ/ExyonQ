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
//! AP-01 — hot member control: `member set <id> weight=0` / `enabled=false`
//! removes the endpoint from selection without a config reload; re-enabling
//! rejoins it. Exercised through the pinned runtime + wire serve loop.

use exyonq_mod_proxy::wire_conn::{pin_runtime, pinned_runtime, serve};
use exyonq_mod_proxy::ProxyRuntime;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledEndpoint, ProxyCompiledSlot, ProxySelectionPolicy,
};
use hyper::header::HeaderValue;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn runtime_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

async fn echo_backend(tag: &'static str, hits: Arc<AtomicU64>) -> u16 {
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
                let body = tag.as_bytes();
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(body).await;
            });
        }
    });
    port
}

fn slot(name: &str, port_a: u16, port_b: u16) -> ProxyCompiledSlot {
    let ep = |id: &str, port: u16| ProxyCompiledEndpoint {
        endpoint_id: id.into(),
        http_uri: format!("http://127.0.0.1:{port}"),
        weight: 1,
        priority: 0,
        admin_enabled: true,
        // max_fails=0: passive accounting disabled — a stale pooled socket to
        // the python echo backend must not ban a member mid-test.
        max_fails: 0,
        fail_timeout_ms: 10_000,
        slow_start_ms: 0,
    };
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: name.into(),
        target: String::new(),
        timeout: Duration::from_secs(2),
        max_connect_retries: 1,
        single_endpoint_executable: false,
        multi_endpoint_executable: true,
        endpoint_count: 2,
        failover_priority_bands: false,
        endpoints: Box::new([ep("ea", port_a), ep("eb", port_b)]),
        health_check: Default::default(),
        selection_policy: ProxySelectionPolicy::WeightedRoundRobin,
        sticky_cookie: None,
        sticky_header: None,
        dns_refresh_ms: 0,
        location_rewrites: Vec::new(),
    }
}

async fn request_once() -> String {
    let (mut client, server) = tokio::io::duplex(16384);
    let server_task = tokio::spawn(async move {
        serve(
            0,
            1,
            server,
            bytes::Bytes::from_static(
                b"GET /api/x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            ),
            bytes::Bytes::new(),
            HeaderValue::from_static("10.9.9.9"),
            None,
            #[cfg(target_os = "linux")]
            None,
        )
        .await
    });
    let mut out = Vec::new();
    let mut buf = vec![0u8; 8192];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let n = tokio::time::timeout_at(deadline, client.read(&mut buf))
            .await
            .expect("read deadline")
            .expect("read");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if String::from_utf8_lossy(&out).contains("BACKEND_") {
            break;
        }
    }
    let _ = server_task.await;
    String::from_utf8_lossy(&out).into_owned()
    // rt kept alive by caller's Arc clone in runtime.
}

#[tokio::test]
async fn member_set_hot_disables_and_reenables_endpoint() {
    let _serial = runtime_lock().await;
    let hits_a = Arc::new(AtomicU64::new(0));
    let hits_b = Arc::new(AtomicU64::new(0));
    let port_a = echo_backend("BACKEND_A_OK", hits_a.clone()).await;
    let port_b = echo_backend("BACKEND_B_OK", hits_b.clone()).await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    rt.bind_compiled_slots(1, &[slot("backend", port_a, port_b)]);

    // Baseline: both members take traffic.
    let mut saw_a = false;
    let mut saw_b = false;
    for _ in 0..8 {
        let r = request_once().await;
        saw_a |= r.contains("BACKEND_A_OK");
        saw_b |= r.contains("BACKEND_B_OK");
    }
    assert!(
        saw_a && saw_b,
        "baseline must hit both: a={saw_a} b={saw_b}"
    );

    // member_list shows both members enabled under `backend`.
    let rows = pinned_runtime()
        .member_list(Some("backend"))
        .expect("member_list");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.enabled));

    // `member set eb weight=0` — drain eb without reload.
    let row = pinned_runtime()
        .member_set(Some("backend"), "eb", None, Some(0))
        .expect("member exists");
    assert_eq!(row.weight, 0);
    // Sanity: runtime still resolves a live member (ea) after draining eb.
    let sel =
        pinned_runtime().upstream_for_cluster_ctx(0, &exyonq_mod_proxy::runtime::AffinityCtx::NONE);
    assert!(sel.is_some(), "resolve must still serve ea");
    hits_a.store(0, Ordering::Relaxed);
    hits_b.store(0, Ordering::Relaxed);
    for _ in 0..8 {
        let r = request_once().await;
        assert!(
            r.contains("BACKEND_A_OK"),
            "drained member must not serve: {r}"
        );
    }
    assert_eq!(hits_b.load(Ordering::Relaxed), 0);

    // enabled=false on ea now leaves zero members → 502-class failure, not a
    // stale route to a disabled member.
    pinned_runtime()
        .member_set(Some("backend"), "ea", Some(false), None)
        .expect("member exists");
    let r = request_once().await;
    assert!(
        r.starts_with("HTTP/1.1 5") || r.contains("502"),
        "all-disabled upstream must fail closed: {r}"
    );

    // Re-enable ea + restore eb weight — traffic resumes immediately.
    pinned_runtime()
        .member_set(Some("backend"), "ea", Some(true), None)
        .unwrap();
    pinned_runtime()
        .member_set(Some("backend"), "eb", None, Some(1))
        .unwrap();
    let mut saw_b = false;
    for _ in 0..8 {
        saw_b |= request_once().await.contains("BACKEND_B_OK");
    }
    assert!(saw_b, "re-enabled member must serve again");

    // Ambiguity: same endpoint id in a second upstream must error without
    // --upstream scope.
    rt.bind_compiled_slots(
        2,
        &[slot("backend", port_a, port_b), {
            let mut s = slot("backend2", port_a, port_b);
            s.cluster_id = 1;
            s
        }],
    );
    let err = pinned_runtime()
        .member_set(None, "ea", Some(false), None)
        .unwrap_err();
    assert!(err.contains("ambiguous"), "{err}");
    let ok = pinned_runtime()
        .member_set(Some("backend2"), "ea", Some(true), None)
        .expect("scoped set works");
    assert_eq!(ok.upstream, "backend2");
}

#[tokio::test]
async fn member_set_controls_sole_member_binding() {
    let _serial = runtime_lock().await;
    let hits = Arc::new(AtomicU64::new(0));
    let port = echo_backend("BACKEND_SOLE_OK", hits).await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    // One eligible member in a multi-endpoint slot → runtime collapses to a
    // Single binding; the member must still be administrable.
    let mut s = slot("soloup", port, port);
    s.endpoints = {
        let mut e = s.endpoints.into_vec();
        e.truncate(1);
        e.into_boxed_slice()
    };
    s.endpoint_count = 1;
    rt.bind_compiled_slots(1, &[s]);

    // The sole member appears in the table and serves.
    let rows = pinned_runtime()
        .member_list(Some("soloup"))
        .expect("member_list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].endpoint_id, "ea");
    assert!(rows[0].selectable);
    let r = request_once().await;
    assert!(r.contains("BACKEND_SOLE_OK"), "baseline must serve: {r}");

    // enabled=false → no live upstreams (503-class), not silent pass-through.
    let row = pinned_runtime()
        .member_set(Some("soloup"), "ea", Some(false), None)
        .expect("sole member administrable");
    assert!(!row.enabled && !row.selectable);
    let r = request_once().await;
    assert!(
        r.starts_with("HTTP/1.1 5") || r.contains("50"),
        "disabled sole member must fail closed: {r}"
    );

    // Re-enable — serves again, no reload.
    pinned_runtime()
        .member_set(Some("soloup"), "ea", Some(true), None)
        .unwrap();
    let r = request_once().await;
    assert!(r.contains("BACKEND_SOLE_OK"), "re-enabled must serve: {r}");
}

async fn read_response(client: &mut tokio::io::DuplexStream) -> String {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 8192];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let n = tokio::time::timeout_at(deadline, client.read(&mut buf))
            .await
            .expect("read deadline")
            .expect("read");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        let s = String::from_utf8_lossy(&out);
        if s.contains("BACKEND_") || s.contains("Service Unavailable") {
            break;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn keepalive_pin_drops_member_disabled_mid_connection() {
    let _serial = runtime_lock().await;
    let hits_a = Arc::new(AtomicU64::new(0));
    let hits_b = Arc::new(AtomicU64::new(0));
    let port_a = echo_backend("BACKEND_A_OK", hits_a.clone()).await;
    let port_b = echo_backend("BACKEND_B_OK", hits_b.clone()).await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    rt.bind_compiled_slots(1, &[slot("backend", port_a, port_b)]);

    // One keep-alive connection: request 1 pins a member; disabling it must
    // drop the cached pin so request 2 on the SAME connection re-resolves.
    let (mut client, server) = tokio::io::duplex(16384);
    let server_task = tokio::spawn(async move {
        serve(
            0,
            1,
            server,
            bytes::Bytes::from_static(b"GET /api/one HTTP/1.1\r\nHost: x\r\n\r\n"),
            bytes::Bytes::new(),
            HeaderValue::from_static("10.9.9.9"),
            None,
            #[cfg(target_os = "linux")]
            None,
        )
        .await
    });

    let r1 = read_response(&mut client).await;
    let (pinned, other) = if r1.contains("BACKEND_A_OK") {
        ("ea", "BACKEND_B_OK")
    } else {
        assert!(r1.contains("BACKEND_B_OK"), "request1 must serve: {r1}");
        ("eb", "BACKEND_A_OK")
    };
    pinned_runtime()
        .member_set(Some("backend"), pinned, Some(false), None)
        .expect("member exists");

    client
        .write_all(b"GET /api/two HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .expect("write req2");
    let r2 = read_response(&mut client).await;
    assert!(
        r2.contains(other),
        "keep-alive pin must not serve disabled member {pinned}: {r2}"
    );
    let _ = server_task.await;
}

#[tokio::test]
async fn member_set_enables_config_disabled_member() {
    let _serial = runtime_lock().await;
    let hits_c = Arc::new(AtomicU64::new(0));
    let port_a = echo_backend("BACKEND_A_OK", Arc::new(AtomicU64::new(0))).await;
    let port_b = echo_backend("BACKEND_B_OK", Arc::new(AtomicU64::new(0))).await;
    let port_c = echo_backend("BACKEND_C_OK", hits_c.clone()).await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    // Two eligible + one statically disabled → Multi; ec stays in the member
    // table with a real target so `member set enabled=true` can rejoin it.
    let mut s = slot("backend", port_a, port_b);
    s.endpoints = {
        let mut e = s.endpoints.into_vec();
        e.push(ProxyCompiledEndpoint {
            endpoint_id: "ec".into(),
            http_uri: format!("http://127.0.0.1:{port_c}"),
            weight: 1,
            priority: 0,
            admin_enabled: false,
            max_fails: 0,
            fail_timeout_ms: 10_000,
            slow_start_ms: 0,
        });
        e.into_boxed_slice()
    };
    s.endpoint_count = 3;
    rt.bind_compiled_slots(1, &[s]);

    let rows = pinned_runtime().member_list(Some("backend")).expect("list");
    assert_eq!(rows.len(), 3);
    let ec = rows.iter().find(|r| r.endpoint_id == "ec").expect("ec row");
    assert!(!ec.enabled && !ec.selectable);

    let row = pinned_runtime()
        .member_set(Some("backend"), "ec", Some(true), None)
        .expect("member exists");
    assert!(
        row.enabled && row.selectable,
        "enabled member must be selectable"
    );

    for _ in 0..12 {
        request_once().await;
    }
    assert!(
        hits_c.load(Ordering::Relaxed) > 0,
        "enabled member must receive traffic"
    );
}
