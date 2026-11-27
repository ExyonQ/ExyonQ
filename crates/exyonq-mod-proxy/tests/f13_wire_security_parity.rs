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
//! F-13 security parity for the published wire fast path.
//!
//! `might_use_proxy_wire` is the admission decision between the raw wire relay
//! and Hyper. It must accept only requests whose framing and authority are
//! unambiguous, so every rejected case is parsed (and, where invalid, rejected)
//! by Hyper rather than frozen by the wire path.

use exyonq_mod_proxy::wire_conn::might_use_proxy_wire;

fn get(host: &str, extra: &str) -> Vec<u8> {
    format!("GET /api/health HTTP/1.1\r\nHost: {host}\r\n{extra}\r\n").into_bytes()
}

fn raw(request: &str) -> Vec<u8> {
    request.as_bytes().to_vec()
}

#[test]
fn accepts_bodyless_get_with_forwarded_security_headers() {
    let head = get(
        "example.test:8443",
        &format!(
            "Authorization: Bearer {}\r\nCookie: session=abc\r\nRange: bytes=0-15\r\n\
             X-Forwarded-For: 203.0.113.7\r\nConnection: close\r\n",
            "token"
        ),
    );
    assert!(
        might_use_proxy_wire(&head),
        "bodyless GET with end-to-end security headers is wire-eligible"
    );
    assert!(might_use_proxy_wire(&get(
        "localhost",
        "Content-Length: 0\r\n"
    )));
    assert!(might_use_proxy_wire(&get("[::1]:8443", "")));
}

#[test]
fn rejects_requests_whose_framing_or_authority_is_ambiguous() {
    let cases: &[(&str, Vec<u8>)] = &[
        (
            "non-GET",
            raw("POST /api/health HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        ),
        (
            "absolute-form target",
            raw("GET http://example.test/api/health HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        ),
        (
            "non-zero content-length",
            get("localhost", "Content-Length: 1\r\n"),
        ),
        (
            "duplicate content-length",
            get("localhost", "Content-Length: 0\r\nContent-Length: 0\r\n"),
        ),
        (
            "transfer-encoding alone",
            get("localhost", "Transfer-Encoding: chunked\r\n"),
        ),
        (
            "transfer-encoding plus content-length",
            get(
                "localhost",
                "Transfer-Encoding: chunked\r\nContent-Length: 0\r\n",
            ),
        ),
        (
            "expect continue",
            get("localhost", "Expect: 100-continue\r\n"),
        ),
        (
            "websocket upgrade",
            get("localhost", "Connection: Upgrade\r\nUpgrade: websocket\r\n"),
        ),
        (
            "h2c upgrade",
            get(
                "localhost",
                "Connection: keep-alive, Upgrade\r\nUpgrade: h2c\r\n",
            ),
        ),
        (
            "duplicate host",
            raw("GET /api/health HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n"),
        ),
        ("missing host", raw("GET /api/health HTTP/1.1\r\n\r\n")),
        (
            "empty host",
            raw("GET /api/health HTTP/1.1\r\nHost:   \r\n\r\n"),
        ),
        (
            "comma host",
            raw("GET /api/health HTTP/1.1\r\nHost: a,b\r\n\r\n"),
        ),
        (
            "space host",
            raw("GET /api/health HTTP/1.1\r\nHost: a b\r\n\r\n"),
        ),
        (
            "obsolete line folding",
            get("localhost", "X-Test: a\r\n b\r\n"),
        ),
    ];

    for (name, head) in cases {
        assert!(
            !might_use_proxy_wire(head),
            "{name} must fall back to Hyper, not the wire relay"
        );
    }
}

#[test]
fn header_names_are_matched_case_insensitively() {
    let head =
        raw("GET /api/health HTTP/1.1\r\nhOsT: localhost\r\ntRaNsFeR-EnCoDiNg: chunked\r\n\r\n");
    assert!(!might_use_proxy_wire(&head));
}
