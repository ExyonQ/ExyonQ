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
//! Canonical hop-by-hop header policy shared by proxy, Cap034, FastCGI, and cache.
//!
//! # Semantic domains
//!
//! - **Fixed set** ([`FIXED_HOP_BY_HOP_HEADERS`]): names that never cross a hop.
//! - **Dynamic Connection tokens**: option tokens that nominate additional hop fields
//!   (RFC 7230 §6.1); `close` / `keep-alive` do not nominate.
//! - **End-to-end controls** (not listed): `Authorization`, `WWW-Authenticate`.
//!
//! Protocol adapters (WebSocket Upgrade restore, CGI UTF-8 Connection lines vs raw
//! `HeaderMap` octets) must call these primitives — they must not redefine the fixed set.

use http::header::{HeaderMap, HeaderName};

/// Fixed hop-by-hop field names (lowercase ASCII).
///
/// Includes both `Trailer` (header field) and `trailers` (historical Connection/TE
/// token spelling). Does **not** include end-to-end `Authorization` or `WWW-Authenticate`.
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

/// True when `name` is in the canonical fixed hop-by-hop set (ASCII case-insensitive).
#[inline]
pub fn is_fixed_hop_by_hop_header(name: &str) -> bool {
    FIXED_HOP_BY_HOP_HEADERS
        .iter()
        .any(|h| name.eq_ignore_ascii_case(h))
}

/// Connection options that are hop directives, not header-field nominations.
#[inline]
pub fn connection_option_skips_nomination(token: &[u8]) -> bool {
    token.eq_ignore_ascii_case(b"close") || token.eq_ignore_ascii_case(b"keep-alive")
}

/// Split a raw `Connection` field value into option tokens (OWS around commas).
pub fn split_connection_option_tokens(raw: &[u8]) -> impl Iterator<Item = &[u8]> {
    raw.split(|&b| b == b',')
        .map(|part| {
            let mut start = 0;
            let mut end = part.len();
            while start < end && (part[start] == b' ' || part[start] == b'\t') {
                start += 1;
            }
            while end > start && (part[end - 1] == b' ' || part[end - 1] == b'\t') {
                end -= 1;
            }
            &part[start..end]
        })
        .filter(|t| !t.is_empty())
}

/// Nominating Connection option tokens from opaque octets (HTTP `HeaderValue` path).
pub fn connection_nominating_tokens_from_bytes(raw: &[u8]) -> Vec<&[u8]> {
    split_connection_option_tokens(raw)
        .filter(|t| !connection_option_skips_nomination(t))
        .collect()
}

/// Nominating Connection option tokens from an already-decoded ASCII/UTF-8 value
/// (FastCGI CGI / string-pair adapters).
pub fn connection_nominating_tokens_from_str(value: &str) -> Vec<&str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter(|t| !connection_option_skips_nomination(t.as_bytes()))
        .collect()
}

/// Collect header names nominated by `Connection` option tokens (RFC 7230 §6.1).
///
/// Parses raw octets (not UTF-8 `to_str`) so opaque Connection values still nominate.
pub fn connection_nominated_header_names(headers: &HeaderMap) -> Vec<HeaderName> {
    let mut nominated: Vec<HeaderName> = Vec::new();
    for value in headers.get_all(http::header::CONNECTION) {
        for token in connection_nominating_tokens_from_bytes(value.as_bytes()) {
            if let Ok(name) = HeaderName::from_bytes(token) {
                nominated.push(name);
            }
        }
    }
    nominated
}

/// Remove hop-by-hop headers: Connection-nominated names, then the fixed set.
///
/// Removals for the fixed set are derived **only** from [`FIXED_HOP_BY_HOP_HEADERS`].
/// Callers that inject intermediary identity headers (`Host`, `X-Forwarded-*`) must
/// do so **after** this function so Connection cannot wipe them.
pub fn strip_hop_by_hop_headers(headers: &mut HeaderMap) {
    for name in connection_nominated_header_names(headers) {
        headers.remove(name);
    }
    for &name in FIXED_HOP_BY_HOP_HEADERS {
        if let Ok(hn) = HeaderName::from_bytes(name.as_bytes()) {
            headers.remove(hn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    fn fixed_set_predicate_and_strip_are_single_source() {
        let mut headers = HeaderMap::new();
        for &name in FIXED_HOP_BY_HOP_HEADERS {
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).expect("valid"),
                HeaderValue::from_static("x"),
            );
        }
        headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("AuthScheme keep"),
        );
        headers.insert(
            HeaderName::from_static("www-authenticate"),
            HeaderValue::from_static("Basic realm=\"o\""),
        );
        headers.insert(
            HeaderName::from_static("x-end-to-end"),
            HeaderValue::from_static("survive"),
        );
        strip_hop_by_hop_headers(&mut headers);
        for &name in FIXED_HOP_BY_HOP_HEADERS {
            assert!(
                !headers.contains_key(name),
                "strip must remove every FIXED member: {name}"
            );
            assert!(is_fixed_hop_by_hop_header(name));
        }
        assert!(headers.contains_key("authorization"));
        assert!(headers.contains_key("www-authenticate"));
        assert!(headers.contains_key("x-end-to-end"));
        assert!(!is_fixed_hop_by_hop_header("Authorization"));
        assert!(!is_fixed_hop_by_hop_header("WWW-Authenticate"));
    }

    #[test]
    fn connection_tokens_skip_close_and_keep_alive() {
        let tokens = connection_nominating_tokens_from_bytes(b" close , X-Secret , keep-alive ");
        assert_eq!(tokens, vec![b"X-Secret".as_slice()]);
        let s = connection_nominating_tokens_from_str("  X-A , close , X-B ");
        assert_eq!(s, vec!["X-A", "X-B"]);
    }
}
