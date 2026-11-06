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
//! GET with a request body must reach the upstream with that body framed.
//! Regression for the empty-body fast path forwarding Content-Length alone,
//! which leaves a strict upstream waiting for bytes that never arrive.

use exyonq_mod_proxy::ProxyRuntime;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledEndpoint, ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest,
    ProxyDispatchService, ProxyMethod, ProxySelectionPolicy,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

fn slot(port: u16) -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "get-body".into(),
        target: format!("http://127.0.0.1:{port}"),
        timeout: Duration::from_secs(2),
        max_connect_retries: 0,
        single_endpoint_executable: true,
        multi_endpoint_executable: false,
        endpoint_count: 1,
        failover_priority_bands: true,
        endpoints: Box::new([ProxyCompiledEndpoint {
            endpoint_id: "origin".into(),
            http_uri: format!("http://127.0.0.1:{port}"),
            weight: 1,
            priority: 0,
            admin_enabled: true,
            max_fails: 1,
            fail_timeout_ms: 60_000,
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

fn request() -> ProxyDispatchRequest {
    ProxyDispatchRequest {
        cluster_id: 0,
        method: ProxyMethod::Get,
        path_and_query: "/".into(),
        host: Some("upstream.test".into()),
        headers: vec![
            ("content-length".into(), "7".into()),
            (
                "content-type".into(),
                "application/x-www-form-urlencoded".into(),
            ),
        ],
        body: Some(b"hi=test".to_vec()),
        remote_addr: "127.0.0.1".into(),
        scheme: "http".into(),
    }
}

fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    })
}

#[tokio::test]
async fn get_body_is_forwarded_and_does_not_hang() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local origin");
    let port = listener.local_addr().expect("origin addr").port();
    let (body_tx, body_rx) = oneshot::channel();

    let origin = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept proxy");
        let mut received = Vec::new();
        let end = loop {
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).await.expect("read headers");
            assert!(n > 0, "proxy closed before request headers");
            received.extend_from_slice(&chunk[..n]);
            if let Some(end) = header_end(&received) {
                break end;
            }
        };
        let expected = content_length(&received[..end]).expect("upstream Content-Length");
        let mut body = received[end + 4..].to_vec();
        while body.len() < expected {
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).await.expect("read body");
            assert!(n > 0, "proxy closed before sending the declared body");
            body.extend_from_slice(&chunk[..n]);
        }
        assert_eq!(&body[..expected], b"hi=test");
        body_tx
            .send(body[..expected].to_vec())
            .expect("report body");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .expect("write response");
    });

    let runtime = ProxyRuntime::new();
    runtime.bind_compiled_slots(1, &[slot(port)]);

    let outcome = tokio::time::timeout(Duration::from_secs(2), runtime.dispatch(request()))
        .await
        .expect("dispatch must not wait for a body the proxy failed to send");
    assert!(matches!(
        outcome,
        ProxyDispatchOutcome::Materialized(_) | ProxyDispatchOutcome::Streaming { .. }
    ));

    let body = tokio::time::timeout(Duration::from_secs(2), body_rx)
        .await
        .expect("origin must receive the body")
        .expect("origin body channel");
    assert_eq!(body, b"hi=test");
    origin.await.expect("origin task");
}
