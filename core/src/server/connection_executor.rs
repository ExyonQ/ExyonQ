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
//! PS3A-F3 I3 — private core connection policy executor.
//!
//! Contains SharedServerState / ProxyClient; workers deliver [`AcceptedConnection`] only.
//!
//! Module is gated `cfg(target_os = "linux")` at `server/mod.rs` — do not duplicate inner
//! `#![cfg(...)]` (clippy `duplicated_attributes` under `-D warnings`).

use crate::kernel::errors::DrainRejected;
use crate::kernel::{GenerationView, HyperHandoff, PlatformConnectionAdmission};
use crate::lifecycle::{ConnectionLifecycleToken, LifecycleState};
use crate::reload::{self, SharedServerState};
use crate::server::accepted_connection::AcceptedConnection;
use crate::server::conn_pool::SyncBenchCache;
use crate::server::connection_errors::{
    classify_accepted_socket_io, ConnectionError, ConnectionServeOutcome, CorePolicyError,
};
use crate::server::drain_probes;
use crate::server::hyper_handoff::spawn_hyper_handoff;
use crate::server::io as conn_io;
use crate::server::wire_dispatch::might_use_static_wire;
use bytes::Bytes;
use exyonq_mod_proxy::ProxyClient;
use exyonq_module_api::static_epoll;
use exyonq_module_api::static_wire;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use tokio::runtime::Handle;
use tracing::warn;

/// Best-effort drain-boundary write (P15-WS5-PROBE-002).
fn write_drain_boundary(stream: &mut TcpStream, head: &[u8]) {
    let _ = stream.set_nodelay(true);
    let _ = stream.write_all(drain_probes::drain_boundary_response(head));
}

/// Admit + pin result for epoll-owned fds (stay in ConnState map or immediate Hyper).
pub(crate) struct EpollAdmitAttach {
    pub token: ConnectionLifecycleToken,
    pub bench_cache: SyncBenchCache,
}

/// PS3A-F3 I6: admission decision at accept (one try_admit, one generation pin).
pub(crate) enum EpollAdmitDecision {
    /// modules_enabled or peek requires Tokio/Hyper — token already admitted.
    HyperReady(EpollAdmitAttach),
    /// Stay on epoll map (static/sendfile pump) — token already admitted.
    StayInMap(EpollAdmitAttach),
}

/// Core-owned policy executor. Not public outside `exyonq-core`.
pub(crate) struct CoreConnectionExecutor {
    shared: SharedServerState,
    proxy_client: ProxyClient,
    ops: Arc<LifecycleState>,
    runtime: Handle,
}

impl CoreConnectionExecutor {
    pub(crate) fn new(
        shared: SharedServerState,
        proxy_client: ProxyClient,
        ops: Arc<LifecycleState>,
        runtime: Handle,
    ) -> Self {
        Self {
            shared,
            proxy_client,
            ops,
            runtime,
        }
    }

