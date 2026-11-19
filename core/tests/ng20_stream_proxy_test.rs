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
//! NG-20 real-socket runtime coverage: TCP balancing and idle close, plus UDP
//! client affinity. Backends are real local sockets, not transport mocks.

use exyonq_config_ir::AppConfig;
use exyonq_core::lifecycle::LifecycleState;
use exyonq_core::stream_proxy::start_streams;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

async fn tcp_backend(label: u8) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("tcp bind");
    let addr = listener.local_addr().expect("tcp addr");
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 64];
                while matches!(stream.read(&mut buf).await, Ok(n) if n > 0) {
                    if stream.write_all(&[label]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (addr, task)
}

async fn udp_backend(label: u8) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("udp bind");
    let addr = socket.local_addr().expect("udp addr");
    let task = tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        loop {
            let Ok((_, peer)) = socket.recv_from(&mut buf).await else {
                break;
            };
            let mut reply = vec![label];
            reply.extend_from_slice(b"pong");
            if socket.send_to(&reply, peer).await.is_err() {
                break;
            }
        }
    });
    (addr, task)
}

fn config(protocol: &str, first: SocketAddr, second: SocketAddr, idle_ms: u64) -> AppConfig {
    let text = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:18080"
routes = []

[[upstream]]
name = "pool"
timeout_ms = 2000

[[upstream.endpoints]]
id = "a"
address = "127.0.0.1"
port = {}

[[upstream.endpoints]]
id = "b"
address = "127.0.0.1"
port = {}

[[stream]]
name = "l4"
listen = "127.0.0.1:0"
protocol = "{}"
upstream = "pool"
idle_timeout_ms = {}
max_sessions = 8
"#,
        first.port(),
        second.port(),
        protocol,
        idle_ms
    );
    text.parse().expect("stream config")
}

async fn start(config: &AppConfig) -> (SocketAddr, std::sync::Arc<LifecycleState>) {
    let ops = LifecycleState::new();
    let addresses = start_streams(&config.streams, &config.upstreams, ops.clone())
        .await
        .expect("stream listeners bind");
    assert_eq!(addresses.len(), 1);
    (addresses[0], ops)
}

async fn tcp_round_trip(addr: SocketAddr) -> u8 {
    let mut stream = TcpStream::connect(addr).await.expect("client connect");
    stream.write_all(b"ping").await.expect("client write");
    let mut reply = [0u8; 1];
    stream.read_exact(&mut reply).await.expect("client read");
    reply[0]
}

#[tokio::test]
async fn tcp_stream_balances_two_backends() {
    let (a, a_task) = tcp_backend(b'A').await;
    let (b, b_task) = tcp_backend(b'B').await;
    let config = config("tcp", a, b, 5_000);
    let (listen, ops) = start(&config).await;

    let mut replies = Vec::new();
    for _ in 0..4 {
        replies.push(tcp_round_trip(listen).await);
    }

    assert_eq!(
        replies.iter().filter(|&&reply| reply == b'A').count(),
        2,
        "WRR must use both equal-weight TCP endpoints: {replies:?}"
    );
    assert_eq!(
        replies.iter().filter(|&&reply| reply == b'B').count(),
        2,
        "WRR must use both equal-weight TCP endpoints: {replies:?}"
    );

    ops.request_shutdown();
    a_task.abort();
    b_task.abort();
}

#[tokio::test]
async fn tcp_stream_closes_idle_connection() {
    let (a, a_task) = tcp_backend(b'A').await;
    let (b, b_task) = tcp_backend(b'B').await;
    let config = config("tcp", a, b, 150);
    let (listen, ops) = start(&config).await;

    let mut client = TcpStream::connect(listen).await.expect("client connect");
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), client.read(&mut byte)).await;
    assert!(read.is_ok(), "idle relay must close the client");
    assert_eq!(read.unwrap().expect("read result"), 0);

    ops.request_shutdown();
    a_task.abort();
    b_task.abort();
}

#[tokio::test]
async fn udp_stream_keeps_client_affinity() {
    let (a, a_task) = udp_backend(b'A').await;
    let (b, b_task) = udp_backend(b'B').await;
    let config = config("udp", a, b, 5_000);
    let (listen, ops) = start(&config).await;

    let first = UdpSocket::bind("127.0.0.1:0").await.expect("udp client");
    first.connect(listen).await.expect("udp connect");
    first.send(b"one").await.expect("udp send one");
    let mut reply = [0u8; 5];
    let n = first.recv(&mut reply).await.expect("udp recv one");
    assert_eq!(&reply[..n], b"Apong");

    first.send(b"two").await.expect("udp send two");
    let n = first.recv(&mut reply).await.expect("udp recv two");
    assert_eq!(
        &reply[..n],
        b"Apong",
        "a UDP source must stay pinned to its endpoint"
    );

    let second = UdpSocket::bind("127.0.0.1:0").await.expect("udp client 2");
    second.connect(listen).await.expect("udp connect 2");
    second.send(b"three").await.expect("udp send three");
    let n = second.recv(&mut reply).await.expect("udp recv three");
    assert_eq!(
        &reply[..n],
        b"Bpong",
        "the next UDP session must use the second endpoint"
    );

    ops.request_shutdown();
    a_task.abort();
    b_task.abort();
}
