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
//! F3-R20 — `location_rewrite` (ProxyPassReverse / proxy_redirect): upstream
//! `Location`/`Content-Location` headers pointing at the backend URL are
//! rewritten to the public prefix on both serving paths.

use exyonq_mod_proxy::wire_conn::{pin_runtime, serve};
use exyonq_mod_proxy::ProxyRuntime;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledEndpoint, ProxyCompiledSlot, ProxySelectionPolicy,
};
use hyper::header::HeaderValue;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn runtime_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// Backend that 302s to its own absolute URL, like an app behind a reverse
/// proxy that emits backend-internal redirects.
async fn redirect_backend() -> u16 {
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
                let head = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/old/x\r\nContent-Location: http://127.0.0.1:{port}/old/y\r\nContent-Length: 0\r\n\r\n"
                );
                let _ = stream.write_all(head.as_bytes()).await;
            });
        }
    });
    port
}

fn slot(port: u16, rewrites: Vec<(String, String)>) -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: String::new(),
        timeout: Duration::from_secs(2),
        max_connect_retries: 1,
        single_endpoint_executable: false,
        multi_endpoint_executable: true,
        endpoint_count: 1,
        failover_priority_bands: false,
        endpoints: Box::new([ProxyCompiledEndpoint {
            endpoint_id: "ea".into(),
            http_uri: format!("http://127.0.0.1:{port}"),
            weight: 1,
            priority: 0,
            admin_enabled: true,
            max_fails: 0,
            fail_timeout_ms: 10_000,
            slow_start_ms: 0,
        }]),
        health_check: Default::default(),
        selection_policy: ProxySelectionPolicy::WeightedRoundRobin,
        sticky_cookie: None,
        sticky_header: None,
        dns_refresh_ms: 0,
        location_rewrites: rewrites,
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
    }
    let _ = server_task.await;
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn wire_response_rewrites_backend_location_headers() {
    let _serial = runtime_lock().await;
    let port = redirect_backend().await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    rt.bind_compiled_slots(
        1,
        &[slot(
            port,
            vec![(format!("http://127.0.0.1:{port}/"), "/app/".to_string())],
        )],
    );

    let r = request_once().await;
    // Apache semantics: backend `/old/x` under `from …/` maps to `/app/old/x`.
    assert!(
        r.contains("ocation: /app/old/x"),
        "Location must be rewritten to the public prefix: {r}"
    );
    assert!(
        r.contains("ocation: /app/old/y"),
        "Content-Location must be rewritten too: {r}"
    );
    assert!(
        !r.contains("127.0.0.1"),
        "backend URL must not leak to the client: {r}"
    );
}

#[tokio::test]
async fn wire_response_untouched_without_rules() {
    let _serial = runtime_lock().await;
    let port = redirect_backend().await;

    let rt = Arc::new(ProxyRuntime::new());
    pin_runtime(rt.clone());
    rt.bind_compiled_slots(1, &[slot(port, Vec::new())]);

    let r = request_once().await;
    assert!(
        r.contains(&format!("ocation: http://127.0.0.1:{port}/old/x")),
        "no rules → verbatim pass-through: {r}"
    );
}
