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

use http::header::{HeaderMap, HeaderName, CONTENT_LENGTH, TRANSFER_ENCODING};

/// Response header names stripped before forwarding to downstream clients.
pub const RESPONSE_HOP_BY_HOP_HEADERS: &[&str] = &["connection", "transfer-encoding", "upgrade"];

/// Request header names stripped before forwarding to upstream (keep-alive warm path).
pub const REQUEST_UPSTREAM_STRIP_HEADERS: &[&str] = &["connection", "upgrade", "proxy-connection"];

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

/// True when a header must not be forwarded hop-by-hop to upstream or stored in cache.
pub fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
            | "proxy-connection"
    )
}

/// Remove hop-by-hop headers from a mutable header map (response or request).
pub fn strip_hop_by_hop_headers(headers: &mut HeaderMap) {
    headers.remove(HeaderName::from_static("connection"));
    headers.remove(HeaderName::from_static("transfer-encoding"));
    headers.remove(HeaderName::from_static("upgrade"));
    headers.remove(HeaderName::from_static("proxy-connection"));
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
    use http::header::{HeaderValue, TRANSFER_ENCODING};

    #[test]
    fn rejects_duplicate_content_length() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("1"));
        headers.append(CONTENT_LENGTH, HeaderValue::from_static("2"));
        assert!(!request_headers_safe_for_proxy(&headers));
    }

    #[test]
    fn rejects_te_and_cl_together() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("10"));
        headers.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        assert!(!request_headers_safe_for_proxy(&headers));
    }

    #[test]
    fn hop_by_hop_detects_upgrade() {
        assert!(is_hop_by_hop_header("Upgrade"));
        assert!(!is_hop_by_hop_header("content-type"));
    }
}