    /// Sole productive admission authority for migrated workers: `PlatformConnectionAdmission::try_admit`.
    pub(crate) fn serve_accepted(&self, mut conn: AcceptedConnection) -> ConnectionServeOutcome {
        let token = match PlatformConnectionAdmission::try_admit(&self.ops) {
            Ok(token) => token,
            Err(_) => {
                let head = match conn.prefetched.take() {
                    Some((h, _)) => h,
                    None => {
                        let _ = conn.stream.set_nodelay(true);
                        let _ = conn
                            .stream
                            .set_read_timeout(Some(conn_io::header_read_timeout()));
                        match conn_io::read_until_headers_blocking(&mut conn.stream) {
                            Ok((h, _)) => h,
                            Err(_) => Bytes::new(),
                        }
                    }
                };
                write_drain_boundary(&mut conn.stream, head.as_ref());
                return ConnectionServeOutcome::ConnectionClosed(ConnectionError::DrainRejected);
            }
        };

        let view = self.pin_generation_view();

        if view.modules_enabled {
            return self.spawn_hyper(conn, None, view, token);
        }

        let (head, rest) = match conn.prefetched.take() {
            Some(parts) => parts,
            None => {
                let _ = conn.stream.set_nodelay(true);
                let _ = conn
                    .stream
                    .set_read_timeout(Some(conn_io::header_read_timeout()));
                match conn_io::read_until_headers_blocking(&mut conn.stream) {
                    Ok(parts) => parts,
                    Err(err) => match classify_accepted_socket_io(err) {
                        Ok(cerr) => return ConnectionServeOutcome::ConnectionClosed(cerr),
                        Err(err) => {
                            warn!(%err, peer = %conn.peer, "sync header read non-local error");
                            return ConnectionServeOutcome::ConnectionClosed(
                                ConnectionError::from_io(err),
                            );
                        }
                    },
                }
            }
        };

        if head.is_empty() {
            return ConnectionServeOutcome::Completed;
        }

        if let Some(site_slot) = view.site_static_slot {
            // P1 bench fd write (io_uring / shared static_epoll path) before blocking pool.
            let p1_wire = static_wire::p1_bench_wire_rodata();
            match static_epoll::try_write_bench_response(
                site_slot,
                conn.stream.as_raw_fd(),
                head.as_ref(),
                p1_wire,
                conn_io::write_response_fd,
            ) {
                static_epoll::StaticEpollBenchWriteResult::Written => {
                    return ConnectionServeOutcome::Completed;
                }
                static_epoll::StaticEpollBenchWriteResult::Handoff
                | static_epoll::StaticEpollBenchWriteResult::NoMatch => {}
            }
            if might_use_static_wire(head.as_ref())
                && static_wire::static_use_blocking_pool(head.as_ref())
            {
                match static_wire::serve_blocking_sync(site_slot, &mut conn.stream, head, rest) {
                    Ok(()) => return ConnectionServeOutcome::Completed,
                    Err(err) => match classify_accepted_socket_io(err) {
                        Ok(cerr) => return ConnectionServeOutcome::ConnectionClosed(cerr),
                        Err(err) => {
                            warn!(%err, peer = %conn.peer, "static serve error");
                            return ConnectionServeOutcome::ConnectionClosed(
                                ConnectionError::from_io(err),
                            );
                        }
                    },
                }
            }
        }

        self.spawn_hyper(conn, Some((head, rest)), view, token)
    }

    /// I6: admit once + pin once for epoll accept (Hyper vs stay-in-map).
    pub(crate) fn admit_epoll_accept(
        &self,
        force_hyper: bool,
    ) -> Result<EpollAdmitDecision, DrainRejected> {
        let attach = self.admit_for_epoll_interest()?;
        if attach.bench_cache.modules_enabled || force_hyper {
            Ok(EpollAdmitDecision::HyperReady(attach))
        } else {
            Ok(EpollAdmitDecision::StayInMap(attach))
        }
    }

    /// I6: admit + pin for keepalive / stay-in-map registration (no Hyper decision).
    pub(crate) fn admit_for_epoll_interest(&self) -> Result<EpollAdmitAttach, DrainRejected> {
        let token = PlatformConnectionAdmission::try_admit(&self.ops)?;
        Ok(EpollAdmitAttach {
            token,
            bench_cache: self.pin_bench_cache(),
        })
    }

    /// I6: pin generation without admit (Tokio→epoll token transfer).
    pub(crate) fn pin_bench_cache(&self) -> SyncBenchCache {
        let view = self.pin_generation_view();
        SyncBenchCache {
            site_static_slot: view.site_static_slot,
            modules_enabled: view.modules_enabled,
            generation: view.generation,
        }
    }

    /// I6: generation-stale check without exposing SharedServerState to the worker.
    pub(crate) fn is_generation_stale(&self, pinned_generation: u64) -> bool {
        reload::read_state(&self.shared).generation != pinned_generation
    }

    /// I6: Hyper handoff when ConnState (or accept decision) already holds the token.
    pub(crate) fn spawn_hyper_admitted(
        &self,
        stream: TcpStream,
        peer: SocketAddr,
        prefetched: Option<(Bytes, Bytes)>,
        bench_cache: SyncBenchCache,
        token: ConnectionLifecycleToken,
    ) -> ConnectionServeOutcome {
        let conn = AcceptedConnection::new(
            stream,
            peer,
            None,
            crate::server::accepted_connection::TransportKind::Epoll,
        );
        self.spawn_hyper(conn, prefetched, bench_cache.into(), token)
    }

