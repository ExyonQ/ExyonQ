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

//! Hop-by-hop header policy aligned with `module-api::hop_by_hop` (dependency-free copy).

use crate::http_parse::Header;

/// Fixed hop-by-hop field names (lowercase ASCII) — must match module-api SSOT.
pub const FIXED_HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

#[inline]
#[allow(dead_code)] // policy API parity with module-api
pub fn is_fixed_hop_by_hop(name: &str) -> bool {
    FIXED_HOP_BY_HOP_HEADERS
        .iter()
        .any(|h| name.eq_ignore_ascii_case(h))
}

#[inline]
fn connection_option_skips_nomination(token: &str) -> bool {
    token.eq_ignore_ascii_case("close") || token.eq_ignore_ascii_case("keep-alive")
}

fn connection_nominating_tokens(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter(|t| !connection_option_skips_nomination(t))
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// Return header names that must not be forwarded (fixed set + Connection nominations).
pub fn hop_by_hop_blocklist(headers: &[Header]) -> Vec<String> {
    let mut block: Vec<String> = FIXED_HOP_BY_HOP_HEADERS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    for h in headers {
        if h.name.eq_ignore_ascii_case("connection") {
            for t in connection_nominating_tokens(&h.value) {
                block.push(t);
            }
        }
    }
    block
}

pub fn should_forward(name: &str, block: &[String]) -> bool {
    !block.iter().any(|b| name.eq_ignore_ascii_case(b.as_str()))
}

/// Build upstream request bytes with hop-by-hop stripped and Host rewritten.
pub fn build_upstream_request(
    method: &str,
    path: &str,
    headers: &[Header],
    authority_host: &str,
    content_length: u64,
) -> Vec<u8> {
    let block = hop_by_hop_blocklist(headers);
    let mut saw_content_length = false;
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(method.as_bytes());
    out.extend_from_slice(b" ");
    out.extend_from_slice(path.as_bytes());
    out.extend_from_slice(b" HTTP/1.1\r\n");
    out.extend_from_slice(b"Host: ");
    out.extend_from_slice(authority_host.as_bytes());
    out.extend_from_slice(b"\r\n");
    for h in headers {
        if h.name.eq_ignore_ascii_case("host") {
            continue;
        }
        if h.name.eq_ignore_ascii_case("content-length") {
            saw_content_length = true;
        }
        if !should_forward(&h.name, &block) {
            continue;
        }
        out.extend_from_slice(h.name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(h.value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if content_length > 0 && !saw_content_length {
        out.extend_from_slice(b"Content-Length: ");
        out.extend_from_slice(content_length.to_string().as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"Connection: keep-alive\r\n\r\n");
    out
}

/// Filter upstream response headers for client; return (status_line_kept, filtered headers text without final CRLF pair body).
pub fn filter_response_headers(raw_headers: &[Header]) -> (Vec<Header>, bool) {
    let block = hop_by_hop_blocklist(raw_headers);
    let mut out = Vec::new();
    let mut connection_close = false;
    for h in raw_headers {
        if h.name.eq_ignore_ascii_case("connection") {
            for tok in h.value.split(',') {
                if tok.trim().eq_ignore_ascii_case("close") {
                    connection_close = true;
                }
            }
        }
        if should_forward(&h.name, &block) {
            out.push(h.clone());
        }
    }
    (out, connection_close)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_tokens_removed() {
        let headers = vec![
            Header {
                name: "Connection".into(),
                value: "foo, bar, keep-alive".into(),
            },
            Header {
                name: "Foo".into(),
                value: "1".into(),
            },
            Header {
                name: "Bar".into(),
                value: "2".into(),
            },
            Header {
                name: "X-End".into(),
                value: "keep".into(),
            },
        ];
        let block = hop_by_hop_blocklist(&headers);
        assert!(!should_forward("Foo", &block));
        assert!(!should_forward("Bar", &block));
        assert!(should_forward("X-End", &block));
        assert!(!should_forward("Connection", &block));
    }
}
