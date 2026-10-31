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
//! NG-07 F1 regression — the raw HTTP/1 wire path must re-resolve the upstream
//! per request for keyed/load policies. A single keep-alive connection sends
//! two requests carrying different `route=` pins; each must reach its own
//! endpoint. The pre-fix code cached the first selection for the connection
//! lifetime, so the second request hit the first request's endpoint.

use exyonq_mod_proxy::wire_conn::{pin_runtime, serve};
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

/// `pin_runtime` is process-global — tests in this binary must serialize
/// their pinned-runtime window or they overwrite each other's bindings.
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
                // Keep-alive capable: Content-Length framing, no Connection:
                // close — the wire path must keep the client socket open.
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(body).await;
            });
        }
    });
    port
}

fn cookie_affinity_slot(port_a: u16, port_b: u16) -> ProxyCompiledSlot {
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
        selection_policy: ProxySelectionPolicy::CookieAffinity,
        sticky_cookie: Some("route".into()),
        sticky_header: None,
        dns_refresh_ms: 0,
        location_rewrites: Vec::new(),
    }
}

/// Read until `needle` appears or EOF; returns what was received.
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
async fn keepalive_cookie_pins_re_resolve_per_request() {
    let _serial = runtime_lock().await;
    let hits_a = Arc::new(AtomicU64::new(0));
    let hits_b = Arc::new(AtomicU64::new(0));
    let port_a = echo_backend("BACKEND_A_OK", hits_a.clone()).await;
    let port_b = echo_backend("BACKEND_B_OK", hits_b.clone()).await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    rt.bind_compiled_slots(1, &[cookie_affinity_slot(port_a, port_b)]);

    let (mut client, server) = tokio::io::duplex(16384);
    let server_task = tokio::spawn(async move {
        serve(
            0,
            1,
            server,
            bytes::Bytes::from_static(
                b"GET /api/first HTTP/1.1\r\nHost: x\r\nCookie: route=ea\r\n\r\n",
            ),
            bytes::Bytes::new(),
            HeaderValue::from_static("10.9.9.9"),
            None,
            #[cfg(target_os = "linux")]
            None,
        )
        .await
    });

    // Request 1 (pin ea): response must carry the route token and hit backend A.
    let r1 = read_until(&mut client, b"BACKEND_A_OK").await;
    let s1 = String::from_utf8_lossy(&r1);
    assert!(s1.starts_with("HTTP/1.1 200"), "r1: {s1}");
    assert!(
        s1.to_ascii_lowercase().contains("set-cookie: route=ea"),
        "r1 must mint route=ea: {s1}"
    );

    // Request 2 on the SAME socket with a different pin must hit backend B.
    client
        .write_all(
            b"GET /api/second HTTP/1.1\r\nHost: x\r\nCookie: route=eb\r\nConnection: close\r\n\r\n",
        )
        .await
        .expect("write r2");
    let r2 = read_until(&mut client, b"BACKEND_B_OK").await;
    let s2 = String::from_utf8_lossy(&r2);
    if !s2.starts_with("HTTP/1.1 200") {
        let join = server_task.await;
        panic!("r2 empty/failed: {s2:?}; serve join: {join:?}");
    }
    assert!(
        s2.to_ascii_lowercase().contains("set-cookie: route=eb"),
        "r2 must mint route=eb: {s2}"
    );
    let set_cookie_count = s2
        .lines()
        .filter(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .count();
    assert_eq!(set_cookie_count, 1, "exactly one route cookie: {s2}");

    let status = server_task.await.expect("join").expect("serve");
    assert_eq!(status, 200);

    // Each pin reached its own endpoint — the pre-fix per-connection cache
    // would have served r2 from backend A.
    assert_eq!(hits_a.load(Ordering::Relaxed), 1, "backend A hits");
    assert_eq!(hits_b.load(Ordering::Relaxed), 1, "backend B hits");
}

/// Killable/revivable echo backend bound before returning.
async fn echo_backend_handle(
    tag: &'static str,
    hits: Arc<AtomicU64>,
    port: u16,
) -> tokio::task::JoinHandle<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    tokio::spawn(async move {
        // JoinSet: aborting this task must also kill accepted connections —
        // otherwise a pooled keepalive conn can keep serving a "dead" backend.
        let mut conns = tokio::task::JoinSet::new();
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let hits = hits.clone();
            conns.spawn(async move {
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
    })
}

async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.expect("bind :0");
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

/// NG-08 F2 regression — the per-connection WRR pin must not bypass the
/// passive ban. Sequence: r1 pins endpoint A; A dies → r2's doomed attempt
/// bans it and fails over to B; A revives; r3 must still go to B — the
/// pre-fix pin would have attempted (and hit) the revived banned A.
#[tokio::test]
async fn keepalive_pin_dropped_when_peer_banned() {
    let _serial = runtime_lock().await;
    let hits_a = Arc::new(AtomicU64::new(0));
    let hits_b = Arc::new(AtomicU64::new(0));
    let port_a = free_port().await;
    let port_b = free_port().await;
    let a_task = echo_backend_handle("BACKEND_A_OK", hits_a.clone(), port_a).await;
    let b_task = echo_backend_handle("BACKEND_B_OK", hits_b.clone(), port_b).await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    let mut slot = cookie_affinity_slot(port_a, port_b);
    slot.selection_policy = ProxySelectionPolicy::WeightedRoundRobin;
    slot.sticky_cookie = None;
    // Long ban window so the ban is still active for r3.
    for ep in slot.endpoints.iter_mut() {
        ep.fail_timeout_ms = 60_000;
    }
    rt.bind_compiled_slots(1, &[slot]);

    let (mut client, server) = tokio::io::duplex(16384);
    let server_task = tokio::spawn(async move {
        serve(
            0,
            1,
            server,
            bytes::Bytes::from_static(b"GET /api/r1 HTTP/1.1\r\nHost: x\r\n\r\n"),
            bytes::Bytes::new(),
            HeaderValue::from_static("10.9.9.9"),
            None,
            #[cfg(target_os = "linux")]
            None,
        )
        .await
    });

    // r1 pins whichever endpoint WRR picks first — discover it from hits.
    let r1 = read_until(&mut client, b"OK").await;
    assert!(
        String::from_utf8_lossy(&r1).starts_with("HTTP/1.1 200"),
        "r1"
    );
    let (pinned_port, pinned_hits, pinned_task, pinned_tag) = if hits_a.load(Ordering::Relaxed) == 1
    {
        (port_a, hits_a.clone(), a_task, "BACKEND_A_OK")
    } else {
        (port_b, hits_b.clone(), b_task, "BACKEND_B_OK")
    };

    // Kill the pinned backend, then r2: the doomed attempt bans the peer and
    // the failover must serve from the live member.
    pinned_task.abort();
    // `abort()` only schedules cancellation: until the listener is really
    // closed, r2 can still be served by the "dead" backend and the assertion
    // below fails for reasons unrelated to the pin being dropped.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match tokio::net::TcpStream::connect(("127.0.0.1", pinned_port)).await {
            Ok(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            Err(_) => break,
        }
    }
    client
        .write_all(b"GET /api/r2 HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .expect("write r2");
    let r2 = read_until(&mut client, b"OK").await;
    let r2_head = String::from_utf8_lossy(&r2)
        .chars()
        .take(200)
        .collect::<String>();
    assert!(r2_head.starts_with("HTTP/1.1 200"), "r2: {r2_head}");

    // Revive the (banned) pinned backend on the same port.
    let mut revived = None;
    for _ in 0..30 {
        match TcpListener::bind(("127.0.0.1", pinned_port)).await {
            Ok(l) => {
                let hits = pinned_hits.clone();
                revived = Some(tokio::spawn(async move {
                    loop {
                        let Ok((mut stream, _)) = l.accept().await else {
                            return;
                        };
                        let hits = hits.clone();
                        tokio::spawn(async move {
                            let mut buf = vec![0u8; 8192];
                            if stream.read(&mut buf).await.unwrap_or(0) == 0 {
                                return;
                            }
                            hits.fetch_add(1, Ordering::Relaxed);
                            let body = pinned_tag.as_bytes();
                            let head = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(head.as_bytes()).await;
                            let _ = stream.write_all(body).await;
                        });
                    }
                }));
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    let _keep = revived.expect("revive pinned backend");

    // r3: the pin was dropped when the peer banned — must NOT hit the revived
    // banned member again.
    let frozen_pinned = pinned_hits.load(Ordering::Relaxed);
    client
        .write_all(b"GET /api/r3 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .expect("write r3");
    let r3 = read_until(&mut client, b"OK").await;
    assert!(
        String::from_utf8_lossy(&r3).starts_with("HTTP/1.1 200"),
        "r3"
    );
    let status = server_task.await.expect("join").expect("serve");
    assert_eq!(status, 200);
    assert_eq!(
        pinned_hits.load(Ordering::Relaxed),
        frozen_pinned,
        "banned pinned peer must not be re-attempted after revival"
    );
}

/// NG-08 audit S1 regression — when the upstream dies mid-body the raw path
/// has already written the response head (+ partial body) to the client, so
/// falling back would emit a SECOND status line on the same downstream
/// stream. The wire path must close instead: exactly one head on the wire.
#[tokio::test]
async fn mid_body_upstream_close_does_not_write_second_head() {
    let _serial = runtime_lock().await;

    // Backend sends a valid head + partial body, then drops the connection.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                if stream.read(&mut buf).await.unwrap_or(0) == 0 {
                    return;
                }
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                    .await;
                // drop → mid-body EOF for the proxy's upstream read
            });
        }
    });

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    let mut slot = cookie_affinity_slot(port, port);
    slot.selection_policy = ProxySelectionPolicy::WeightedRoundRobin;
    slot.sticky_cookie = None;
    rt.bind_compiled_slots(1, &[slot]);

    let (mut client, server) = tokio::io::duplex(16384);
    let server_task = tokio::spawn(async move {
        serve(
            0,
            1,
            server,
            bytes::Bytes::from_static(b"GET /api/trunc HTTP/1.1\r\nHost: x\r\n\r\n"),
            bytes::Bytes::new(),
            HeaderValue::from_static("10.9.9.9"),
            None,
            #[cfg(target_os = "linux")]
            None,
        )
        .await
    });

    // Read everything until the proxy closes the downstream stream.
    let mut received = Vec::new();
    let mut buf = vec![0u8; 8192];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(5), client.read(&mut buf))
            .await
            .expect("read deadline")
            .expect("read");
        if n == 0 {
            break;
        }
        received.extend_from_slice(&buf[..n]);
    }
    let status = server_task.await.expect("join").expect("serve");

    let heads = received
        .windows(b"HTTP/1.1 ".len())
        .filter(|w| *w == b"HTTP/1.1 ")
        .count();
    assert_eq!(
        heads,
        1,
        "exactly one response head on the wire: {}",
        String::from_utf8_lossy(&received)
    );
    assert_eq!(status, 502, "post-head failure closes with 502");
}
