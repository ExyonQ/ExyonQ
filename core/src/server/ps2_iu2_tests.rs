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
//! PS2-IU2: idle header-read timeout must not kill accept workers (facade path).
#![cfg(all(test, target_os = "linux"))]

use crate::kernel::PlatformConnectionEntry;
use crate::lifecycle::LifecycleState;
use crate::reload;
use crate::server::accepted_connection::{AcceptedConnection, TransportKind};
use crate::server::connection_errors::ConnectionServeOutcome;
use crate::server::connection_executor::CoreConnectionExecutor;
use exyonq_mod_proxy::build_incoming_client;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn facade_accept_loop(
    listener: TcpListener,
    shutdown: Arc<AtomicBool>,
    entry: Arc<PlatformConnectionEntry>,
) -> io::Result<()> {
    while !shutdown.load(Ordering::Relaxed) {
        let (stream, peer) = match listener.accept() {
            Ok(conn) => conn,
            Err(_) if shutdown.load(Ordering::Relaxed) => break,
            Err(err) => return Err(err),
        };
        match entry.serve_accepted_connection(AcceptedConnection::new(
            stream,
            peer,
            None,
            TransportKind::IoUring,
        )) {
            ConnectionServeOutcome::Completed
            | ConnectionServeOutcome::ConnectionClosed(_)
            | ConnectionServeOutcome::PolicyFailed(_) => {}
        }
    }
    Ok(())
}

fn spawn_facade_workers(
    listener: &TcpListener,
    workers: usize,
    entry: Arc<PlatformConnectionEntry>,
    shutdown: Arc<AtomicBool>,
) -> Vec<thread::JoinHandle<()>> {
    (0..workers)
        .map(|_| {
            let listener = listener.try_clone().expect("clone listener");
            let shutdown = Arc::clone(&shutdown);
            let entry = Arc::clone(&entry);
            thread::spawn(move || {
                let _ = facade_accept_loop(listener, shutdown, entry);
            })
        })
        .collect()
}

fn unblock_worker_accepts(addr: std::net::SocketAddr, count: usize) {
    for _ in 0..count {
        let _ = TcpStream::connect(addr);
    }
}

#[test]
fn ps2_iu2_workers_survive_idle_header_read_timeout() {
    crate::kernel::test_hooks::ensure_wire_hooks_installed();
    let prev = std::env::var("EXYONQ_READ_TIMEOUT_MS").ok();
    crate::server::reset_header_read_timeout_cache_for_tests();
    std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "80");
    crate::server::reset_header_read_timeout_cache_for_tests();

    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let raw = include_str!("../../../tests/fixtures/minimal.toml");
    let config: crate::config::AppConfig = raw.parse().expect("config");
    let proxy = build_incoming_client();
    let shared = rt.block_on(async {
        reload::wrap_state(
            crate::server::state::ServerState::new(config, proxy.clone())
                .await
                .expect("state"),
        )
    });
    let ops = Arc::new(LifecycleState::new());
    let shutdown = Arc::new(AtomicBool::new(false));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let executor = Arc::new(CoreConnectionExecutor::new(
        shared,
        proxy.clone(),
        Arc::clone(&ops),
        rt.handle().clone(),
    ));
    let entry = Arc::new(PlatformConnectionEntry::from_executor(executor));

    const WORKERS: usize = 4;
    let handles = spawn_facade_workers(&listener, WORKERS, entry, Arc::clone(&shutdown));

    let idle = thread::spawn(move || {
        let _idle = TcpStream::connect(addr).expect("connect");
        thread::sleep(Duration::from_millis(200));
    });
    idle.join().expect("join");
    thread::sleep(Duration::from_millis(50));

    for (i, handle) in handles.iter().enumerate() {
        assert!(
            !handle.is_finished(),
            "worker {i} must survive read timeout"
        );
    }
    assert_eq!(ops.active_connections(), 0);

    let mut client = TcpStream::connect(addr).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    client
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .expect("write");
    let mut buf = [0u8; 512];
    let n = client.read(&mut buf).expect("read health");
    let response = std::str::from_utf8(&buf[..n]).expect("utf8");
    assert!(
        response.contains("200"),
        "post-timeout request failed: {response}"
    );

    shutdown.store(true, Ordering::SeqCst);
    unblock_worker_accepts(addr, WORKERS);
    for handle in handles {
        handle.join().expect("join worker");
    }

    match prev {
        Some(v) => std::env::set_var("EXYONQ_READ_TIMEOUT_MS", v),
        None => std::env::remove_var("EXYONQ_READ_TIMEOUT_MS"),
    }
    crate::server::reset_header_read_timeout_cache_for_tests();
}

#[test]
fn ps2_iu2_non_transient_listener_error_still_fatal() {
    let err = io::Error::other("listener infrastructure failure");
    assert!(!crate::server::io::is_connection_level_io_error(&err));
}

#[test]
fn ps1a_io_uring_admission_drain_waits_for_token() {
    let ops = LifecycleState::new();
    let token = ops.try_enter().expect("enter");
    ops.start_drain();
    assert!(!ops.drain_complete());
    drop(token);
    assert!(ops.drain_complete());
}