    /// PS3A-PM3-F1: Hyper handoff from opaque attachment scalars (no SyncBenchCache at kernel).
    // TECH_DEBT_HANDLER_ARITY = DEFERRED_POST_V043 (Linux epoll handoff boundary; R2D).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_hyper_admitted_pinned(
        &self,
        stream: TcpStream,
        peer: SocketAddr,
        prefetched: Option<(Bytes, Bytes)>,
        generation: u64,
        site_static_slot: Option<u32>,
        modules_enabled: bool,
        token: ConnectionLifecycleToken,
    ) -> ConnectionServeOutcome {
        self.spawn_hyper_admitted(
            stream,
            peer,
            prefetched,
            SyncBenchCache {
                site_static_slot,
                modules_enabled,
                generation,
            },
            token,
        )
    }

    /// Best-effort drain-reject write (accept_batch / keepalive cold path).
    #[allow(dead_code)] // reserved for accept_batch / keepalive cold paths
    pub(crate) fn write_drain_rejected(stream: &mut TcpStream) {
        write_drain_boundary(stream, b"");
    }

    fn pin_generation_view(&self) -> GenerationView {
        let state = reload::read_state(&self.shared);
        GenerationView::pinned(
            state.generation,
            state.modules_enabled(),
            state.site_static_slot,
        )
    }

    fn spawn_hyper(
        &self,
        conn: AcceptedConnection,
        prefetched: Option<(Bytes, Bytes)>,
        view: GenerationView,
        token: ConnectionLifecycleToken,
    ) -> ConnectionServeOutcome {
        let handoff = HyperHandoff::new(conn.stream, conn.peer, prefetched, view, token);
        match spawn_hyper_handoff(
            handoff,
            &self.shared,
            self.proxy_client.clone(),
            &self.ops,
            &self.runtime,
        ) {
            Ok(()) => ConnectionServeOutcome::Completed,
            Err(err) => {
                ConnectionServeOutcome::PolicyFailed(CorePolicyError::HandoffConstruction(err))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::accepted_connection::TransportKind;
    use exyonq_mod_proxy::build_incoming_client;
    use std::net::{TcpListener, TcpStream};
    use tokio::runtime::Runtime;

    #[test]
    fn i3_executor_admit_only_via_platform_facade_on_drain() {
        let ops = Arc::new(LifecycleState::new());
        ops.start_drain();
        let rt = Runtime::new().expect("rt");
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
        let exec =
            CoreConnectionExecutor::new(shared, proxy, Arc::clone(&ops), rt.handle().clone());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (_server, peer) = listener.accept().unwrap();
        let outcome = exec.serve_accepted(AcceptedConnection::new(
            client,
            peer,
            None,
            TransportKind::SyncAccept,
        ));
        assert!(matches!(
            outcome,
            ConnectionServeOutcome::ConnectionClosed(ConnectionError::DrainRejected)
        ));
        assert_eq!(ops.active_connections(), 0);
    }

    /// Former sync_accept native test — executor handoff (policy remains core).
    #[test]
    fn ps1a_sync_executor_handoff_transfers_token_to_spawned_task() {
        let ops = Arc::new(LifecycleState::new());
        let rt = Runtime::new().expect("runtime");
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
        let exec = Arc::new(CoreConnectionExecutor::new(
            shared,
            proxy,
            Arc::clone(&ops),
            rt.handle().clone(),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (stream, peer, client) = {
            let client = std::thread::spawn(move || {
                let s = TcpStream::connect(addr).expect("connect");
                let _ = s.shutdown(std::net::Shutdown::Write);
                s
            });
            let (server, peer) = listener.accept().expect("accept");
            (server, peer, client)
        };
        let head = bytes::Bytes::from_static(b"GET /unknown HTTP/1.1\r\nHost: x\r\n\r\n");
        let outcome = exec.serve_accepted(AcceptedConnection::new(
            stream,
            peer,
            Some((head, bytes::Bytes::new())),
            TransportKind::SyncAccept,
        ));
        assert!(matches!(outcome, ConnectionServeOutcome::Completed));
        assert_eq!(ops.active_connections(), 1);
        let _ = client.join();
        rt.block_on(async {
            for _ in 0..50 {
                if ops.active_connections() == 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        });
        assert_eq!(ops.active_connections(), 0);
    }
}
