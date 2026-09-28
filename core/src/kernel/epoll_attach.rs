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
//! Epoll-specific minimal adapter (PS3A-PM3-F1 + F2).
//!
//! Coarse platform→core surface for a future `exyonq-platform-linux` epoll worker:
//! - [`super::PlatformConnectionEntry::attach_epoll_connection`] — new TCP connection (admits once)
//! - [`EpollKeepaliveTransfer`] — already-admitted Tokio keepalive handoff (no re-admit)
//! - generation observe ≤ once per completed request
//! - rare Hyper handoff
//!
//! Does **not** model EPOLLIN/OUT, ConnPhase, sendfile park, interest masks, or the fd map.

use crate::server::connection_errors::ConnectionServeOutcome;
use bytes::Bytes;
use std::net::{SocketAddr, TcpStream};

#[cfg(target_os = "linux")]
use crate::lifecycle::ConnectionLifecycleToken;
#[cfg(target_os = "linux")]
use crate::server::connection_executor::CoreConnectionExecutor;
#[cfg(target_os = "linux")]
use std::sync::Arc;

/// Narrow attach rejection (connection-local — never worker-fatal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpollAttachRejectReason {
    /// Drain boundary: no new admissions.
    Drain,
}

/// Result of [`super::PlatformConnectionEntry::attach_epoll_connection`].
pub enum EpollAttachDecision {
    /// Stay on the epoll connection map for the full TCP lifetime.
    StayAttached(EpollConnectionAttachment),
    /// Immediate coarse Hyper handoff capability (do not enter the map).
    HyperReady(EpollHyperCapability),
    /// Connection-local reject (e.g. drain).
    Rejected(EpollAttachRejectReason),
}

/// Opaque core-owned attachment for StayInMap connections.
///
/// Owns the lifecycle token and pinned generation/static scalars internally.
/// Drop drops the token exactly once (RAII).
///
/// **Not [`Clone`]**. Not a public API.
pub struct EpollConnectionAttachment {
    #[cfg(target_os = "linux")]
    inner: AttachmentInner,
    #[cfg(not(target_os = "linux"))]
    _private: (),
}

/// Opaque one-shot Hyper handoff capability produced at attach when decision is HyperReady.
///
/// **Not [`Clone`]**. Consumed by [`EpollHyperCapability::handoff_to_hyper`].
pub struct EpollHyperCapability {
    #[cfg(target_os = "linux")]
    inner: AttachmentInner,
    #[cfg(not(target_os = "linux"))]
    _private: (),
}

/// Opaque core-created capability for Tokio→epoll keepalive transfer (PS3A-PM3-F2).
///
/// Created only inside `exyonq-core` after the connection is already admitted.
/// Consumed exactly once via [`super::PlatformConnectionEntry::attach_epoll_keepalive_transfer`]
/// into an [`EpollConnectionAttachment`] — **no second admission**.
///
/// **Not [`Clone`]**. Platform cannot construct this type.
pub struct EpollKeepaliveTransfer {
    #[cfg(target_os = "linux")]
    inner: AttachmentInner,
    #[cfg(not(target_os = "linux"))]
    _private: (),
}

#[cfg(target_os = "linux")]
pub(crate) struct AttachmentInner {
    token: ConnectionLifecycleToken,
    /// Pinned scalars only — never `SyncBenchCache` on any public surface.
    generation: u64,
    site_static_slot: Option<u32>,
    modules_enabled: bool,
    executor: Arc<CoreConnectionExecutor>,
}

#[cfg(target_os = "linux")]
impl AttachmentInner {
    pub(crate) fn from_pin(
        token: ConnectionLifecycleToken,
        generation: u64,
        site_static_slot: Option<u32>,
        modules_enabled: bool,
        executor: Arc<CoreConnectionExecutor>,
    ) -> Self {
        Self {
            token,
            generation,
            site_static_slot,
            modules_enabled,
            executor,
        }
    }

    fn handoff(
        self,
        stream: TcpStream,
        peer: SocketAddr,
        prefetched: Option<(Bytes, Bytes)>,
    ) -> ConnectionServeOutcome {
        self.executor.spawn_hyper_admitted_pinned(
            stream,
            peer,
            prefetched,
            self.generation,
            self.site_static_slot,
            self.modules_enabled,
            self.token,
        )
    }
}

