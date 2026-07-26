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
//! PS3A-F3 I0 — predicate goldens (freeze semantics during the core split).
//!
//! Test-only module. Does not change productive behavior.

#[cfg(test)]
mod tests {
    use crate::kernel::test_hooks::ensure_wire_hooks_installed;
    use crate::kernel::{plan_wire_decision, GenerationView, HyperHandoff, WirePlanDecision};
    use crate::lifecycle::LifecycleState;
    use bytes::Bytes;
    use std::io;
    use std::net::{TcpListener, TcpStream};

    /// Frozen ErrorKind set for CONNECTION_LOCAL (closes connection; worker continues).
    /// Must stay aligned with `io::is_connection_level_io_error` on Linux.
    fn connection_local_kinds() -> &'static [io::ErrorKind] {
        &[
            io::ErrorKind::WouldBlock,
            io::ErrorKind::TimedOut,
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::InvalidData,
        ]
    }

    /// ErrorKinds that must NOT be treated as connection-local (infrastructure / other).
    fn non_connection_local_kinds() -> &'static [io::ErrorKind] {
        &[
            io::ErrorKind::Interrupted, // retried inside blocking_tcp_read; not connection-local if surfaced
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::AddrInUse,
            io::ErrorKind::AddrNotAvailable,
            io::ErrorKind::Other,
        ]
    }

    #[test]
    fn i0_connection_local_kind_set_frozen() {
        assert_eq!(connection_local_kinds().len(), 7);
        for kind in non_connection_local_kinds() {
            assert!(
                !connection_local_kinds().contains(kind),
                "disjoint sets broken for {kind:?}"
            );
        }
        #[cfg(target_os = "linux")]
        {
            use crate::server::io::is_connection_level_io_error;
            for kind in connection_local_kinds() {
                let err = io::Error::from(*kind);
                assert!(
                    is_connection_level_io_error(&err),
                    "expected CONNECTION_LOCAL for {kind:?}"
                );
            }
            for kind in non_connection_local_kinds() {
                let err = io::Error::from(*kind);
                assert!(
                    !is_connection_level_io_error(&err),
                    "expected non-CONNECTION_LOCAL for {kind:?}"
                );
            }
        }
    }

    #[test]
    fn i0_wouldblock_is_connection_local_timeout_class() {
        // SO_RCVTIMEO → WouldBlock/EAGAIN on Linux accepted sockets (IU2).
        assert!(connection_local_kinds().contains(&io::ErrorKind::WouldBlock));
        assert!(connection_local_kinds().contains(&io::ErrorKind::TimedOut));
    }

    #[test]
    fn i0_interrupted_is_not_connection_local_classifier() {
        // EINTR is retried in blocking_tcp_read; classifier must not treat it as CONNECTION_LOCAL.
        assert!(!connection_local_kinds().contains(&io::ErrorKind::Interrupted));
    }

    #[test]
    fn i0_listener_class_errors_are_worker_fatal_not_connection_local() {
        // Bind/listen-style failures are WORKER_FATAL / infrastructure, not CONNECTION_LOCAL.
        for kind in [
            io::ErrorKind::AddrInUse,
            io::ErrorKind::AddrNotAvailable,
            io::ErrorKind::PermissionDenied,
        ] {
            assert!(!connection_local_kinds().contains(&kind));
        }
    }

    #[test]
    fn i0_planner_static_proxy_hyper_goldens() {
        ensure_wire_hooks_installed();
        let bench = GenerationView::pinned(1, false, Some(0));
        let modules = GenerationView::pinned(1, true, Some(0));

        let site = b"GET /site/1k.bin HTTP/1.1\r\nHost: x\r\n\r\n";
        let api = b"GET /api/health HTTP/1.1\r\nHost: x\r\n\r\n";
        let unknown = b"GET /unknown HTTP/1.1\r\nHost: x\r\n\r\n";

        assert_eq!(
            plan_wire_decision(&bench, site).expect("plan"),
            WirePlanDecision::Static
        );
        assert_eq!(
            plan_wire_decision(&bench, api).expect("plan"),
            WirePlanDecision::Proxy
        );
        assert_eq!(
            plan_wire_decision(&bench, unknown).expect("plan"),
            WirePlanDecision::Hyper
        );
        assert_eq!(
            plan_wire_decision(&modules, site).expect("plan"),
            WirePlanDecision::Hyper
        );
        assert!(plan_wire_decision(&bench, b"").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn i0_tokio_accept_required_vs_planner_transport_delta() {
        ensure_wire_hooks_installed();
        use crate::server::wire_dispatch::tokio_accept_required;

        // Transport hint ≠ WirePlanDecision: sendfile sizes force Tokio accept even for /site/.
        assert!(tokio_accept_required(b"GET /api/health HTTP/1.1\r\n"));
        assert!(tokio_accept_required(b"GET /site/64k.bin HTTP/1.1\r\n"));
        assert!(tokio_accept_required(b"GET /site/1m.bin HTTP/1.1\r\n"));
        assert!(!tokio_accept_required(b"GET /site/1k.bin HTTP/1.1\r\n"));

        let bench = GenerationView::pinned(1, false, Some(0));
        // Planner still says Static for 1k site; accept hint for 64k is transport-only.
        assert_eq!(
            plan_wire_decision(&bench, b"GET /site/1k.bin HTTP/1.1\r\nHost: x\r\n\r\n")
                .expect("plan"),
            WirePlanDecision::Static
        );
    }

    #[test]
    fn i0_handoff_preserves_peer_prefetch_generation() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        let generation = GenerationView::pinned(99, false, Some(3));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer_target = listener.local_addr().unwrap();
        let client = TcpStream::connect(peer_target).unwrap();
        let (server, peer) = listener.accept().unwrap();
        drop(server);

        let head = Bytes::from_static(b"GET /x HTTP/1.1\r\nHost: h\r\n\r\n");
        let rest = Bytes::from_static(b"body");
        let handoff = HyperHandoff::new(
            client,
            peer,
            Some((head.clone(), rest.clone())),
            generation,
            token,
        );
        assert_eq!(handoff.peer, peer);
        assert_eq!(handoff.generation.generation, 99);
        assert_eq!(handoff.generation.site_static_slot, Some(3));
        let (h, r) = handoff.prefetched.as_ref().expect("prefetch");
        assert_eq!(h, &head);
        assert_eq!(r, &rest);
        drop(handoff);
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn i0_core_policy_empty_head_is_planning_failure_not_connection_local() {
        ensure_wire_hooks_installed();
        let view = GenerationView::pinned(1, false, Some(0));
        // Empty head → WirePlanningError (CORE_POLICY_FAILURE class), not CONNECTION_LOCAL I/O.
        assert!(plan_wire_decision(&view, b"").is_err());
    }
}
