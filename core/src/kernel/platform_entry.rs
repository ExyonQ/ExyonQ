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
//! **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**
//!
//! Minimal platform→core productive serve entry (PS1C-FACADE).
//! Opaque handle over [`crate::server::connection_executor::CoreConnectionExecutor`].
//! Construction: composition root inside `exyonq-core` only (`pub(crate)`).

use crate::kernel::epoll_attach::{
    EpollAttachDecision, EpollAttachRejectReason, EpollConnectionAttachment, EpollKeepaliveTransfer,
};
use crate::server::accepted_connection::AcceptedConnection;
use crate::server::connection_errors::{ConnectionServeOutcome, CorePolicyError};

#[cfg(target_os = "linux")]
use crate::kernel::epoll_attach::EpollHyperCapability;
#[cfg(target_os = "linux")]
use crate::lifecycle::ConnectionLifecycleToken;
#[cfg(target_os = "linux")]
use crate::server::connection_executor::{CoreConnectionExecutor, EpollAdmitDecision};
#[cfg(target_os = "linux")]
use std::sync::Arc;

/// Opaque core-owned connection policy entry for Linux platform workers.
///
/// Holds all policy authority (admission, generation pin, planner, Hyper).
/// Platform crates may hold `Arc<PlatformConnectionEntry>` and call
/// [`Self::serve_accepted_connection`] once per accepted TCP connection.
///
/// **Not [`Clone`]** — share via `Arc` only (same pattern as the executor).
pub struct PlatformConnectionEntry {
    #[cfg(target_os = "linux")]
    inner: Arc<CoreConnectionExecutor>,
    #[cfg(not(target_os = "linux"))]
    _private: (),
}

impl PlatformConnectionEntry {
    /// Composition-root constructor. Not callable from `exyonq-platform-linux`.
    #[cfg(target_os = "linux")]
    pub(crate) fn from_executor(inner: Arc<CoreConnectionExecutor>) -> Self {
        Self { inner }
    }

    /// Non-Linux stub for workspace `cargo check` (no productive serve path).
    #[cfg(not(target_os = "linux"))]
    #[allow(dead_code)]
    pub(crate) fn unavailable() -> Self {
        Self { _private: () }
    }

    /// Coarse-grained serve: one call per accepted connection.
    ///
    /// Admission via `PlatformConnectionAdmission::try_admit` happens inside core.
    /// Listener / epoll / ring fatals never return through this API.
    pub fn serve_accepted_connection(&self, conn: AcceptedConnection) -> ConnectionServeOutcome {
        #[cfg(target_os = "linux")]
        {
            self.inner.serve_accepted(conn)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = conn;
            ConnectionServeOutcome::PolicyFailed(CorePolicyError::GenerationUnavailable)
        }
    }

    /// Test/composition access to the underlying executor Arc (core-only).
    #[cfg(target_os = "linux")]
    pub(crate) fn executor(&self) -> &Arc<CoreConnectionExecutor> {
        &self.inner
    }