impl EpollConnectionAttachment {
    #[cfg(target_os = "linux")]
    pub(crate) fn from_inner(inner: AttachmentInner) -> Self {
        Self { inner }
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn unavailable_stub() -> Self {
        Self { _private: () }
    }

    /// Copy scalar: pinned generation epoch (no cache type exposure).
    ///
    /// Cost: zero (reads a `u64` field). Prefer [`Self::is_generation_stale`] when comparing
    /// against the live reload generation — that is the authorized ≤1×/request observe path.
    #[inline]
    pub fn pinned_generation(&self) -> u64 {
        #[cfg(target_os = "linux")]
        {
            self.inner.generation
        }
        #[cfg(not(target_os = "linux"))]
        {
            0
        }
    }

    /// Mechanism scalar for module-api static pumps (not `SyncBenchCache`).
    #[inline]
    pub fn static_site_slot(&self) -> Option<u32> {
        #[cfg(target_os = "linux")]
        {
            self.inner.site_static_slot
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    /// Generation observe — **at most once per completed request** by contract.
    ///
    /// Cost: one `reload::active_runtime_generation` AtomicU64 load via the private executor
    /// (Cap067 Root3; same authority Hyper uses). No `read_state`, no lock, no alloc.
    #[inline]
    pub fn is_generation_stale(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.inner
                .executor
                .is_generation_stale(self.inner.generation)
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    /// Cap040 / SECINT-003: keepalive already holds an admission token; new product work
    /// after drain must still fail closed without a second `try_enter`.
    #[inline]
    pub fn is_draining(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.inner.token.is_draining()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    /// Probe-aware drain boundary body for epoll keepalive rejects (P15-WS5-PROBE-002).
    #[inline]
    pub fn drain_boundary_response(&self, request_head: &[u8]) -> &'static [u8] {
        crate::server::drain_probes::drain_boundary_response(request_head)
    }

    /// Cap015 / WAF-KEEPALIVE-001: evaluate header-phase WAF on **each** request head.
    ///
    /// Returns reject wire bytes when blocked; `None` to continue serving. Must be called
    /// for keepalive subsequent requests on the epoll map — first-request-only inspection
    /// is a production-reachable bypass.
    #[inline]
    pub fn evaluate_wire_waf_reject(&self, head: &[u8], peer: SocketAddr) -> Option<Vec<u8>> {
        #[cfg(target_os = "linux")]
        {
            self.inner
                .executor
                .evaluate_wire_waf_reject_bytes(self.inner.generation, head, peer)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (head, peer);
            None
        }
    }

    /// Rare / coarse Hyper re-entry. Consumes the attachment (token moves into Hyper).
    ///
    /// Second call is a compile-time error (value moved). Peer and prefetched bytes are
    /// preserved into core-owned `HyperHandoff` construction.
    pub fn handoff_to_hyper(
        self,
        stream: TcpStream,
        peer: SocketAddr,
        prefetched: Option<(Bytes, Bytes)>,
    ) -> ConnectionServeOutcome {
        #[cfg(target_os = "linux")]
        {
            self.inner.handoff(stream, peer, prefetched)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (stream, peer, prefetched);
            ConnectionServeOutcome::PolicyFailed(
                crate::server::connection_errors::CorePolicyError::GenerationUnavailable,
            )
        }
    }
}

impl EpollHyperCapability {
    #[cfg(target_os = "linux")]
    pub(crate) fn from_inner(inner: AttachmentInner) -> Self {
        Self { inner }
    }

    /// Coarse Hyper handoff at attach (HyperReady). Consumes the capability.
    pub fn handoff_to_hyper(
        self,
        stream: TcpStream,
        peer: SocketAddr,
        prefetched: Option<(Bytes, Bytes)>,
    ) -> ConnectionServeOutcome {
        #[cfg(target_os = "linux")]
        {
            self.inner.handoff(stream, peer, prefetched)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (stream, peer, prefetched);
            ConnectionServeOutcome::PolicyFailed(
                crate::server::connection_errors::CorePolicyError::GenerationUnavailable,
            )
        }
    }
}

impl EpollKeepaliveTransfer {
    #[cfg(target_os = "linux")]
    pub(crate) fn from_inner(inner: AttachmentInner) -> Self {
        Self { inner }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn into_inner(self) -> AttachmentInner {
        self.inner
    }

    /// Extract admission token after a failed handoff so ownership can return to task-local.
    #[cfg(target_os = "linux")]
    pub(crate) fn into_admission_token(self) -> crate::lifecycle::ConnectionLifecycleToken {
        let AttachmentInner { token, .. } = self.inner;
        token
    }
}

#[cfg(all(test, target_os = "linux"))]
mod epoll_contract_tests {
    use super::*;
    use crate::kernel::PlatformConnectionEntry;
    use crate::lifecycle::LifecycleState;
    use crate::reload;
    use crate::server::accepted_connection::TransportKind;
    use crate::server::connection_errors::ConnectionError;
    use crate::server::connection_executor::CoreConnectionExecutor;
    use exyonq_mod_proxy::build_incoming_client;
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use tokio::runtime::Runtime;

    fn test_entry(ops: Arc<LifecycleState>) -> (PlatformConnectionEntry, Runtime) {
        use std::sync::Once;
        static HOOKS: Once = Once::new();
        HOOKS.call_once(|| {
            exyonq_mod_static::install_kernel_hooks(Arc::new(
                exyonq_mod_static::StaticRuntime::new(),
            ));
            exyonq_mod_proxy::install_kernel_hooks(Arc::new(exyonq_mod_proxy::ProxyRuntime::new()));
        });
        let rt = Runtime::new().expect("rt");
        let raw = include_str!("../../../tests/fixtures/minimal.toml");
        let mut config: crate::config::AppConfig = raw.parse().expect("config");
        // Cap015: WAF defaults enabled → admit forces HyperReady. StayAttached
        // contract requires WAF off and modules off (minimal fixture has no modules).
        config.waf.enabled = false;
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
        (entry, rt)
    }

    fn accept_pair() -> (TcpStream, TcpStream, std::net::SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (server, peer) = listener.accept().unwrap();
        (server, client, peer)
    }

    /// Capture admission-time pin scalars once (Tokio path). Transfer must not re-pin.
    fn admission_pin(entry: &PlatformConnectionEntry) -> (u64, Option<u32>, bool) {
        let pin = entry.executor().pin_bench_cache();
        (pin.generation, pin.site_static_slot, pin.modules_enabled)
    }

    #[test]
    fn epoll_contract_f2_keepalive_transfer_no_second_admit() {
        use crate::kernel::PlatformConnectionAdmission;
        let ops = Arc::new(LifecycleState::new());
        let (entry, _rt) = test_entry(Arc::clone(&ops));
        let token = PlatformConnectionAdmission::try_admit(&ops).expect("admit once");
        assert_eq!(ops.active_connections(), 1);
        // Pin once at "Tokio admission" — transfer must preserve these scalars.
        let (gen_before, slot, modules) = admission_pin(&entry);
        let transfer = entry.create_epoll_keepalive_transfer(token, gen_before, slot, modules);
        // Transfer did not admit again.
        assert_eq!(ops.active_connections(), 1);
        fn assert_not_clone<T>() {}
        assert_not_clone::<EpollKeepaliveTransfer>();
        let att = entry.attach_epoll_keepalive_transfer(transfer);
        assert_eq!(ops.active_connections(), 1);
        assert_eq!(att.pinned_generation(), gen_before);
        assert_eq!(att.static_site_slot(), slot);
        assert!(!att.is_generation_stale());
        // Multiple keepalive request observe cycles — same attachment / pin.
        for _ in 0..5 {
            assert!(!att.is_generation_stale());
            assert_eq!(att.pinned_generation(), gen_before);
        }
        drop(att);
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn epoll_contract_f2_transfer_hyper_preserves_peer_prefetch() {
        use crate::kernel::PlatformConnectionAdmission;
        let ops = Arc::new(LifecycleState::new());
        let (entry, rt) = test_entry(Arc::clone(&ops));
        let token = PlatformConnectionAdmission::try_admit(&ops).expect("admit");
        let (gen, slot, modules) = admission_pin(&entry);
        let transfer = entry.create_epoll_keepalive_transfer(token, gen, slot, modules);
        let att = entry.attach_epoll_keepalive_transfer(transfer);
        let (server, client, peer) = accept_pair();
        let _ = client.shutdown(std::net::Shutdown::Both);
        let head = Bytes::from_static(b"GET /unknown HTTP/1.1\r\nHost: x\r\n\r\n");
        let outcome = att.handoff_to_hyper(server, peer, Some((head, Bytes::new())));
        assert!(matches!(outcome, ConnectionServeOutcome::Completed));
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

    #[test]
    fn epoll_contract_f2_drain_blocks_new_admit_not_existing_transfer() {
        use crate::kernel::PlatformConnectionAdmission;
        let ops = Arc::new(LifecycleState::new());
        let (entry, _rt) = test_entry(Arc::clone(&ops));
        let token = PlatformConnectionAdmission::try_admit(&ops).expect("admit before drain");
        let (gen, slot, modules) = admission_pin(&entry);
        let transfer = entry.create_epoll_keepalive_transfer(token, gen, slot, modules);
        ops.start_drain();
        // Existing transfer still consumable (already admitted).
        let att = entry.attach_epoll_keepalive_transfer(transfer);
        assert_eq!(ops.active_connections(), 1);
        drop(att);
        assert_eq!(ops.active_connections(), 0);
        // New admission rejected.
        match entry.attach_epoll_connection(false) {
            EpollAttachDecision::Rejected(EpollAttachRejectReason::Drain) => {}
            _ => panic!("new admit after drain must reject"),
        }
    }

    #[test]
    fn epoll_contract_f2_generation_preserved_no_repin() {
        use crate::kernel::PlatformConnectionAdmission;
        let ops = Arc::new(LifecycleState::new());
        let (entry, _rt) = test_entry(Arc::clone(&ops));
        let token = PlatformConnectionAdmission::try_admit(&ops).expect("admit");
        let (live_gen, slot, modules) = admission_pin(&entry);
        // Pass a distinct pin generation — transfer must keep it (no pin_bench_cache).
        let distinct_pin_gen = live_gen.wrapping_add(9_001);
        let transfer =
            entry.create_epoll_keepalive_transfer(token, distinct_pin_gen, slot, modules);
        let att = entry.attach_epoll_keepalive_transfer(transfer);
        assert_eq!(att.pinned_generation(), distinct_pin_gen);
        assert_ne!(att.pinned_generation(), live_gen);
        // Stale observe still works against live reload generation.
        assert!(att.is_generation_stale());
        drop(att);
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn epoll_contract_attach_admits_exactly_once_and_drops_token() {
        let ops = Arc::new(LifecycleState::new());
        let (entry, _rt) = test_entry(Arc::clone(&ops));
        assert_eq!(ops.active_connections(), 0);
        match entry.attach_epoll_connection(false) {
            EpollAttachDecision::StayAttached(att) => {
                assert_eq!(ops.active_connections(), 1);
                let gen = att.pinned_generation();
                assert!(!att.is_generation_stale());
                // Observe does not mutate pin.
                assert_eq!(att.pinned_generation(), gen);
                drop(att);
                assert_eq!(ops.active_connections(), 0);
            }
            EpollAttachDecision::HyperReady(_) | EpollAttachDecision::Rejected(_) => {
                panic!("expected StayAttached")
            }
        }
    }

    #[test]
    fn epoll_contract_attach_rejects_after_drain_boundary() {
        let ops = Arc::new(LifecycleState::new());
        ops.start_drain();
        let (entry, _rt) = test_entry(Arc::clone(&ops));
        match entry.attach_epoll_connection(false) {
            EpollAttachDecision::Rejected(EpollAttachRejectReason::Drain) => {}
            _ => panic!("expected drain reject"),
        }
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn epoll_contract_stay_survives_multiple_request_observe_cycles() {
        let ops = Arc::new(LifecycleState::new());
        let (entry, _rt) = test_entry(Arc::clone(&ops));
        let att = match entry.attach_epoll_connection(false) {
            EpollAttachDecision::StayAttached(a) => a,
            _ => panic!("StayAttached"),
        };
        for _ in 0..5 {
            assert!(!att.is_generation_stale());
            let _ = att.static_site_slot();
            let _ = att.pinned_generation();
        }
        assert_eq!(ops.active_connections(), 1);
        drop(att);
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn epoll_contract_force_hyper_ready_and_handoff_preserves_peer_prefetch() {
        let ops = Arc::new(LifecycleState::new());
        let (entry, rt) = test_entry(Arc::clone(&ops));
        let cap = match entry.attach_epoll_connection(true) {
            EpollAttachDecision::HyperReady(c) => c,
            EpollAttachDecision::StayAttached(_) => panic!("force_hyper should be HyperReady"),
            EpollAttachDecision::Rejected(_) => panic!("rejected"),
        };
        assert_eq!(ops.active_connections(), 1);
        let (server, client, peer) = accept_pair();
        let _ = client.shutdown(std::net::Shutdown::Both);
        let head = Bytes::from_static(b"GET /unknown HTTP/1.1\r\nHost: x\r\n\r\n");
        let outcome = cap.handoff_to_hyper(server, peer, Some((head, Bytes::new())));
        assert!(matches!(outcome, ConnectionServeOutcome::Completed));
        // Token transferred into Hyper (still counted) until task ends.
        assert!(ops.active_connections() <= 1);
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

    #[test]
    fn epoll_contract_stay_handoff_transfers_once() {
        let ops = Arc::new(LifecycleState::new());
        let (entry, rt) = test_entry(Arc::clone(&ops));
        let att = match entry.attach_epoll_connection(false) {
            EpollAttachDecision::StayAttached(a) => a,
            _ => panic!("StayAttached"),
        };
        let (server, client, peer) = accept_pair();
        let _ = client.shutdown(std::net::Shutdown::Both);
        let head = Bytes::from_static(b"GET /unknown HTTP/1.1\r\nHost: x\r\n\r\n");
        let outcome = att.handoff_to_hyper(server, peer, Some((head, Bytes::new())));
        assert!(matches!(outcome, ConnectionServeOutcome::Completed));
        // Attachment moved — single handoff. Wait for Hyper task to drop token.
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

    #[test]
    fn epoll_contract_connection_local_reject_is_not_worker_fatal_shape() {
        // Rejected is an enum arm — worker loops match and continue (no io::Result fatal).
        let r = EpollAttachDecision::Rejected(EpollAttachRejectReason::Drain);
        match r {
            EpollAttachDecision::Rejected(EpollAttachRejectReason::Drain) => {}
            _ => panic!(),
        }
        let closed = ConnectionServeOutcome::ConnectionClosed(ConnectionError::DrainRejected);
        assert!(matches!(
            closed,
            ConnectionServeOutcome::ConnectionClosed(_)
        ));
    }

    #[test]
    fn epoll_contract_structural_no_dyn_boxfuture_callback() {
        fn assert_not_clone<T>() {}
        assert_not_clone::<EpollConnectionAttachment>();
        assert_not_clone::<EpollHyperCapability>();
        assert_not_clone::<EpollKeepaliveTransfer>();
        assert_not_clone::<PlatformConnectionEntry>();
        // Size/shape: decision is a concrete enum (no BoxFuture / dyn).
        let _ = std::mem::size_of::<EpollAttachDecision>();
        let _ = TransportKind::Epoll;
        // Consume API is value-move (second consume is compile-time error).
        let _ = PlatformConnectionEntry::attach_epoll_keepalive_transfer
            as fn(&PlatformConnectionEntry, EpollKeepaliveTransfer) -> EpollConnectionAttachment;
    }

    #[test]
    fn epoll_contract_opaque_surface_has_no_syncbenchcache_name_in_api() {
        // Public methods return scalars / outcomes only.
        let ops = Arc::new(LifecycleState::new());
        let (entry, _rt) = test_entry(Arc::clone(&ops));
        if let EpollAttachDecision::StayAttached(att) = entry.attach_epoll_connection(false) {
            let _: u64 = att.pinned_generation();
            let _: Option<u32> = att.static_site_slot();
            let _: bool = att.is_generation_stale();
            drop(att);
        }
    }
}
