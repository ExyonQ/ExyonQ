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
//! NG-11 — the wire keep-alive loop must re-resolve the effective client IP
//! per request from each request's own forwarding header, against the socket
//! peer's trust (not the previously resolved value).

use exyonq_mod_proxy::wire_conn::{pin_runtime, serve};
use exyonq_mod_proxy::ProxyRuntime;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledEndpoint, ProxyCompiledSlot, ProxySelectionPolicy,
};
use exyonq_module_api::real_ip::RealIpTrust;
use hyper::header::HeaderValue;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn runtime_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// Upstream that replies `xff=<value-of-x-forwarded-for>` per request.
async fn xff_echo_backend() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut req = Vec::new();
                let mut buf = vec![0u8; 8192];
                let _ = tokio::time::timeout(Duration::from_secs(5), async {
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = stream.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        req.extend_from_slice(&buf[..n]);
                    }
                })
                .await;
                let text = String::from_utf8_lossy(&req);
                let xff = text
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("x-forwarded-for:")
                            .map(|v| v.trim().to_string())
                    })
                    .unwrap_or_default();
                let body = format!("xff={xff}");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    port
}

fn plain_slot(port: u16) -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: format!("http://127.0.0.1:{port}"),
        timeout: Duration::from_secs(2),
        max_connect_retries: 1,
        single_endpoint_executable: true,
        multi_endpoint_executable: false,
        endpoint_count: 1,
        failover_priority_bands: false,
        endpoints: Box::new([ProxyCompiledEndpoint {
            endpoint_id: "e1".into(),
            http_uri: format!("http://127.0.0.1:{port}"),
            weight: 1,
            priority: 0,
            admin_enabled: true,
            max_fails: 1,
            fail_timeout_ms: 10_000,
            slow_start_ms: 0,
        }]),
        health_check: Default::default(),
        selection_policy: ProxySelectionPolicy::WeightedRoundRobin,
        sticky_cookie: None,
        sticky_header: None,
        dns_refresh_ms: 0,
        location_rewrites: Vec::new(),
    }
}

fn trust(nets: &[&str]) -> Option<RealIpTrust> {
    Some(RealIpTrust::new(
        "x-forwarded-for",
        &nets.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    ))
}

async fn read_until(stream: &mut tokio::io::DuplexStream, needle: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 8192];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !out.windows(needle.len()).any(|w| w == needle) {
        let n = tokio::time::timeout_at(deadline, stream.read(&mut buf))
            .await
            .expect("read deadline")
            .expect("read");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

#[tokio::test]
async fn keepalive_requests_resolve_their_own_xff() {
    let _serial = runtime_lock().await;
    let port = xff_echo_backend().await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    rt.bind_compiled_slots(1, &[plain_slot(port)]);

    let (mut client, server) = tokio::io::duplex(16384);
    let server_task = tokio::spawn(async move {
        serve(
            0,
            1,
            server,
            // Trusted peer (10.9.9.9) carrying client 203.0.113.1.
            bytes::Bytes::from_static(
                b"GET /api/a HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 203.0.113.1\r\n\r\n",
            ),
            bytes::Bytes::new(),
            HeaderValue::from_static("10.9.9.9"),
            trust(&["10.9.9.0/24"]),
            #[cfg(target_os = "linux")]
            None,
        )
        .await
    });

    let r1 = read_until(&mut client, b"203.0.113.1").await;
    let s1 = String::from_utf8_lossy(&r1);
    assert!(
        s1.contains("xff=203.0.113.1"),
        "request 1 must carry resolved client upstream: {s1}"
    );

    // Request 2 on the SAME socket, different claimed client.
    client
        .write_all(
            b"GET /api/b HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 203.0.113.2\r\nConnection: close\r\n\r\n",
        )
        .await
        .expect("write r2");
    let r2 = read_until(&mut client, b"203.0.113.2").await;
    let s2 = String::from_utf8_lossy(&r2);
    assert!(
        s2.contains("xff=203.0.113.2"),
        "request 2 must resolve its own chain: {s2}"
    );
    let _ = server_task.await;
}

#[tokio::test]
async fn untrusted_peer_xff_ignored_on_wire() {
    let _serial = runtime_lock().await;
    let port = xff_echo_backend().await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    rt.bind_compiled_slots(1, &[plain_slot(port)]);

    let (mut client, server) = tokio::io::duplex(16384);
    let server_task = tokio::spawn(async move {
        serve(
            0,
            1,
            server,
            bytes::Bytes::from_static(
                b"GET /api/a HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 6.6.6.6\r\nConnection: close\r\n\r\n",
            ),
            bytes::Bytes::new(),
            // Peer is 10.9.9.9 — outside the trusted net → socket peer wins.
            HeaderValue::from_static("10.9.9.9"),
            trust(&["10.8.0.0/16"]),
            #[cfg(target_os = "linux")]
            None,
        )
        .await
    });

    let r1 = read_until(&mut client, b"10.9.9.9").await;
    let s = String::from_utf8_lossy(&r1);
    assert!(
        s.contains("xff=10.9.9.9"),
        "untrusted peer must not spoof upstream XFF: {s}"
    );
    let _ = server_task.await;
}
