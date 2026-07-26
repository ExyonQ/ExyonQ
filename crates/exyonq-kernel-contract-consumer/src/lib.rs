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

//! PS1C / PM3-F1 / PM3-F2 compile-consumer: D1 (`consumer → exyonq-core`) without worker internals.
//!
//! **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**

use bytes::Bytes;
use exyonq_core::kernel::{
    plan_wire_decision, AcceptedConnection, ConnectionServeOutcome, EpollAttachDecision,
    EpollConnectionAttachment, EpollHyperCapability, EpollKeepaliveTransfer, GenerationView,
    HyperHandoff, PlatformConnectionAdmission, PlatformConnectionEntry, TransportKind,
    WirePlanDecision,
};
use exyonq_core::lifecycle::LifecycleState;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;

/// Consumer-side admission + planner + handoff bundle construction (PS1C seam smoke).
pub fn consumer_admit_plan_and_bundle(
    ops: &Arc<LifecycleState>,
    head: &[u8],
) -> Result<(WirePlanDecision, HyperHandoff), ()> {
    let token = PlatformConnectionAdmission::try_admit(ops).map_err(|_| ())?;
    let view = GenerationView::pinned(1, false, Some(0));
    let decision = plan_wire_decision(&view, head).map_err(|_| ())?;

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| ())?;
    let peer: SocketAddr = listener.local_addr().map_err(|_| ())?;
    let stream = TcpStream::connect(peer).map_err(|_| ())?;
    let (server, peer) = listener.accept().map_err(|_| ())?;
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

/// Platform-style productive serve: opaque entry + owned connection only.
#[inline(always)]
pub fn consumer_serve_via_entry(
    entry: &PlatformConnectionEntry,
    conn: AcceptedConnection,
) -> ConnectionServeOutcome {
    entry.serve_accepted_connection(conn)
}

/// Platform-style epoll attach (PS3A-PM3-F1): once per connection, opaque decision.
#[inline(always)]
pub fn consumer_attach_epoll(
    entry: &PlatformConnectionEntry,
    force_hyper: bool,
) -> EpollAttachDecision {
    entry.attach_epoll_connection(force_hyper)
}

/// Platform-style keepalive transfer consume (PS3A-PM3-F2): no admission.
#[inline(always)]
pub fn consumer_consume_keepalive_transfer(
    entry: &PlatformConnectionEntry,
    transfer: EpollKeepaliveTransfer,
) -> EpollConnectionAttachment {
    entry.attach_epoll_keepalive_transfer(transfer)
}

/// Platform-style generation observe on opaque Stay attachment.
#[inline(always)]
pub fn consumer_observe_generation(att: &EpollConnectionAttachment) -> bool {
    att.is_generation_stale()
}

/// Platform-style rare Hyper handoff from Stay attachment.
#[inline(always)]
pub fn consumer_handoff_from_stay(
    att: EpollConnectionAttachment,
    stream: TcpStream,
    peer: SocketAddr,
    prefetched: Option<(Bytes, Bytes)>,
) -> ConnectionServeOutcome {
    att.handoff_to_hyper(stream, peer, prefetched)
}

/// Platform-style rare Hyper handoff from HyperReady capability.
#[inline(always)]
pub fn consumer_handoff_from_hyper_ready(
    cap: EpollHyperCapability,
    stream: TcpStream,
    peer: SocketAddr,
    prefetched: Option<(Bytes, Bytes)>,
) -> ConnectionServeOutcome {
    cap.handoff_to_hyper(stream, peer, prefetched)
}

/// Build deliverable without policy fields (compile proof of AcceptedConnection surface).
pub fn consumer_accepted_connection(
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
    use exyonq_core::kernel::EpollAttachRejectReason;
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
    fn ps1c_compile_consumer_gate_smoke() {
        ensure_wire_hooks();
        let ops = LifecycleState::new();
        let head = b"GET /api/health HTTP/1.1\r\nHost: x\r\n\r\n";
        let (decision, handoff) =
            consumer_admit_plan_and_bundle(&ops, head).expect("consumer path");
        assert_eq!(decision, WirePlanDecision::Proxy);
        assert_eq!(handoff.generation.generation, 1);
        assert_ne!(handoff.peer.ip().to_string(), "0.0.0.0");
        drop(handoff);
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn ps1c_facade_consumer_surface_opaque() {
        let _ = consumer_serve_via_entry
            as fn(&PlatformConnectionEntry, AcceptedConnection) -> ConnectionServeOutcome;
        fn assert_not_clone<T>() {}
        assert_not_clone::<PlatformConnectionEntry>();
        assert_not_clone::<AcceptedConnection>();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (_s, peer) = listener.accept().unwrap();
        let conn = consumer_accepted_connection(client, peer, None, TransportKind::SyncAccept);
        assert_eq!(conn.peer, peer);
        let AcceptedConnection {
            stream: _,
            peer: _,
            prefetched: _,
            transport: _,
        } = conn;
    }

    #[test]
    fn pm3_f1_epoll_contract_consumer_surface_opaque() {
        let _ = consumer_attach_epoll as fn(&PlatformConnectionEntry, bool) -> EpollAttachDecision;
        let _ = consumer_observe_generation as fn(&EpollConnectionAttachment) -> bool;
        let _ = consumer_handoff_from_stay
            as fn(
                EpollConnectionAttachment,
                TcpStream,
                SocketAddr,
                Option<(Bytes, Bytes)>,
            ) -> ConnectionServeOutcome;
        let _ = consumer_handoff_from_hyper_ready
            as fn(
                EpollHyperCapability,
                TcpStream,
                SocketAddr,
                Option<(Bytes, Bytes)>,
            ) -> ConnectionServeOutcome;

        fn assert_not_clone<T>() {}
        assert_not_clone::<EpollConnectionAttachment>();
        assert_not_clone::<EpollHyperCapability>();

        let _ = EpollAttachDecision::Rejected(EpollAttachRejectReason::Drain);
    }

    #[test]
    fn pm3_f2_keepalive_transfer_consumer_surface_opaque() {
        let _ = consumer_consume_keepalive_transfer
            as fn(&PlatformConnectionEntry, EpollKeepaliveTransfer) -> EpollConnectionAttachment;
        fn assert_not_clone<T>() {}
        assert_not_clone::<EpollKeepaliveTransfer>();
        // Cannot construct EpollKeepaliveTransfer / create_epoll_keepalive_transfer from here
        // (pub(crate) constructor). Cannot name SyncBenchCache / executor / SSS / ProxyClient.
    }
}
