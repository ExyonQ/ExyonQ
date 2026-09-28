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

    #[test]
    fn epoll_sendfile_eligible_auto_on_for_static_get() {
        // Cap067: default auto-on; kill-switch must disable.
        assert!(wire_eligibility::epoll_sendfile_eligible(SENDFILE_64K));
        assert!(wire_eligibility::epoll_sendfile_eligible(SENDFILE_1M));
        assert!(wire_eligibility::epoll_sendfile_eligible(SMALL_STATIC));
        assert!(!wire_eligibility::epoll_sendfile_eligible(
            b"GET /health HTTP/1.1\r\n"
        ));
    }

    #[test]
    fn epoll_sendfile_eligible_treats_http11_absolute_form_as_origin_path() {
        // RFC 9112 absolute-form is a valid HTTP/1.1 request-target (rewrk, proxies).
        // Same product path as origin-form — not a filename or loadgen special case.
        const ABSOLUTE_SITE: &[u8] = b"GET http://exyonq:8080/site/1k.bin HTTP/1.1\r\n";
        const ABSOLUTE_SITE_HTTPS: &[u8] = b"GET https://host/site/1k.bin HTTP/1.1\r\n";
        const ABSOLUTE_HEAD: &[u8] = b"HEAD http://exyonq:8080/site/1k.bin HTTP/1.1\r\n";
        const ABSOLUTE_QUERY: &[u8] = b"GET http://exyonq:8080/site/1k.bin?x=1 HTTP/1.1\r\n";
        const ABSOLUTE_API: &[u8] = b"GET http://exyonq:8080/api/foo HTTP/1.1\r\n";
        const ABSOLUTE_HEALTH: &[u8] = b"GET http://exyonq:8080/health HTTP/1.1\r\n";
        const ABSOLUTE_METRICS: &[u8] = b"GET http://exyonq:8080/metrics HTTP/1.1\r\n";

        assert_eq!(
            wire_eligibility::epoll_sendfile_eligible(ABSOLUTE_SITE),
            wire_eligibility::epoll_sendfile_eligible(SMALL_STATIC)
        );
        assert!(wire_eligibility::epoll_sendfile_eligible(ABSOLUTE_SITE));
        assert!(wire_eligibility::epoll_sendfile_eligible(
            ABSOLUTE_SITE_HTTPS
        ));
        assert!(wire_eligibility::epoll_sendfile_eligible(ABSOLUTE_HEAD));
        assert!(wire_eligibility::epoll_sendfile_eligible(ABSOLUTE_QUERY));
        assert!(!wire_eligibility::epoll_sendfile_eligible(ABSOLUTE_API));
        assert!(!wire_eligibility::epoll_sendfile_eligible(ABSOLUTE_HEALTH));
        assert!(!wire_eligibility::epoll_sendfile_eligible(ABSOLUTE_METRICS));
        assert_eq!(
            wire_eligibility::request_path_from_head(ABSOLUTE_SITE),
            Some("/site/1k.bin")
        );
        assert_eq!(
            wire_eligibility::request_path_from_head(ABSOLUTE_QUERY),
            Some("/site/1k.bin")
        );
        assert_eq!(
            wire_eligibility::request_path_from_head(SMALL_STATIC),
            Some("/site/1k.bin")
        );
    }

    #[test]
    fn might_use_static_wire_includes_sendfile_candidates() {
        assert!(wire_eligibility::might_use_static_wire(SMALL_STATIC));
        assert!(wire_eligibility::might_use_static_wire(SENDFILE_64K));
        assert!(!wire_eligibility::might_use_static_wire(PROXY));
        assert!(wire_eligibility::might_use_static_wire(INVALID));
        assert!(wire_eligibility::might_use_static_wire(
            b"GET /health HTTP/1.1\r\n"
        ));
        // `/metrics` is Hyper-only (LA-CAP054-008); must not StayAttached on Cap067.
        assert!(!wire_eligibility::might_use_static_wire(
            b"GET /metrics HTTP/1.1\r\n"
        ));
    }
}
