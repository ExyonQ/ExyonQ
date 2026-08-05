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
//! Linux sync accept workers (PS3A-PM1): accept + enqueue; policy via PlatformConnectionEntry.
//!
//! INTERNAL WORKSPACE PLATFORM CRATE — NOT STABLE PUBLIC API
//! Single productive implementation (moved from `exyonq-core::server::sync_accept`).

use crate::conn_pool::{ConnJob, ConnectionPool};
use crate::linux_bind::bind_tuned_std;
use exyonq_core::kernel::{
    AcceptedConnection, ConnectionServeOutcome, PlatformConnectionEntry, TransportKind,
};
use exyonq_core::server::OsWorkerGuard;
use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::thread;
use tracing::warn;

const POOL_SATURATED_RESPONSE: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\nunavailable";

fn reject_sync_accept_connection(mut stream: TcpStream, peer: SocketAddr) {
    let _ = stream.set_nodelay(true);
    if stream.write_all(POOL_SATURATED_RESPONSE).is_err() {
        warn!(%peer, "connection pool saturated, failed to write 503");
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// Productive sync-accept starter registered at composition root.
pub fn start_sync_accept_workers(
    listen: SocketAddr,
    workers: usize,
    entry: Arc<PlatformConnectionEntry>,
) -> io::Result<OsWorkerGuard> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let pool = ConnectionPool::spawn(Arc::new(serve_connection_job));
    let mut handles = Vec::with_capacity(workers);
    let mut listeners = Vec::with_capacity(workers);

    let pool = Arc::new(pool);
    for _ in 0..workers {
        let listener = bind_tuned_std(listen)?;
        listeners.push(listener.try_clone()?);

        let shutdown = Arc::clone(&shutdown);
        let pool_tx = pool.sender();
        let entry = Arc::clone(&entry);
        handles.push(thread::spawn(move || {
            if let Err(err) = accept_loop(listener, shutdown, pool_tx, entry) {
                warn!(%err, "sync accept worker stopped");
            }
        }));
    }

    let mut guard = OsWorkerGuard::new(shutdown, handles, listeners);
    guard.retain_arc(pool);
    Ok(guard)
}

fn accept_loop(
    listener: TcpListener,
    shutdown: Arc<AtomicBool>,
    pool: SyncSender<ConnJob>,
    entry: Arc<PlatformConnectionEntry>,
) -> io::Result<()> {
    while !shutdown.load(Ordering::Relaxed) {
        let (stream, peer) = match listener.accept() {
            Ok(conn) => conn,
            Err(_) if shutdown.load(Ordering::Relaxed) => break,
            Err(err) => return Err(err),
        };
        let job = ConnJob {
            conn: AcceptedConnection::new(stream, peer, None, TransportKind::SyncAccept),
            entry: Arc::clone(&entry),
        };
        match pool.try_send(job) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(job)) => {
                warn!(%peer, "connection pool saturated, rejecting connection");
                reject_sync_accept_connection(job.conn.stream, peer);
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => break,
        }
    }
    Ok(())
}

fn serve_connection_job(job: ConnJob) {
    let ConnJob { conn, entry } = job;
    let peer = conn.peer;
    match entry.serve_accepted_connection(conn) {
        ConnectionServeOutcome::Completed => {}
        ConnectionServeOutcome::ConnectionClosed(err) => {
            warn!(%err, %peer, "sync connection closed");
        }
        ConnectionServeOutcome::PolicyFailed(err) => {
            warn!(%err, %peer, "sync connection policy failed");
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use exyonq_core::lifecycle::LifecycleState;
    use std::net::{TcpListener, TcpStream as StdTcpStream};

    #[test]
    fn pm1_pool_rejection_creates_no_token() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (stream, peer) = {
            let _client = std::thread::spawn(move || StdTcpStream::connect(addr).expect("connect"));
            listener.accept().expect("accept")
        };
        let ops = LifecycleState::new();
        reject_sync_accept_connection(stream, peer);
        assert_eq!(ops.active_connections(), 0);
    }
}
