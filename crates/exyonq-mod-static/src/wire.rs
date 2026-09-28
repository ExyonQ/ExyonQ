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
//! Pre-serialized HTTP/1.1 responses for the raw static hot path.

use bytes::Bytes;
use std::fmt::Write as _;
use std::sync::Arc;

/// HTTP status + headers only (body sent separately, e.g. Linux sendfile).
#[derive(Clone)]
pub struct HeaderPair {
    pub keep_alive: Arc<Bytes>,
    pub close: Arc<Bytes>,
    pub body_len: usize,
}

impl HeaderPair {
    pub fn for_body_len(body_len: usize, content_type: &'static str) -> Self {
        Self {
            keep_alive: Arc::new(build_header(body_len, content_type, false)),
            close: Arc::new(build_header(body_len, content_type, true)),
            body_len,
        }
    }

    pub fn select_header(&self, client_close: bool) -> Arc<Bytes> {
        if client_close {
            Arc::clone(&self.close)
        } else {
            Arc::clone(&self.keep_alive)
        }
    }
}

#[derive(Clone)]
pub struct WirePair {
    pub keep_alive: Arc<Bytes>,
    pub close: Arc<Bytes>,
}

impl WirePair {
    pub fn for_body(body: &[u8], content_type: &'static str) -> Self {
        Self {
            keep_alive: Arc::new(build_wire(body, content_type, false)),
            close: Arc::new(build_wire(body, content_type, true)),
        }
    }

    pub fn select(&self, client_close: bool) -> Arc<Bytes> {
        if client_close {
            Arc::clone(&self.close)
        } else {
            Arc::clone(&self.keep_alive)
        }
    }
}

/// Keep-alive response headers (no body) for a given Content-Length.
pub fn header_keep(body_len: usize, content_type: &'static str) -> Arc<Bytes> {
    Arc::new(build_header(body_len, content_type, false))
}

