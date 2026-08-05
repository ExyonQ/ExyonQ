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

    pub fn for_bench_body(body_len: usize) -> Self {
        Self {
            keep_alive: bench_header_keep(body_len),
            close: Arc::new(build_bench_header(body_len, true)),
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

    pub fn for_bench_body(body: &[u8]) -> Self {
        Self {
            keep_alive: Arc::new(build_bench_wire(body, false)),
            close: Arc::new(build_bench_wire(body, true)),
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

/// Bench static loop uses HTTP/1.1 default keep-alive only (no close variant in memory).
pub fn bench_header_keep(body_len: usize) -> Arc<Bytes> {
    Arc::new(build_bench_header(body_len, false))
}

pub fn bench_wire_keep(body: &[u8]) -> Arc<Bytes> {
    Arc::new(build_bench_wire(body, false))
}

pub fn health_wire(client_close: bool) -> Bytes {
    build_wire(b"ok", "text/plain; charset=utf-8", client_close)
}

pub fn not_found_wire(client_close: bool) -> Bytes {
    build_wire(b"not found", "text/plain; charset=utf-8", client_close)
}

const BENCH_HEADER_512: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 512\r\n\r\n";
const BENCH_HEADER_1024: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\n\r\n";
const BENCH_HEADER_64K: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 65536\r\n\r\n";
const BENCH_HEADER_1M: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\n";

/// P1 bench wire embedded at build time (header + 1024× `x`), lives in `.rodata`.
#[cfg(target_os = "linux")]
pub static P1_BENCH_WIRE_RODATA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/p1_bench_wire.bin"));

#[cfg(target_os = "linux")]
pub fn p1_bench_wire_rodata() -> &'static [u8] {
    P1_BENCH_WIRE_RODATA
}

#[cfg(not(target_os = "linux"))]
pub fn p1_bench_wire_rodata() -> &'static [u8] {
    &[]
}

#[cfg(target_os = "linux")]
pub fn rodata_p1_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("EXYONQ_RODATA_P1").ok().as_deref() != Some("0"))
}

#[cfg(not(target_os = "linux"))]
pub fn rodata_p1_enabled() -> bool {
    false
}

/// KD2: exported for core static_conn/epoll until KD2.3 moves wire glue out of core.
#[doc(hidden)]
pub static HEALTH_KEEP_ALIVE: std::sync::LazyLock<Bytes> =
    std::sync::LazyLock::new(|| build_wire(b"ok", "text/plain; charset=utf-8", false));

/// KD2: exported for core static_conn/epoll until KD2.3.
#[doc(hidden)]
pub static NOT_FOUND_KEEP_ALIVE: std::sync::LazyLock<Bytes> =
    std::sync::LazyLock::new(|| build_wire(b"not found", "text/plain; charset=utf-8", false));

fn build_bench_header(body_len: usize, client_close: bool) -> Bytes {
    if client_close {
        return Bytes::from(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n"
        ));
    }
    match body_len {
        512 => Bytes::from_static(BENCH_HEADER_512),
        1024 => Bytes::from_static(BENCH_HEADER_1024),
        65536 => Bytes::from_static(BENCH_HEADER_64K),
        1048576 => Bytes::from_static(BENCH_HEADER_1M),
        other => Bytes::from(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {other}\r\n\r\n"
        )),
    }
}

fn build_bench_wire(body: &[u8], client_close: bool) -> Bytes {
    let header = build_bench_header(body.len(), client_close);
    let mut wire = Vec::with_capacity(header.len() + body.len());
    wire.extend_from_slice(&header);
    wire.extend_from_slice(body);
    Bytes::from(wire)
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

pub fn not_found_status_wire(client_close: bool) -> Bytes {
    let conn = if client_close { "close" } else { "keep-alive" };
    Bytes::from(format!(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 9\r\nConnection: {conn}\r\n\r\nnot found"
    ))
}

pub fn forbidden_status_wire(client_close: bool) -> Bytes {
    let conn = if client_close { "close" } else { "keep-alive" };
    Bytes::from(format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 9\r\nConnection: {conn}\r\n\r\nforbidden"
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
}
