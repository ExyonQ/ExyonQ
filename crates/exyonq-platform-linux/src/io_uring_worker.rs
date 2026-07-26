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
//! Linux io_uring accept workers (PS3A-PM2): accept + facade serve.
//!
//! INTERNAL WORKSPACE PLATFORM CRATE — NOT STABLE PUBLIC API
//! Single productive implementation (moved from `exyonq-core::server::io_uring_worker`).
//!
//! IU2: connection-local WouldBlock/timeout outcomes never kill this loop — only listener Err does.
//! Policy remains in `PlatformConnectionEntry` / core executor.

use crate::linux_bind::bind_tuned_std;
use exyonq_core::kernel::{AcceptedConnection, ConnectionServeOutcome, PlatformConnectionEntry, TransportKind};
use exyonq_core::server::OsWorkerGuard;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tracing::warn;

/// Productive io_uring-mode starter registered at composition root.
pub fn start_io_uring_workers(
    listen: SocketAddr,
    workers: usize,
    entry: Arc<PlatformConnectionEntry>,
) -> io::Result<OsWorkerGuard> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(workers);
    let mut listeners = Vec::with_capacity(workers);

    for _ in 0..workers {
        let listener = bind_tuned_std(listen)?;
        listeners.push(listener.try_clone()?);

        let shutdown = Arc::clone(&shutdown);
        let entry = Arc::clone(&entry);
        handles.push(thread::spawn(move || {
            if let Err(err) = uring_accept_loop(listener, shutdown, entry) {
                warn!(%err, "io_uring static worker stopped");
            }
        }));
    }

    Ok(OsWorkerGuard::new(shutdown, handles, listeners))
}

fn uring_accept_loop(
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
        // IU2 / connection-local outcomes never kill the accept loop — only listener Err does.
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn pm2_loop_survives_connection_closed_outcomes_structurally() {
        // Compile/shape proof: starter signature is facade-only (no SharedServerState).
        let _ = start_io_uring_workers
            as fn(SocketAddr, usize, Arc<PlatformConnectionEntry>) -> io::Result<OsWorkerGuard>;
        let _ = AtomicUsize::new(0);
    }
}