/// Cap067/Cap020: 200 OK headers with validators (ETag / Last-Modified) + Accept-Ranges.
pub fn ok_header_with_validators(
    body_len: usize,
    content_type: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Bytes {
    let mut s = format!(
        "HTTP/1.1 200 OK\r\n\
Accept-Ranges: bytes\r\n\
Content-Type: {content_type}\r\n\
Content-Length: {body_len}\r\n\
Connection: keep-alive\r\n"
    );
    if let Some(etag) = etag {
        let _ = write!(s, "ETag: {etag}\r\n");
    }
    if let Some(lm) = last_modified {
        let _ = write!(s, "Last-Modified: {lm}\r\n");
    }
    s.push_str("\r\n");
    Bytes::from(s)
}

/// Cap067/ADR-046: 200 OK for a precompressed static object (Content-Encoding + Vary).
///
/// `body_len` is the **coded** length. ETag should be source identity + coding suffix
/// (see [`crate::encoding_cache::encoded_etag`]).
pub fn ok_header_encoded_with_validators(
    body_len: usize,
    content_type: &str,
    content_encoding: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Bytes {
    let mut s = format!(
        "HTTP/1.1 200 OK\r\n\
Accept-Ranges: bytes\r\n\
Content-Type: {content_type}\r\n\
Content-Encoding: {content_encoding}\r\n\
Vary: Accept-Encoding\r\n\
Content-Length: {body_len}\r\n\
Connection: keep-alive\r\n"
    );
    if let Some(etag) = etag {
        let _ = write!(s, "ETag: {etag}\r\n");
    }
    if let Some(lm) = last_modified {
        let _ = write!(s, "Last-Modified: {lm}\r\n");
    }
    s.push_str("\r\n");
    Bytes::from(s)
}

pub fn health_wire(client_close: bool) -> Bytes {
    build_wire(b"ok", "text/plain; charset=utf-8", client_close)
}

pub fn not_found_wire(client_close: bool) -> Bytes {
    // LA-CAP067-WIRE-001: real HTTP 404 on the wire (never 200 + "not found").
    not_found_status_wire(client_close)
}

/// KD2: exported for core static_conn/epoll until KD2.3 moves wire glue out of core.
#[doc(hidden)]
pub static HEALTH_KEEP_ALIVE: std::sync::LazyLock<Bytes> =
    std::sync::LazyLock::new(|| build_wire(b"ok", "text/plain; charset=utf-8", false));

/// HEAD /health: same Content-Length as GET, no body.
#[doc(hidden)]
pub static HEALTH_HEAD_KEEP_ALIVE: std::sync::LazyLock<Bytes> =
    std::sync::LazyLock::new(|| build_header(2, "text/plain; charset=utf-8", false));

/// Select GET or HEAD health wire bytes.
#[inline]
pub fn health_keep_alive_for_head(head: &[u8]) -> &'static [u8] {
    if head.starts_with(b"HEAD ") {
        HEALTH_HEAD_KEEP_ALIVE.as_ref()
    } else {
        HEALTH_KEEP_ALIVE.as_ref()
    }
}

/// KD2: exported for core static_conn/epoll until KD2.3.
/// LA-CAP067-WIRE-001: HTTP/1.1 404 Not Found (keep-alive), not a fake 200.
#[doc(hidden)]
pub static NOT_FOUND_KEEP_ALIVE: std::sync::LazyLock<Bytes> =
    std::sync::LazyLock::new(|| not_found_status_wire(false));

/// HEAD variant of [`NOT_FOUND_KEEP_ALIVE`] (Content-Length 9, zero body).
#[doc(hidden)]
pub static NOT_FOUND_HEAD_KEEP_ALIVE: std::sync::LazyLock<Bytes> =
    std::sync::LazyLock::new(|| not_found_status_head_wire(false));

/// Select GET or HEAD not-found wire bytes (real 404 status line).
#[inline]
pub fn not_found_keep_alive_for_head(head: &[u8]) -> &'static [u8] {
    if head.starts_with(b"HEAD ") {
        NOT_FOUND_HEAD_KEEP_ALIVE.as_ref()
    } else {
        NOT_FOUND_KEEP_ALIVE.as_ref()
    }
}

/// Split a precooked HTTP/1 wire buffer into header block (incl. CRLFCRLF) and body.
pub fn split_wire_header_body(wire: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut i = 0;
    while i + 3 < wire.len() {
        if &wire[i..i + 4] == b"\r\n\r\n" {
            return Some((&wire[..i + 4], &wire[i + 4..]));
        }
        i += 1;
    }
    None
}

/// Cap019: 206 Partial Content response headers (wire/sendfile).
pub fn partial_content_header(start: u64, end: u64, full_length: u64, content_type: &str) -> Bytes {
    partial_content_header_with_validators(start, end, full_length, content_type, None, None)
}

