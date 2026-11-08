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
//! M-02: the raw HTTP/1 proxy wire and the Hyper empty-body forward must send
//! the same end-to-end request headers to a real upstream. The wire path used
//! to omit Cookie/Authorization/Host/Range/Accept-Encoding from its selection
//! context; this test compares the bytes both paths actually put on the wire.

use exyonq_mod_proxy::hyper_forward::{attempt_forward_empty_body_with_headers, ProxyHyperMetrics};
use exyonq_mod_proxy::wire_conn::{pin_runtime, serve};
use exyonq_mod_proxy::{ProxyRuntime, UpstreamTarget};
use exyonq_module_api::proxy_dispatch::ProxyCompiledSlot;
use hyper::header::{HeaderValue, HOST};
use hyper::Method;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

const REQUEST: &[u8] = b"GET /edge/parity?q=1 HTTP/1.1\r\n\
Host: origin.example\r\n\
Cookie: sid=abc; theme=dark\r\n\
Authorization: Custom m02-test\r\n\
Range: bytes=0-15\r\n\
Accept-Encoding: gzip, br\r\n\
Connection: close, X-Remove-Me\r\n\
X-Remove-Me: hop-by-hop\r\n\
Content-Length: 0\r\n\
\r\n";

const HEADERS: &[&str] = &[
    "cookie",
    "authorization",
    "host",
    "range",
    "accept-encoding",
];

#[derive(Clone)]
struct Capture {
    port: u16,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
}

async fn capture_backend() -> Capture {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
            let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await;
            let Ok(Ok(n)) = read else { break };
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
        }
        captured.lock().await.push(raw);
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
        let _ = stream.shutdown().await;
    });
    Capture { port, requests }
}

fn header_pairs(raw_head: &[u8]) -> Vec<(String, String)> {
    raw_head
        .split(|&b| b == b'\n')
        .skip(1)
        .filter_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let pos = line.iter().position(|&b| b == b':')?;
            let (name, value) = line.split_at(pos);
            let value = std::str::from_utf8(&value[1..]).ok()?.trim();
            let name = std::str::from_utf8(name).ok()?;
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

fn selected_headers(raw_head: &[u8]) -> BTreeMap<String, String> {
    let mut selected = BTreeMap::new();
    for (name, value) in header_pairs(raw_head) {
        let lower = name.to_ascii_lowercase();
        if HEADERS.contains(&lower.as_str()) {
            // First value wins; repeated Cookie lines are merged by the proxy.
            selected.entry(lower).or_insert(value);
        }
    }
    selected
}

fn assert_no_hop_by_hop(raw_head: &[u8]) {
    for (name, _) in header_pairs(raw_head) {
        let lower = name.to_ascii_lowercase();
        assert!(
            !matches!(
                lower.as_str(),
                "connection" | "keep-alive" | "transfer-encoding" | "x-remove-me"
            ),
            "hop-by-hop header leaked upstream: {name}"
        );
        assert!(
            lower != "content-length",
            "empty GET must not forward Content-Length: {lower}"
        );
    }
}

async fn hyper_request(port: u16) -> Vec<u8> {
    let target = UpstreamTarget::from_config(&format!("http://127.0.0.1:{port}"), 2_000)
        .expect("upstream target");
    let metrics = ProxyHyperMetrics::default();
    let mut extra = header_pairs(REQUEST);
    // Keep the same explicit Host the wire request carries; Hyper's path also
    // accepts it as a proxy_set_headers replacement.
    let host = extra
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.clone())
        .expect("Host");
    extra.retain(|(name, _)| !name.eq_ignore_ascii_case("host"));
    extra.push((HOST.as_str().to_string(), host));
    let response = attempt_forward_empty_body_with_headers(
        &target,
        "/edge/parity?q=1",
        Method::GET,
        Some(&HeaderValue::from_static("127.0.0.1")),
        None,
        &extra,
        &metrics,
        Duration::from_secs(2),
    )
    .await
    .expect("hyper forward");
    assert_eq!(response.status().as_u16(), 200);
    Vec::new()
}

#[tokio::test]
async fn wire_and_hyper_forward_same_end_to_end_headers() {
    // `pin_runtime` is process-global; keep this test's binding window serial.
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _serial = LOCK.lock().await;

    let wire_upstream = capture_backend().await;
    let hyper_upstream = capture_backend().await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    rt.bind_compiled_slots(
        1,
        &[ProxyCompiledSlot::legacy_single(
            0,
            "backend",
            format!("http://127.0.0.1:{}", wire_upstream.port),
            Duration::from_secs(2),
        )],
    );

    let (mut client, server) = tokio::io::duplex(16 * 1024);
    let server_task = tokio::spawn(async move {
        serve(
            0,
            1,
            server,
            bytes::Bytes::from_static(REQUEST),
            bytes::Bytes::new(),
            HeaderValue::from_static("127.0.0.1"),
            None,
            #[cfg(target_os = "linux")]
            None,
        )
        .await
    });

    // Drain the client response so serve reaches its close path.
    let mut response = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut response)).await;
    assert_eq!(server_task.await.expect("join").expect("serve"), 200);
    assert!(
        response.starts_with(b"HTTP/1.1 200"),
        "wire response: {}",
        String::from_utf8_lossy(&response)
    );

    hyper_request(hyper_upstream.port).await;

    let wire_raw = wire_upstream.requests.lock().await.clone();
    let hyper_raw = hyper_upstream.requests.lock().await.clone();
    assert_eq!(wire_raw.len(), 1, "wire upstream request count");
    assert_eq!(hyper_raw.len(), 1, "hyper upstream request count");

    let wire_headers = selected_headers(&wire_raw[0]);
    let hyper_headers = selected_headers(&hyper_raw[0]);
    for name in HEADERS {
        assert!(
            wire_headers.contains_key(*name),
            "wire omitted {name}: {:?}",
            wire_headers
        );
        assert_eq!(
            wire_headers.get(*name),
            hyper_headers.get(*name),
            "wire/Hyper differ for {name}: wire={:?} hyper={:?}",
            wire_headers.get(*name),
            hyper_headers.get(*name)
        );
    }
    assert_no_hop_by_hop(&wire_raw[0]);
    assert_no_hop_by_hop(&hyper_raw[0]);
}
