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
//! NG-21 — HTTP/2 applies the same `[tuning]` header budget and header
//! deadline as HTTP/1.1.
//!
//! Real product path: the composition root registered by `bootstrap`, then
//! `exyonq_core::server::run_on` on a real TCP socket, driven by Hyper's real
//! HTTP/2 client (h2c prior knowledge). No mock transport, no fixture server.

mod bootstrap;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::net::SocketAddr;
use std::sync::Once;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Header deadline the whole file runs with.
const DEADLINE_MS: u64 = 500;

static TUNING: Once = Once::new();

/// Publish the committed geometry in the same cell the H1 pre-parser reads.
fn publish_limits() {
    TUNING.call_once(|| {
        exyonq_module_api::runtime_tuning::publish(Some(DEADLINE_MS), None);
    });
}

fn config_for(addr: SocketAddr, root: &std::path::Path) -> exyonq_core::AppConfig {
    let raw = format!(
        "config_version = 1\n\
         [[server]]\n\
         listen = \"{addr}\"\n\
         routes = [\"site\"]\n\
         [[route]]\n\
         name = \"site\"\n\
         match = {{ path = \"/\" }}\n\
         root = \"{}\"\n\
         index = \"index.html\"\n",
        root.display()
    );
    let mut config: exyonq_core::AppConfig = raw.parse().expect("generated config parses");
    // Default-on WAF is unrelated to the header geometry under test.
    config.waf.enabled = false;
    config.http3 = Default::default();
    config
}

async fn start_server() -> SocketAddr {
    bootstrap::ensure_integration_modules();
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(root.path().join("index.html"), b"ok").expect("write index");
    let root = Box::leak(Box::new(root));

    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("probe bind");
    let addr = probe.local_addr().expect("probe addr");
    drop(probe);

    let config = config_for(addr, root.path());
    tokio::spawn(async move {
        let _ = exyonq_core::server::run_on(addr, config).await;
    });

    for _ in 0..200 {
        if TcpStream::connect(addr).await.is_ok() {
            publish_limits();
            return addr;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("server did not start on {addr}");
}

async fn h2_client(addr: SocketAddr) -> hyper::client::conn::http2::SendRequest<Empty<Bytes>> {
    let tcp = TcpStream::connect(addr).await.expect("client tcp");
    let (sender, conn) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tcp))
            .await
            .expect("h2 handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });
    sender
}

fn padded_request(pad: usize) -> hyper::Request<Empty<Bytes>> {
    hyper::Request::builder()
        .method("GET")
        .uri("http://localhost/")
        .header("x-pad", "a".repeat(pad))
        .body(Empty::new())
        .expect("request")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h2_header_within_budget_is_served() {
    let addr = start_server().await;
    let mut sender = h2_client(addr).await;

    let response = sender
        .send_request(padded_request(4_000))
        .await
        .expect("h2 response within budget");
    assert_eq!(
        response.status().as_u16(),
        200,
        "within budget must be served"
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(&body[..], b"ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h2_header_over_budget_is_rejected() {
    let addr = start_server().await;
    let mut sender = h2_client(addr).await;

    match sender.send_request(padded_request(12_000)).await {
        Ok(response) => {
            assert_eq!(
                response.status().as_u16(),
                431,
                "over-budget header block must be refused, never served"
            );
            let body = response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes();
            assert!(
                !body.starts_with(b"ok"),
                "no route content may be served for a refused header block"
            );
        }
        Err(err) => {
            // A stream reset instead of 431 is also a refusal: nothing served.
            assert!(
                !err.to_string().is_empty(),
                "refusal must carry an observable h2 error"
            );
        }
    }

    // The server keeps serving: the rejection is per request, not a crash.
    let mut sender = h2_client(addr).await;
    let response = sender
        .send_request(padded_request(64))
        .await
        .expect("request after rejection");
    assert_eq!(response.status().as_u16(), 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h2_header_deadline_cuts_silent_client() {
    let addr = start_server().await;
    let mut tcp = TcpStream::connect(addr).await.expect("client tcp");
    tcp.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
        .await
        .expect("h2 preface");
    tcp.flush().await.expect("flush");

    let started = Instant::now();
    let mut buf = [0u8; 1024];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), tcp.read(&mut buf)).await {
            // EOF or reset: the connection was cut before the headers arrived.
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(_)) => continue,
            Err(_) => panic!(
                "connection survived {:?} without a request head; the H2 header deadline did not fire",
                started.elapsed()
            ),
        }
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "silent client must be cut by the committed header deadline, took {:?}",
        started.elapsed()
    );
}