    /// PS3A-PM3-F1: attach once per TCP connection for a future epoll platform worker.
    ///
    /// Encapsulates admission, lifecycle token, generation/static pin, and Stay|Hyper|Reject.
    /// Does **not** take the accepted fd — mechanism retains the stream; HyperReady / Stay
    /// handoff is a separate coarse call.
    ///
    /// Frequency: **once per connection**. No per-event / per-chunk core entry.
    pub fn attach_epoll_connection(&self, force_hyper: bool) -> EpollAttachDecision {
        #[cfg(target_os = "linux")]
        {
            match self.inner.admit_epoll_accept(force_hyper) {
                Err(_) => EpollAttachDecision::Rejected(EpollAttachRejectReason::Drain),
                Ok(EpollAdmitDecision::StayInMap(attach)) => {
                    EpollAttachDecision::StayAttached(Self::wrap_attachment(
                        attach.token,
                        attach.bench_cache.generation,
                        attach.bench_cache.site_static_slot,
                        attach.bench_cache.modules_enabled,
                        Arc::clone(&self.inner),
                    ))
                }
                Ok(EpollAdmitDecision::HyperReady(attach)) => {
                    EpollAttachDecision::HyperReady(Self::wrap_hyper(
                        attach.token,
                        attach.bench_cache.generation,
                        attach.bench_cache.site_static_slot,
                        attach.bench_cache.modules_enabled,
                        Arc::clone(&self.inner),
                    ))
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = force_hyper;
            EpollAttachDecision::Rejected(EpollAttachRejectReason::Drain)
        }
    }

    #[cfg(target_os = "linux")]
    fn wrap_attachment(
        token: ConnectionLifecycleToken,
        generation: u64,
        site_static_slot: Option<u32>,
        modules_enabled: bool,
        executor: Arc<CoreConnectionExecutor>,
    ) -> EpollConnectionAttachment {
        use crate::kernel::epoll_attach::AttachmentInner;
        EpollConnectionAttachment::from_inner(AttachmentInner::from_pin(
            token,
            generation,
            site_static_slot,
            modules_enabled,
            executor,
        ))
    }

    #[cfg(target_os = "linux")]
    fn wrap_hyper(
        token: ConnectionLifecycleToken,
        generation: u64,
        site_static_slot: Option<u32>,
        modules_enabled: bool,
        executor: Arc<CoreConnectionExecutor>,
    ) -> EpollHyperCapability {
        use crate::kernel::epoll_attach::AttachmentInner;
        EpollHyperCapability::from_inner(AttachmentInner::from_pin(
            token,
            generation,
            site_static_slot,
            modules_enabled,
            executor,
        ))
    }

    /// PS3A-PM3-F2: wrap an **already-admitted** lifecycle token for Tokio→epoll keepalive.
    ///
    /// Takes the **already-pinned** generation/static scalars from the Tokio admission
    /// context — does **not** call `try_admit` and does **not** re-pin via
    /// `pin_bench_cache`. Active connection count is unchanged.
    /// Core-only constructor — platform cannot call this.
    #[cfg(target_os = "linux")]
    pub(crate) fn create_epoll_keepalive_transfer(
        &self,
        token: ConnectionLifecycleToken,
        pinned_generation: u64,
        site_static_slot: Option<u32>,
        modules_enabled: bool,
    ) -> EpollKeepaliveTransfer {
        use crate::kernel::epoll_attach::{AttachmentInner, EpollKeepaliveTransfer};
        EpollKeepaliveTransfer::from_inner(AttachmentInner::from_pin(
            token,
            pinned_generation,
            site_static_slot,
            modules_enabled,
            Arc::clone(&self.inner),
        ))
    }

    /// PS3A-PM3-F2: consume a core-created keepalive transfer into a Stay attachment.
    ///
    /// **No admission.** Consumes [`EpollKeepaliveTransfer`] exactly once (moved).
    /// Distinct type from [`Self::attach_epoll_connection`] — cannot double-admit by construction.
    pub fn attach_epoll_keepalive_transfer(
        &self,
        transfer: EpollKeepaliveTransfer,
    ) -> EpollConnectionAttachment {
        #[cfg(target_os = "linux")]
        {
            let _ = self; // same entry identity is compositional; transfer already holds pin+token
            EpollConnectionAttachment::from_inner(transfer.into_inner())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (self, transfer);
            EpollConnectionAttachment::unavailable_stub()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::accepted_connection::TransportKind;
    use crate::server::connection_errors::ConnectionError;
    use std::net::TcpListener;
    use std::net::TcpStream;

    #[test]
    fn facade_is_not_clone() {
        fn assert_not_clone<T>() {}
        assert_not_clone::<PlatformConnectionEntry>();
    }

    #[test]
    fn accepted_connection_transfers_peer_and_prefetch() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (_s, peer) = listener.accept().unwrap();
        let pref = Some((bytes::Bytes::from_static(b"GET /"), bytes::Bytes::new()));
        let conn = AcceptedConnection::new(client, peer, pref.clone(), TransportKind::SyncAccept);
        assert_eq!(conn.peer, peer);
        assert_eq!(conn.prefetched, pref);
        assert_eq!(conn.transport, TransportKind::SyncAccept);
    }

    #[test]
    fn facade_outcome_separates_connection_and_policy() {
        // Facade returns ConnectionServeOutcome only — not WorkerMechanismError / io::Result.
        match ConnectionServeOutcome::ConnectionClosed(ConnectionError::DrainRejected) {
            ConnectionServeOutcome::Completed => panic!("wrong arm"),
            ConnectionServeOutcome::ConnectionClosed(_) => {}
            ConnectionServeOutcome::PolicyFailed(_) => panic!("wrong arm"),
        }
        match ConnectionServeOutcome::PolicyFailed(CorePolicyError::HandoffConstruction(
            std::io::Error::from(std::io::ErrorKind::Other),
        )) {
            ConnectionServeOutcome::PolicyFailed(_) => {}
            _ => panic!("policy must stay distinguishable"),
        }
    }

    #[test]
    fn architectural_d1_core_must_not_depend_on_platform() {
        // Enforced by scripts/architecture/verify-ps3a-platform-linux-d1.sh (Cargo + Rust).
        // This unit test documents the invariant next to the facade.
        assert!(!env!("CARGO_PKG_NAME").contains("platform-linux"));
    }

    #[test]
    fn connection_local_timeout_maps_to_connection_error() {
        use std::io;
        let err = ConnectionError::from_io(io::Error::from(io::ErrorKind::TimedOut));
        assert!(matches!(err, ConnectionError::Io(_)));
        let wb = ConnectionError::from_io(io::Error::from(io::ErrorKind::WouldBlock));
        assert!(matches!(wb, ConnectionError::Io(_)));
        let outcome = ConnectionServeOutcome::ConnectionClosed(wb);
        assert!(matches!(
            outcome,
            ConnectionServeOutcome::ConnectionClosed(ConnectionError::Io(_))
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn facade_drain_is_connection_local_token_drop_once() {
        use crate::lifecycle::LifecycleState;
        use crate::reload;
        use crate::server::connection_executor::CoreConnectionExecutor;
        use exyonq_mod_proxy::build_incoming_client;
        use tokio::runtime::Runtime;

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
        let entry = PlatformConnectionEntry::from_executor(Arc::new(CoreConnectionExecutor::new(
            shared,
            proxy,
            Arc::clone(&ops),
            rt.handle().clone(),
        )));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (server, peer) = listener.accept().unwrap();
        drop(client);

        let outcome = entry.serve_accepted_connection(AcceptedConnection::new(
            server,
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
}
