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
//! Hop-by-hop filtering and request safety checks (policy only — no socket ownership).
//!
//! Fixed hop-by-hop semantics live in [`exyonq_module_api::hop_by_hop`]. This module
//! re-exports them for proxy callers and adds request/content-length helpers.

use http::header::{HeaderMap, HeaderValue, CONTENT_LENGTH, TRANSFER_ENCODING};

pub use exyonq_module_api::hop_by_hop::{
    connection_nominated_header_names, is_fixed_hop_by_hop_header as is_hop_by_hop_header,
    strip_hop_by_hop_headers, FIXED_HOP_BY_HOP_HEADERS,
};

/// Response header names stripped before forwarding to downstream clients.
pub const RESPONSE_HOP_BY_HOP_HEADERS: &[&str] = FIXED_HOP_BY_HOP_HEADERS;

/// Request header names stripped before forwarding to upstream.
pub const REQUEST_UPSTREAM_STRIP_HEADERS: &[&str] = FIXED_HOP_BY_HOP_HEADERS;

/// EXY-11: insert a header into an upstream `HeaderMap` preserving repeated
/// `Cookie` lines. `Cookie` merges into the existing value with "; " (RFC 6265
/// §5.4) because CGI-style backends read a single line; every other field keeps
/// the previous replace-insert (last-wins) semantics — appending arbitrary
/// fields could forward duplicate framing headers (`Content-Length`) on paths
/// without `request_headers_safe_for_proxy`. If the cookie concatenation is not
/// a valid `HeaderValue`, falls back to replace-insert.
pub fn insert_or_merge_cookie(
    map: &mut HeaderMap,
    name: http::header::HeaderName,
    value: HeaderValue,
) {
    if name.as_str().eq_ignore_ascii_case("cookie") {
        if let Some(existing) = map.get(&name) {
            let mut merged = existing.as_bytes().to_vec();
            merged.extend_from_slice(b"; ");
            merged.extend_from_slice(value.as_bytes());
            if let Ok(v) = HeaderValue::from_bytes(&merged) {
                map.insert(name, v);
                return;
            }
            map.insert(name, value);
            return;
        }
    }
    map.insert(name, value);
}

/// EXY-11: fold repeated `cookie` pairs into one "; "-joined pair, keeping the
/// first occurrence position and value order (RFC 6265 §5.4). Used by the
/// streaming forward path, which builds the request pair-by-pair.
pub fn merge_cookie_pairs(headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::with_capacity(headers.len());
    let mut cookie_at: Option<usize> = None;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("cookie") {
            match cookie_at {
                Some(idx) => {
                    out[idx].1.push_str("; ");
                    out[idx].1.push_str(value);
                }
                None => {
                    cookie_at = Some(out.len());
                    out.push((name.clone(), value.clone()));
                }
            }
            continue;
        }
        out.push((name.clone(), value.clone()));
    }
    out
}

/// EXY-11: fold repeated `Cookie` entries of a `HeaderMap` in place (RFC 6265
/// §5.4). Used by paths that forward the client request verbatim (WebSocket
/// upgrade, spike `forward_request_complete`) where cookies would otherwise
/// arrive as separate lines.
pub fn merge_cookie_header_map(map: &mut HeaderMap) {
    let mut it = map.get_all(http::header::COOKIE).iter();
    let Some(first) = it.next() else { return };
    let mut merged = first.as_bytes().to_vec();
    for v in it {
        merged.extend_from_slice(b"; ");
        merged.extend_from_slice(v.as_bytes());
    }
    if let Ok(v) = HeaderValue::from_bytes(&merged) {
        map.insert(http::header::COOKIE, v);
    }
}

