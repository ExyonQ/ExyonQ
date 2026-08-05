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
//! Blocking-pool eligibility table tests (KD2D — single module authority).

#[cfg(all(test, target_os = "linux"))]
mod linux {
    use crate::wire_eligibility;

    const SMALL_STATIC: &[u8] = b"GET /site/1k.bin HTTP/1.1\r\n";
    const HEAD_SMALL: &[u8] = b"HEAD /site/1k.bin HTTP/1.1\r\n";
    const SENDFILE_64K: &[u8] = b"GET /site/64k.bin HTTP/1.1\r\n";
    const SENDFILE_1M: &[u8] = b"GET /site/1m.bin HTTP/1.1\r\n";
    const PROXY: &[u8] = b"GET /api/ HTTP/1.1\r\n";
    const INVALID: &[u8] = b"GET /assets/page.html HTTP/1.1\r\n";

    #[test]
    fn wire_blocking_off_by_default() {
        assert!(!wire_eligibility::static_wire_use_blocking_pool(
            SMALL_STATIC
        ));
        assert!(!wire_eligibility::static_wire_use_blocking_pool(HEAD_SMALL));
        assert!(!wire_eligibility::static_wire_use_blocking_pool(
            SENDFILE_64K
        ));
        assert!(!wire_eligibility::static_wire_use_blocking_pool(
            SENDFILE_1M
        ));
        assert!(!wire_eligibility::static_wire_use_blocking_pool(PROXY));
        assert!(!wire_eligibility::static_wire_use_blocking_pool(INVALID));
    }

    #[test]
    fn sendfile_blocking_off_by_default() {
        assert!(!wire_eligibility::static_sendfile_use_blocking_pool(
            SENDFILE_64K
        ));
        assert!(!wire_eligibility::static_sendfile_use_blocking_pool(
            SENDFILE_1M
        ));
        assert!(!wire_eligibility::static_sendfile_use_blocking_pool(
            SMALL_STATIC
        ));
    }

    #[test]
    fn one_m_blocking_off_by_default() {
        assert!(!wire_eligibility::static_one_m_sendfile_use_blocking(
            SENDFILE_1M
        ));
        assert!(!wire_eligibility::static_one_m_sendfile_use_blocking(
            SENDFILE_64K
        ));
    }

    #[test]
    fn combined_use_blocking_matches_or_of_wire_and_sendfile() {
        for head in [
            SMALL_STATIC,
            HEAD_SMALL,
            SENDFILE_64K,
            SENDFILE_1M,
            PROXY,
            INVALID,
        ] {
            let expected = wire_eligibility::static_wire_use_blocking_pool(head)
                || wire_eligibility::static_sendfile_use_blocking_pool(head);
            assert_eq!(
                expected,
                wire_eligibility::static_wire_use_blocking_pool(head)
                    || wire_eligibility::static_sendfile_use_blocking_pool(head),
                "head={}",
                String::from_utf8_lossy(head)
            );
        }
    }

    /// Sync bench path (`sync_accept`) uses `static_use_blocking_pool` = wire ∨ sendfile.
    fn sync_blocking_eligibility(head: &[u8]) -> bool {
        wire_eligibility::static_wire_use_blocking_pool(head)
            || wire_eligibility::static_sendfile_use_blocking_pool(head)
    }

    /// TCP path (`wire_dispatch::serve_static_tcp` when sendfile ineligible) adds 1M gate.
    fn tcp_blocking_admission(head: &[u8]) -> bool {
        sync_blocking_eligibility(head)
            || wire_eligibility::static_one_m_sendfile_use_blocking(head)
    }

    #[test]
    fn sync_vs_tcp_blocking_semantics_default_env() {
        for head in [
            SMALL_STATIC,
            HEAD_SMALL,
            SENDFILE_64K,
            SENDFILE_1M,
            PROXY,
            INVALID,
        ] {
            let sync = sync_blocking_eligibility(head);
            let tcp = tcp_blocking_admission(head);
            let one_m = wire_eligibility::static_one_m_sendfile_use_blocking(head);
            assert_eq!(tcp, sync || one_m, "head={}", String::from_utf8_lossy(head));
        }
    }

    #[test]
    fn sync_and_tcp_agree_when_one_m_gate_off() {
        for head in [SMALL_STATIC, HEAD_SMALL, SENDFILE_64K, SENDFILE_1M, PROXY] {
            if wire_eligibility::static_one_m_sendfile_use_blocking(head) {
                continue;
            }
            assert_eq!(
                sync_blocking_eligibility(head),
                tcp_blocking_admission(head),
                "head={}",
                String::from_utf8_lossy(head)
            );
        }
    }

    #[test]
    fn tcp_admission_can_differ_from_sync_only_via_one_m_gate() {
        for head in [SENDFILE_1M, SENDFILE_64K, SMALL_STATIC] {
            let sync = sync_blocking_eligibility(head);
            let tcp = tcp_blocking_admission(head);
            let one_m = wire_eligibility::static_one_m_sendfile_use_blocking(head);
            if tcp != sync {
                assert!(
                    one_m && !sync,
                    "divergence must be one_m-only: head={}",
                    String::from_utf8_lossy(head)
                );
            }
        }
    }

    #[test]
    fn epoll_sendfile_ineligible_by_default() {
        assert!(!wire_eligibility::epoll_sendfile_eligible(SENDFILE_64K));
        assert!(!wire_eligibility::epoll_sendfile_eligible(SENDFILE_1M));
    }

    #[test]
    fn might_use_static_wire_covers_bench_paths() {
        assert!(wire_eligibility::might_use_static_wire(SMALL_STATIC));
        assert!(wire_eligibility::might_use_static_wire(SENDFILE_64K));
        assert!(!wire_eligibility::might_use_static_wire(PROXY));
        assert!(!wire_eligibility::might_use_static_wire(INVALID));
    }
}
