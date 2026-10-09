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
use crate::server::wire_dispatch::{
    evaluate_wire_waf, format_waf_reject_http, might_use_static_wire,
    waf_wire_materialization_active,
};
use bytes::Bytes;
use exyonq_mod_proxy::ProxyClient;
use exyonq_module_api::static_epoll;
use exyonq_module_api::static_wire;
use hyper::header::HeaderValue;
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
    /// Peek / force_hyper / WAF-enforce requires Tokio/Hyper — token already admitted.
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
        crate::server::hyper_handoff::ensure_handoff_bridge(
            &runtime,
            SharedServerState::clone(&shared),
            proxy_client.clone(),
            Arc::clone(&ops),
        );
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

        // Compression (body filters) forces Hyper. Ratelimit+metrics are wire-cheap.
        if view.modules_enabled && self.hyper_required_from_shared() {
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

        {
            let state = reload::read_state(&self.shared);
            if state
                .config
                .servers
                .iter()
                .any(|server| !server.response_headers.is_empty())
            {
                return self.spawn_hyper(conn, Some((head, rest)), view, token);
            }
        }

        if let Some(site_slot) = view.site_static_slot {
            // Cap015 / WAF-LOGIC-P1-E: sync/epoll static must not bypass WAF.
            // ARCH-002: when WAF is disabled, skip XFF/HeaderValue + wire materialization.
            let state = reload::read_state(&self.shared);
            if waf_wire_materialization_active(&state) {
                let xff = HeaderValue::from_str(&conn.peer.ip().to_string())
                    .unwrap_or_else(|_| HeaderValue::from_static("127.0.0.1"));
                if let crate::waf::WafHookResult::Reject(reject) =
                    evaluate_wire_waf(&state, view.generation, head.as_ref(), &xff)
                {
                    let bytes = format_waf_reject_http(&reject);
                    let _ = conn.stream.set_nodelay(true);
                    if let Err(err) = conn.stream.write_all(&bytes) {
                        warn!(
                            %err,
                            peer = %conn.peer,
                            "waf reject response write failed"
                        );
                    }
                    return ConnectionServeOutcome::Completed;
                }
            }

            // Inline health wire (io_uring / shared static_epoll path) before blocking pool.
            match static_epoll::try_write_inline_wire_response(
                site_slot,
                conn.stream.as_raw_fd(),
                head.as_ref(),
                conn_io::write_response_fd,
            ) {
                static_epoll::StaticEpollInlineWireResult::Written => {
                    return ConnectionServeOutcome::Completed;
                }
                static_epoll::StaticEpollInlineWireResult::Handoff
                | static_epoll::StaticEpollInlineWireResult::NoMatch => {}
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

    /// Cap015 / WAF-KEEPALIVE-001: header-phase WAF for each epoll keepalive request.
    ///
    /// Returns reject response bytes when the request must be blocked; `None` to continue.
    /// Callers must invoke this on **every** request head, including keepalive subsequent
    /// requests (inspecting only the first request on a TCP connection is insufficient).
    pub(crate) fn evaluate_wire_waf_reject_bytes(
        &self,
        generation: u64,
        head: &[u8],
        peer: SocketAddr,
    ) -> Option<Vec<u8>> {
        // Root3 / ARCH-002: Cap067 P1 WAF-off must not pay `read_state` (RwLock + Arc clone)
        // solely to discover that wire materialization is inactive.
        // Fail-closed:
        // - while structural reload is in progress, always load ServerState (LA-CAP067-R3-001)
        // - re-sample published flag after reload_in_progress so a concurrent WAF-enable
        //   cannot complete between the two loads and skip inspect (LA-CAP067-R3-002)
        if !reload::waf_wire_inspection_active_published()
            && !reload::reload_in_progress()
            && !reload::waf_wire_inspection_active_published()
        {
            return None;
        }
        let state = reload::read_state(&self.shared);
        // Cap015: when WAF is active this still runs on every keepalive request.
        if !waf_wire_materialization_active(&state) {
            return None;
        }
        let xff = HeaderValue::from_str(&peer.ip().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("127.0.0.1"));
        match evaluate_wire_waf(&state, generation, head, &xff) {
            crate::waf::WafHookResult::Reject(reject) => Some(format_waf_reject_http(&reject)),
            crate::waf::WafHookResult::Continue => None,
        }
    }

    /// I6: admit once + pin once for epoll accept (Hyper vs stay-in-map).
    pub(crate) fn admit_epoll_accept(
        &self,
        force_hyper: bool,
    ) -> Result<EpollAdmitDecision, DrainRejected> {
        // Cap067 P5: HyperReady (`GET /api/`, Connection:close) must not pay
        // `read_state` (RwLock + Arc clone) on the accept thread — that serializes
        // Cap067 workers and collapses effective concurrency under wrk -c100.
        // Cheap pin: generation from the published atomic; Hyper dispatch reloads
        // ServerState on the Tokio task. WAF enforce still forces Hyper via the
        // StayInMap path below (peek may say stay, WAF says Hyper).
        if force_hyper {
            let token = PlatformConnectionAdmission::try_admit(&self.ops)?;
            return Ok(EpollAdmitDecision::HyperReady(EpollAdmitAttach {
                token,
                bench_cache: SyncBenchCache {
                    site_static_slot: None,
                    modules_enabled: true,
                    generation: reload::active_runtime_generation(),
                },
            }));
        }

        let attach = self.admit_for_epoll_interest()?;
        // Cap015 / WAF-LOGIC-P1-E: when WAF is enforcing, keep Hyper for accept-time
        // handoff. Monitor/disabled WAF stays on Cap067 — keepalive re-enters WAF via
        // [`Self::evaluate_wire_waf_reject_bytes`] (WAF-KEEPALIVE-001).
        //
        // `modules_enabled` must NOT force Hyper: Cap067 `EPOLL_LISTEN` owns static
        // sendfile with modules compiled in; non-static (/api/, NeedHyper) hand off
        // per-request. Divert Tokio→Cap067 already uses `admit_for_epoll_interest`
        // without this gate — listen must match.
        let state = reload::read_state(&self.shared);
        let waf_requires_hyper = state.waf_enforce;
        if waf_requires_hyper {
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
    ///
    /// Cap067 Root3: uses process-visible [`reload::active_runtime_generation`] (AtomicU64),
    /// the same authority Hyper uses — not per-request `read_state` (RwLock + Arc::clone).
    pub(crate) fn is_generation_stale(&self, pinned_generation: u64) -> bool {
        reload::active_runtime_generation() != pinned_generation
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

    fn hyper_required_from_shared(&self) -> bool {
        reload::read_state(&self.shared).hyper_required_for_modules()
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
        let exec = CoreConnectionExecutor::new(
            shared,
            proxy.clone(),
            Arc::clone(&ops),
            rt.handle().clone(),
        );
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
            proxy.clone(),
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
        // Hyper task may already have finished under load; never allow >1.
        assert!(ops.active_connections() <= 1);
        let _ = client.join();
        rt.block_on(async {
            for _ in 0..100 {
                if ops.active_connections() == 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        });
        assert_eq!(ops.active_connections(), 0);
    }
}