/// Reject ambiguous length/framing headers before forwarding to upstream.
pub fn request_headers_safe_for_proxy(headers: &HeaderMap) -> bool {
    if headers.get_all(CONTENT_LENGTH).into_iter().nth(1).is_some() {
        return false;
    }
    if headers.contains_key(TRANSFER_ENCODING) && headers.contains_key(CONTENT_LENGTH) {
        return false;
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentLengthParse {
    Absent,
    Valid(usize),
    Invalid,
}

/// Parse single valid `Content-Length` for cache/materialization policy.
pub fn parse_response_content_length(headers: &HeaderMap) -> ContentLengthParse {
    let mut values = headers.get_all(CONTENT_LENGTH).into_iter();
    let Some(first) = values.next() else {
        return ContentLengthParse::Absent;
    };
    if values.next().is_some() {
        return ContentLengthParse::Invalid;
    }
    let Ok(raw) = first.to_str() else {
        return ContentLengthParse::Invalid;
    };
    let Ok(len) = raw.parse::<u64>() else {
        return ContentLengthParse::Invalid;
    };
    if len > usize::MAX as u64 {
        return ContentLengthParse::Invalid;
    }
    ContentLengthParse::Valid(len as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::{HeaderName, HeaderValue};

    #[test]
    fn cookie_insert_merges_repeated_with_semicolon() {
        let mut headers = HeaderMap::new();
        insert_or_merge_cookie(
            &mut headers,
            HeaderName::from_static("cookie"),
            HeaderValue::from_static("a=1"),
        );
        insert_or_merge_cookie(
            &mut headers,
            HeaderName::from_static("cookie"),
            HeaderValue::from_static("b=2"),
        );
        assert_eq!(
            headers.get("cookie").and_then(|v| v.to_str().ok()),
            Some("a=1; b=2")
        );
        assert_eq!(headers.get_all("cookie").iter().count(), 1);
    }

    #[test]
    fn insert_keeps_last_wins_for_non_cookie_fields() {
        // Only `Cookie` merges — other fields keep the previous replace-insert
        // semantics so duplicate framing headers (Content-Length) cannot leak
        // into upstreams on paths without the framing check (contract-audit
        // MF-2).
        let mut headers = HeaderMap::new();
        insert_or_merge_cookie(
            &mut headers,
            HeaderName::from_static("content-length"),
            HeaderValue::from_static("5"),
        );
        insert_or_merge_cookie(
            &mut headers,
            HeaderName::from_static("content-length"),
            HeaderValue::from_static("5"),
        );
        let all: Vec<_> = headers.get_all("content-length").iter().collect();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn merge_cookie_pairs_keeps_position_and_order() {
        let pairs = vec![
            ("Accept".to_string(), "text/html".to_string()),
            ("Cookie".to_string(), "a=1".to_string()),
            ("X-Other".to_string(), "x".to_string()),
            ("cookie".to_string(), "b=2".to_string()),
            ("Cookie".to_string(), "c=3".to_string()),
        ];
        let merged = merge_cookie_pairs(&pairs);
        assert_eq!(
            merged,
            vec![
                ("Accept".to_string(), "text/html".to_string()),
                ("Cookie".to_string(), "a=1; b=2; c=3".to_string()),
                ("X-Other".to_string(), "x".to_string()),
            ]
        );
    }

    #[test]
    fn merge_cookie_pairs_passthrough_when_single() {
        let pairs = vec![
            ("Cookie".to_string(), "a=1".to_string()),
            ("Accept".to_string(), "*/*".to_string()),
        ];
        assert_eq!(merge_cookie_pairs(&pairs), pairs);
    }

    #[test]
    fn detects_duplicate_content_length() {
        let mut headers = HeaderMap::new();
        headers.append(CONTENT_LENGTH, HeaderValue::from_static("1"));
        headers.append(CONTENT_LENGTH, HeaderValue::from_static("2"));
        assert!(!request_headers_safe_for_proxy(&headers));
    }

    #[test]
    fn detects_te_and_cl_together() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("1"));
        headers.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        assert!(!request_headers_safe_for_proxy(&headers));
    }

    #[test]
    fn hop_by_hop_detection() {
        assert!(is_hop_by_hop_header("Connection"));
        assert!(is_hop_by_hop_header("Upgrade"));
        assert!(!is_hop_by_hop_header("content-type"));
    }

    #[test]
    fn strip_removes_connection_nominated_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("connection"),
            HeaderValue::from_static("X-Secret, keep-alive"),
        );
        headers.insert(
            HeaderName::from_static("x-secret"),
            HeaderValue::from_static("leak"),
        );
        headers.insert(
            HeaderName::from_static("x-end-to-end"),
            HeaderValue::from_static("ok"),
        );
        strip_hop_by_hop_headers(&mut headers);
        assert!(!headers.contains_key("x-secret"));
        assert!(!headers.contains_key("connection"));
        assert!(headers.contains_key("x-end-to-end"));
    }

    #[test]
    fn strip_handles_ows_and_multiple_connection_values() {
        let mut headers = HeaderMap::new();
        headers.append(
            HeaderName::from_static("connection"),
            HeaderValue::from_static("  X-A  "),
        );
        headers.append(
            HeaderName::from_static("connection"),
            HeaderValue::from_static("X-B, close"),
        );
        headers.insert(
            HeaderName::from_static("x-a"),
            HeaderValue::from_static("1"),
        );
        headers.insert(
            HeaderName::from_static("x-b"),
            HeaderValue::from_static("2"),
        );
        headers.insert(
            HeaderName::from_static("x-keep"),
            HeaderValue::from_static("3"),
        );
        strip_hop_by_hop_headers(&mut headers);
        assert!(!headers.contains_key("x-a"));
        assert!(!headers.contains_key("x-b"));
        assert!(headers.contains_key("x-keep"));
    }

    #[test]
    fn strip_parses_opaque_non_utf8_connection_without_panic() {
        let mut headers = HeaderMap::new();
        let mut raw = b"X-Opaque".to_vec();
        raw.push(0xff);
        headers.insert(
            HeaderName::from_static("connection"),
            HeaderValue::from_bytes(&raw).expect("bytes"),
        );
        headers.insert(
            HeaderName::from_static("x-opaque"),
            HeaderValue::from_static("nope"),
        );
        strip_hop_by_hop_headers(&mut headers);
        // Token with 0xff is not a valid HeaderName → no nomination; Connection still stripped.
        assert!(!headers.contains_key("connection"));
        assert!(headers.contains_key("x-opaque"));
    }

    #[test]
    fn strip_nominates_ascii_custom_from_connection() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("connection"),
            HeaderValue::from_static("X-Opaque"),
        );
        headers.insert(
            HeaderName::from_static("x-opaque"),
            HeaderValue::from_static("nope"),
        );
        strip_hop_by_hop_headers(&mut headers);
        assert!(!headers.contains_key("x-opaque"));
    }

    #[test]
    fn strip_case_insensitive_nominated_name() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("connection"),
            HeaderValue::from_static("x-backend-private"),
        );
        headers.insert(
            HeaderName::from_static("x-backend-private"),
            HeaderValue::from_static("secret"),
        );
        strip_hop_by_hop_headers(&mut headers);
        assert!(!headers.contains_key("x-backend-private"));
        assert!(!headers.contains_key("connection"));
    }

    #[test]
    fn strip_removes_every_fixed_member_preserves_origin_auth() {
        let mut headers = HeaderMap::new();
        for &name in FIXED_HOP_BY_HOP_HEADERS {
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).expect("valid"),
                HeaderValue::from_static("x"),
            );
        }
        headers.insert(
            HeaderName::from_static("www-authenticate"),
            HeaderValue::from_static("Basic realm=\"o\""),
        );
        headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("AuthScheme keep"),
        );
        headers.insert(
            HeaderName::from_static("x-end-to-end"),
            HeaderValue::from_static("survive"),
        );
        strip_hop_by_hop_headers(&mut headers);
        for &name in FIXED_HOP_BY_HOP_HEADERS {
            assert!(
                !headers.contains_key(name),
                "missing strip for FIXED member {name}"
            );
            assert!(is_hop_by_hop_header(name));
        }
        assert!(headers.contains_key("www-authenticate"));
        assert!(headers.contains_key("authorization"));
        assert!(headers.contains_key("x-end-to-end"));
        assert!(!is_hop_by_hop_header("Authorization"));
        assert!(!is_hop_by_hop_header("WWW-Authenticate"));
    }

    #[test]
    fn strip_does_not_prevent_post_strip_xff_reinject() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("connection"),
            HeaderValue::from_static("x-forwarded-for"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("1.2.3.4"),
        );
        strip_hop_by_hop_headers(&mut headers);
        assert!(!headers.contains_key("x-forwarded-for"));
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("9.9.9.9"),
        );
        assert_eq!(
            headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()),
            Some("9.9.9.9")
        );
    }

    #[test]
    fn aliases_point_at_canonical_fixed_set() {
        assert!(std::ptr::eq(
            RESPONSE_HOP_BY_HOP_HEADERS.as_ptr(),
            FIXED_HOP_BY_HOP_HEADERS.as_ptr()
        ));
        assert!(std::ptr::eq(
            REQUEST_UPSTREAM_STRIP_HEADERS.as_ptr(),
            FIXED_HOP_BY_HOP_HEADERS.as_ptr()
        ));
    }
}
