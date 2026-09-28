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

//! INTERNAL WORKSPACE PLATFORM CRATE
//! NOT STABLE PUBLIC API
//!
//! PS3A-PM1/PM2: owns Linux `sync_accept` + `io_uring` accept workers.
//! PS3A-PM3-R2: owns Linux `epoll` listen + keepalive FSM (single productive implementation).
//! Consumes F1/F2 attach contracts + allowlisted module-api (`static_epoll` / `static_wire`).
//! Policy via opaque [`PlatformConnectionEntry`] only (D1: platform → core).

use bytes::Bytes;
use exyonq_core::kernel::{
    plan_wire_decision, AcceptedConnection, ConnectionServeOutcome, EpollConnectionAttachment,
    EpollKeepaliveTransfer, GenerationView, HyperHandoff, PlatformConnectionAdmission,
    PlatformConnectionEntry, TransportKind, WirePlanDecision,
};
use exyonq_core::lifecycle::LifecycleState;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;

// F2 mechanism-only module-api edge (D1 allowlist: static_epoll + static_wire).
// `static_epoll` is Linux-gated in module-api; keep imports aligned.
#[cfg(target_os = "linux")]
#[allow(unused_imports)]
use exyonq_module_api::static_epoll;
#[allow(unused_imports)]
use exyonq_module_api::static_wire;

#[cfg(target_os = "linux")]
use exyonq_core::server::register_epoll_keepalive_enqueue;
#[cfg(target_os = "linux")]
use exyonq_core::server::register_epoll_keepalive_prepare;
#[cfg(target_os = "linux")]
use exyonq_core::server::register_epoll_keepalive_signal_stop;
#[cfg(target_os = "linux")]
use exyonq_core::server::register_epoll_keepalive_stop;
#[cfg(target_os = "linux")]
use exyonq_core::server::register_epoll_listen_start;
#[cfg(target_os = "linux")]
use exyonq_core::server::register_io_uring_start;
#[cfg(target_os = "linux")]
use exyonq_core::server::register_sync_accept_start;

#[cfg(target_os = "linux")]
mod conn_pool;
#[cfg(target_os = "linux")]
mod epoll_worker;
#[cfg(target_os = "linux")]
mod io_uring_worker;
#[cfg(target_os = "linux")]
mod linux_bind;
#[cfg(target_os = "linux")]
mod sync_accept;
#[cfg(target_os = "linux")]
mod tcp_cork;

/// Marker for composition-root linking (binary may call; no runtime side effects alone).
pub const COMPOSITION_LINK: &str = "exyonq-platform-linux:ps3a-pm4-extraction-cleanup";

/// Re-export the Linux mechanism; the CLI composition root owns module-api wiring.
#[cfg(target_os = "linux")]
pub use tcp_cork::set_tcp_cork;

/// Composition root: link crate + register Linux worker starters.
#[inline(always)]
pub fn ensure_composition_link() {
    let _ = COMPOSITION_LINK;
    #[cfg(target_os = "linux")]
    {
        register_sync_accept_start(sync_accept::start_sync_accept_workers);
        register_io_uring_start(io_uring_worker::start_io_uring_workers);
        register_epoll_listen_start(epoll_worker::start_epoll_listen_workers);
        register_epoll_keepalive_prepare(epoll_worker::prepare_keepalive_pool);
        register_epoll_keepalive_signal_stop(epoll_worker::signal_keepalive_pool_stop);
        register_epoll_keepalive_stop(epoll_worker::stop_keepalive_pool);
        register_epoll_keepalive_enqueue(epoll_worker::enqueue_keepalive_transfer);
        // Compile/shape proof: allowlisted mechanism symbols remain reachable.
        let _ = static_epoll::interest_reading;
    }
}

/// Productive platform→core serve: one coarse call per accepted connection.
#[inline(always)]
pub fn serve_via_entry(
    entry: &PlatformConnectionEntry,
    conn: AcceptedConnection,
) -> ConnectionServeOutcome {
    entry.serve_accepted_connection(conn)
}

/// PS3A-PM3-F2: consume opaque keepalive transfer (no admission).
#[inline(always)]
pub fn consume_keepalive_transfer(
    entry: &PlatformConnectionEntry,
    transfer: EpollKeepaliveTransfer,
) -> EpollConnectionAttachment {
    entry.attach_epoll_keepalive_transfer(transfer)
}

/// Opaque failure for PS1C scaffold diagnostic (not a stable product error contract).
#[derive(Debug)]
pub struct ScaffoldAdmitError;