/// Cap019 + Cap020 validators on 206.
pub fn partial_content_header_with_validators(
    start: u64,
    end: u64,
    full_length: u64,
    content_type: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Bytes {
    let content_length = end - start + 1;
    let mut s = format!(
        "HTTP/1.1 206 Partial Content\r\n\
Accept-Ranges: bytes\r\n\
Content-Range: bytes {start}-{end}/{full_length}\r\n\
Content-Length: {content_length}\r\n\
Content-Type: {content_type}\r\n"
    );
    if let Some(etag) = etag {
        let _ = write!(s, "ETag: {etag}\r\n");
    }
    if let Some(lm) = last_modified {
        let _ = write!(s, "Last-Modified: {lm}\r\n");
    }
    s.push_str("\r\n");
    Bytes::from(s)
}

/// Cap019: 416 Range Not Satisfiable response headers.
pub fn range_not_satisfiable_header(full_length: u64) -> Bytes {
    Bytes::from(format!(
        "HTTP/1.1 416 Range Not Satisfiable\r\n\
Accept-Ranges: bytes\r\n\
Content-Range: bytes */{full_length}\r\n\
Content-Length: 0\r\n\
\r\n"
    ))
}

/// Cap020: 304 Not Modified (wire/sendfile).
pub fn not_modified_header(etag: &str, last_modified: Option<&str>) -> Bytes {
    let mut s = format!("HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\n");
    if let Some(lm) = last_modified {
        let _ = write!(s, "Last-Modified: {lm}\r\n");
    }
    s.push_str("\r\n");
    Bytes::from(s)
}

fn build_header(body_len: usize, content_type: &str, client_close: bool) -> Bytes {
    let mut header = String::with_capacity(128);
    let _ = write!(
        header,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {body_len}\r\nConnection: "
    );
    if client_close {
        header.push_str("close\r\n\r\n");
    } else {
        header.push_str("keep-alive\r\n\r\n");
    }
    Bytes::from(header)
}

fn build_wire(body: &[u8], content_type: &str, client_close: bool) -> Bytes {
    let header = build_header(body.len(), content_type, client_close);
    let mut wire = Vec::with_capacity(header.len() + body.len());
    wire.extend_from_slice(&header);
    wire.extend_from_slice(body);
    Bytes::from(wire)
}

pub fn not_acceptable_status_wire(client_close: bool) -> Bytes {
    let conn = if client_close { "close" } else { "keep-alive" };
    Bytes::from(format!(
        "HTTP/1.1 406 Not Acceptable\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 15\r\nConnection: {conn}\r\n\r\nNot Acceptable"
    ))
}

pub fn not_acceptable_status_head_wire(client_close: bool) -> Bytes {
    let conn = if client_close { "close" } else { "keep-alive" };
    Bytes::from(format!(
        "HTTP/1.1 406 Not Acceptable\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 0\r\nConnection: {conn}\r\n\r\n"
    ))
}

pub fn not_found_status_wire(client_close: bool) -> Bytes {
    let conn = if client_close { "close" } else { "keep-alive" };
    Bytes::from(format!(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 9\r\nConnection: {conn}\r\n\r\nnot found"
    ))
}

/// HEAD variant: same Content-Length as GET miss, zero body bytes on the wire.
pub fn not_found_status_head_wire(client_close: bool) -> Bytes {
    let conn = if client_close { "close" } else { "keep-alive" };
    Bytes::from(format!(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 9\r\nConnection: {conn}\r\n\r\n"
    ))
}

pub fn forbidden_status_wire(client_close: bool) -> Bytes {
    let conn = if client_close { "close" } else { "keep-alive" };
    Bytes::from(format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 9\r\nConnection: {conn}\r\n\r\nforbidden"
    ))
}

/// HEAD variant: same Content-Length as GET forbid, zero body bytes on the wire.
pub fn forbidden_status_head_wire(client_close: bool) -> Bytes {
    let conn = if client_close { "close" } else { "keep-alive" };
    Bytes::from(format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 9\r\nConnection: {conn}\r\n\r\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_contains_body() {
        let body = b"xy";
        let wire = build_wire(body, "application/octet-stream", false);
        assert!(wire.starts_with(b"HTTP/1.1 200 OK"));
        assert!(wire.ends_with(body));
    }

    #[test]
    fn health_head_omits_body() {
        let get = health_keep_alive_for_head(b"GET /health HTTP/1.1\r\n");
        let head = health_keep_alive_for_head(b"HEAD /health HTTP/1.1\r\n");
        assert!(get.ends_with(b"ok"));
        assert!(!head.ends_with(b"ok"));
        assert!(head.windows(4).any(|w| w == b"\r\n\r\n"));
        let (_, body) = split_wire_header_body(head).expect("headers");
        assert!(body.is_empty());
        assert!(head.windows(18).any(|w| w == b"Content-Length: 2\r"));
    }
}