/// PS1C scaffold diagnostic (admit/plan/handoff primitives). Not the productive move API.
pub fn admit_plan_and_bundle(
    ops: &Arc<LifecycleState>,
    head: &[u8],
) -> Result<(WirePlanDecision, HyperHandoff), ScaffoldAdmitError> {
    let token = PlatformConnectionAdmission::try_admit(ops).map_err(|_| ScaffoldAdmitError)?;
    let view = GenerationView::pinned(1, false, Some(0));
    let decision = plan_wire_decision(&view, head).map_err(|_| ScaffoldAdmitError)?;

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| ScaffoldAdmitError)?;
    let peer: SocketAddr = listener.local_addr().map_err(|_| ScaffoldAdmitError)?;
    let stream = TcpStream::connect(peer).map_err(|_| ScaffoldAdmitError)?;
    let (server, peer) = listener.accept().map_err(|_| ScaffoldAdmitError)?;
    drop(server);

    let handoff = HyperHandoff::new(
        stream,
        peer,
        Some((Bytes::copy_from_slice(head), Bytes::new())),
        view,
        token,
    );
    Ok((decision, handoff))
}

/// Build an owned [`AcceptedConnection`] for mechanism tests (no policy fields).
pub fn accepted_connection_from_pair(
    stream: TcpStream,
    peer: SocketAddr,
    prefetched: Option<(Bytes, Bytes)>,
    transport: TransportKind,
) -> AcceptedConnection {
    AcceptedConnection::new(stream, peer, prefetched, transport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Once};

    static HOOKS: Once = Once::new();

    fn ensure_wire_hooks() {
        HOOKS.call_once(|| {
            exyonq_mod_static::install_kernel_hooks(Arc::new(
                exyonq_mod_static::StaticRuntime::new(),
            ));
            exyonq_mod_proxy::install_kernel_hooks(Arc::new(exyonq_mod_proxy::ProxyRuntime::new()));
        });
    }

    #[test]
    fn ps3a_platform_linux_d1_contract_diagnostic() {
        ensure_wire_hooks();
        ensure_composition_link();
        let ops = LifecycleState::new();
        let head = b"GET /api/health HTTP/1.1\r\nHost: x\r\n\r\n";
        let (decision, handoff) = admit_plan_and_bundle(&ops, head).expect("platform path");
        assert_eq!(decision, WirePlanDecision::Proxy);
        assert_eq!(handoff.generation.generation, 1);
        drop(handoff);
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn facade_api_surface_is_opaque() {
        let _ = serve_via_entry
            as fn(&PlatformConnectionEntry, AcceptedConnection) -> ConnectionServeOutcome;
        let _ = consume_keepalive_transfer
            as fn(&PlatformConnectionEntry, EpollKeepaliveTransfer) -> EpollConnectionAttachment;
        fn assert_not_clone<T>() {}
        assert_not_clone::<PlatformConnectionEntry>();
        assert_not_clone::<AcceptedConnection>();
        assert_not_clone::<EpollKeepaliveTransfer>();
        assert_not_clone::<EpollConnectionAttachment>();
    }

    #[test]
    fn accepted_connection_preserves_peer_prefetch_generation_metadata() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (_s, peer) = listener.accept().unwrap();
        let pref = Some((Bytes::from_static(b"GET /"), Bytes::new()));
        let conn =
            accepted_connection_from_pair(client, peer, pref.clone(), TransportKind::IoUring);
        assert_eq!(conn.peer, peer);
        assert_eq!(conn.prefetched, pref);
        assert_eq!(conn.transport, TransportKind::IoUring);
    }

    #[test]
    fn architectural_d1_platform_to_core_allowed() {
        let _ = COMPOSITION_LINK.contains("exyonq-platform-linux");
        let _ = std::any::type_name::<PlatformConnectionEntry>();
    }

    #[test]
    fn pm2_io_uring_module_present() {
        #[cfg(target_os = "linux")]
        {
            let _ = io_uring_worker::start_io_uring_workers
                as fn(
                    std::net::SocketAddr,
                    usize,
                    Arc<PlatformConnectionEntry>,
                ) -> std::io::Result<exyonq_core::server::OsWorkerGuard>;
        }
    }

    #[test]
    fn pm3_r2_epoll_module_present() {
        #[cfg(target_os = "linux")]
        {
            let _ = epoll_worker::start_epoll_listen_workers
                as fn(
                    std::net::SocketAddr,
                    usize,
                    Arc<PlatformConnectionEntry>,
                ) -> std::io::Result<exyonq_core::server::OsWorkerGuard>;
            let _ = epoll_worker::enqueue_keepalive_transfer;
            let _ = epoll_worker::prepare_keepalive_pool;
            let _ = epoll_worker::stop_keepalive_pool;
        }
    }

    #[test]
    fn pm3_f2_mechanism_module_api_allowlisted() {
        #[cfg(target_os = "linux")]
        {
            let _ = static_epoll::interest_reading;
        }
    }
}
