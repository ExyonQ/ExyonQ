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
//! Raw HTTP/1.1 loop for cached `/api/*` proxy paths (P4/P8 bench hot path) — KD3.4 module-owned.
//!
//! Downstream framing: known-length upstream responses use `Content-Length` + keep-alive
//! (reuse the TCP admission). Unknown length / TE keeps Cap034 chunked + close. Host must
//! stay stable across keep-alive re-entry (LA-001 fail-closed on Host switch).
//!
//! # Wiring invariant (VALID_STRUCTURAL_INVARIANT)
//!
//! Every supported product entry into this module's serving path occurs only after
//! [`pin_runtime`] / [`crate::install_kernel_hooks`] has completed:
//! - CLI: `exyonq_mod_proxy::registration_with_default_runtime()` pins before register;
//! - listen workers / tests: `install_kernel_hooks` before accept/serve.
//!
//! [`try_shutdown_health`] is the only cold path that tolerates a missing pin (no-op).
//! `runtime()` therefore treats an absent pin as a programming error (fail-stop), not an
//! operator-recoverable condition.

use crate::runtime::ProxyRuntime;
use crate::wire_io;
use bytes::Bytes;
use hyper::header::HeaderValue;
use std::io::{self, Result};
use std::sync::{Arc, OnceLock, RwLock};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

#[cfg(target_os = "linux")]
use std::os::fd::RawFd;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

/// Larger batches reduce flush/syscall overhead on long SSE streams (P8).
/// Compiled only when the progressive / p8t-fix-baseline paths are reachable
/// (or under `cfg(test)`). With `p8-transport-diag` the production call site
/// uses `write_finite_te_single_buffer` instead — keep helpers out of that build.
#[cfg(any(test, not(feature = "p8-transport-diag")))]
const SSE_FLUSH_CHUNK: usize = 16384;
#[cfg(all(feature = "p8t-fix-baseline", not(feature = "p8-transport-diag")))]
const SSE_FLUSH_INTERVAL: usize = 2;

static RUNTIME: OnceLock<RwLock<Arc<ProxyRuntime>>> = OnceLock::new();

pub fn pin_runtime(runtime: Arc<ProxyRuntime>) {
    match RUNTIME.get() {
        Some(lock) => {
            *lock.write().expect("proxy runtime pin poisoned") = runtime;
        }
        None => {
            let _ = RUNTIME.set(RwLock::new(runtime));
        }
    }
}

/// Cap041: abort Cap024 health probe tasks when present (no-op if hooks not installed).
pub fn try_shutdown_health() {
    let Some(lock) = RUNTIME.get() else {
        return;
    };
    let Ok(guard) = lock.read() else {
        return;
    };
    guard.shutdown_health();
}

/// Process-pinned [`ProxyRuntime`].
///
/// # Panics
///
/// Panics if called before [`pin_runtime`] / `install_kernel_hooks`. Supported product
/// entry points always pin first (see module docs). Poisoned `RwLock` also fail-stops —
/// recovering poisoned cluster state would be unsound.
fn runtime() -> Arc<ProxyRuntime> {
    RUNTIME
        .get()
        .expect("ProxyRuntime not pinned — call install_kernel_hooks from CLI")
        .read()
        .expect("proxy runtime pin poisoned")
        .clone()
}

/// Pinned module runtime (composition root / cache load by cluster_id).
pub fn pinned_runtime() -> Arc<ProxyRuntime> {
    runtime()
}

/// Count header-name occurrences case-insensitively (`needle` is ASCII lowercase
/// including the trailing colon, e.g. `b"content-length:"`). Scans line starts only
/// (no whole-head lowercase allocation).
fn header_field_count_ci(head: &[u8], needle_lower: &[u8]) -> usize {
    let mut count = 0usize;
    for line in head.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(prefix) = line.get(..needle_lower.len()) {
            if prefix.eq_ignore_ascii_case(needle_lower) {
                count += 1;
            }
        }
    }
    count
}

fn header_name_present_ci(head: &[u8], needle_lower: &[u8]) -> bool {
    header_field_count_ci(head, needle_lower) > 0
}

/// Raw-head WebSocket upgrade detection — wire proxy cannot tunnel upgrades.
/// Cap031 LA-002: Connection is a comma-separated token list (e.g. `close, Upgrade`).
fn wire_head_is_websocket_upgrade(head: &[u8]) -> bool {
    let mut upgrade_websocket = false;
    let mut connection_upgrade = false;
    for line in head.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.len() >= 8 && line[..8].eq_ignore_ascii_case(b"upgrade:") {
            let mut v = &line[8..];
            while v.first().copied() == Some(b' ') || v.first().copied() == Some(b'\t') {
                v = &v[1..];
            }
            if v.eq_ignore_ascii_case(b"websocket") {
                upgrade_websocket = true;
            }
        } else if line.len() >= 11 && line[..11].eq_ignore_ascii_case(b"connection:") {
            let mut v = &line[11..];
            while v.first().copied() == Some(b' ') || v.first().copied() == Some(b'\t') {
                v = &v[1..];
            }
            for token in v.split(|&b| b == b',') {
                let mut t = token;
                while t.first().copied() == Some(b' ') || t.first().copied() == Some(b'\t') {
                    t = &t[1..];
                }
                while t.last().copied() == Some(b' ') || t.last().copied() == Some(b'\t') {
                    t = &t[..t.len() - 1];
                }
                if t.eq_ignore_ascii_case(b"upgrade") {
                    connection_upgrade = true;
                }
            }
        }
    }
    upgrade_websocket && connection_upgrade
}

pub fn might_use_proxy_wire(head: &[u8]) -> bool {
    if !(head.starts_with(b"GET /api/") || head.starts_with(b"GET /api ")) {
        return false;
    }
    if wire_head_is_websocket_upgrade(head) {
        return false;
    }
    // LA-PROXY-TE-CL-CASE-001: header names are case-insensitive (RFC 9110).
    if header_field_count_ci(head, b"content-length:") > 1 {
        return false;
    }
    if header_name_present_ci(head, b"transfer-encoding:")
        && header_name_present_ci(head, b"content-length:")
    {
        return false;
    }
    true
}

fn parse_get_path_and_query(head: &[u8]) -> Option<&str> {
    if !head.starts_with(b"GET ") {
        return None;
    }
    let rest = &head[4..];
    let end = rest.iter().position(|&b| b == b' ')?;
    std::str::from_utf8(&rest[..end]).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireWriteOutcome {
    /// Cap034 one-shot: caller must shut down the write half.
    Close,
    /// Known-length keep-alive: caller may read the next request on this socket.
    KeepAlive,
}

fn connection_close_requested(head: &[u8]) -> bool {
    for line in head.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.len() >= 11 && line[..11].eq_ignore_ascii_case(b"connection:") {
            let mut v = &line[11..];
            while v.first().copied() == Some(b' ') || v.first().copied() == Some(b'\t') {
                v = &v[1..];
            }
            // Token match for "close" (comma-separated Connection options).
            for token in v.split(|&b| b == b',') {
                let mut t = token;
                while t.first().copied() == Some(b' ') || t.first().copied() == Some(b'\t') {
                    t = &t[1..];
                }
                while t.last().copied() == Some(b' ') || t.last().copied() == Some(b'\t') {
                    t = &t[..t.len() - 1];
                }
                if t.eq_ignore_ascii_case(b"close") {
                    return true;
                }
            }
        }
    }
    false
}

/// Opt-in A/B: `EXYONQ_PROXY_ABORTIVE_CLOSE=1` — SO_LINGER(1,0) then drop on
/// `WireWriteOutcome::Close` (RST, no TIME-WAIT / FIN-WAIT-1). Default OFF =
/// Cap034 half-close until Netcup KEEP. Linux-only (ADR-045 linger capsule).
#[cfg(target_os = "linux")]
fn abortive_close_enabled() -> bool {
    matches!(
        std::env::var("EXYONQ_PROXY_ABORTIVE_CLOSE").ok().as_deref(),
        Some("1") | Some("true") | Some("on")
    )
}

#[cfg(target_os = "linux")]
tokio::task_local! {
    static PROXY_PEER_FD: Option<RawFd>;
}

/// Scope a peer TCP fd for the proxy wire serve task (Linux abortive-close A/B).
#[cfg(target_os = "linux")]
pub async fn with_proxy_peer_fd<F>(fd: Option<RawFd>, fut: F) -> F::Output
where
    F: std::future::Future,
{
    PROXY_PEER_FD.scope(fd, fut).await
}

#[cfg(target_os = "linux")]
fn scoped_proxy_peer_fd() -> Option<RawFd> {
    PROXY_PEER_FD.try_with(|v| *v).ok().flatten()
}

#[cfg(target_os = "linux")]
fn set_abortive_linger(fd: RawFd) {
    // Best-effort: failure falls through to normal drop.
    let _ = exyonq_linux_ffi::set_abortive_linger(fd);
}

/// End a one-shot admission after a close-framed response.
#[cfg(target_os = "linux")]
async fn finish_close_admission<S: AsyncWrite + Unpin>(stream: &mut S, peer_fd: Option<RawFd>) {
    if abortive_close_enabled() {
        if let Some(fd) = peer_fd.or_else(scoped_proxy_peer_fd) {
            set_abortive_linger(fd);
            return;
        }
    }
    let _ = stream.shutdown().await;
}

#[cfg(not(target_os = "linux"))]
async fn finish_close_admission<S: AsyncWrite + Unpin>(stream: &mut S) {
    let _ = stream.shutdown().await;
}

pub(crate) async fn write_proxy_response_wire<S: AsyncWrite + Unpin>(
    stream: &mut S,
    response: hyper::Response<BoxBody>,
    client_close: bool,
) -> Result<WireWriteOutcome> {
    let (parts, body) = response.into_parts();
    // Cap030: wire path must stream incrementally — never full-collect the upstream
    // body before the first downstream write (BUFFERED_PROXY is a product defect).
    write_streaming_proxy_response_wire(stream, parts, body, client_close).await
}

/// Encode proxy wire response head without intermediate `String` / `format!` temps.
///
/// Hop-by-hop skip (`Connection` / `Transfer-Encoding` / `Content-Length`) is preserved.
/// Framing follows [`crate::wire_framing::decide_downstream_framing`]: known-length
/// pass-through uses `Content-Length` + keep-alive (or close if the client asked);
/// otherwise Cap034 chunked+close.
#[cfg(test)]
fn encode_proxy_response_head(
    parts: &http::response::Parts,
    client_close: bool,
) -> (Vec<u8>, WireWriteOutcome) {
    let framing = crate::wire_framing::decide_downstream_framing(parts, client_close);
    encode_proxy_response_head_with_framing(parts, framing)
}

fn encode_proxy_response_head_with_framing(
    parts: &http::response::Parts,
    framing: crate::wire_framing::DownstreamFraming,
) -> (Vec<u8>, WireWriteOutcome) {
    let mut wire = Vec::with_capacity(256);
    wire.extend_from_slice(b"HTTP/1.1 ");
    wire.extend_from_slice(parts.status.as_str().as_bytes());
    if let Some(reason) = parts.status.canonical_reason() {
        wire.push(b' ');
        wire.extend_from_slice(reason.as_bytes());
    }
    wire.extend_from_slice(b"\r\n");

    for (name, value) in parts.headers.iter() {
        if name == hyper::header::CONNECTION
            || name == hyper::header::TRANSFER_ENCODING
            || name == hyper::header::CONTENT_LENGTH
        {
            continue;
        }
        // Same gate as the prior format! path: only UTF-8 HeaderValue text.
        if let Ok(value) = value.to_str() {
            wire.extend_from_slice(name.as_str().as_bytes());
            wire.extend_from_slice(b": ");
            wire.extend_from_slice(value.as_bytes());
            wire.extend_from_slice(b"\r\n");
        }
    }

    let outcome = match framing {
        crate::wire_framing::DownstreamFraming::KnownLengthKeepAlive { content_length } => {
            wire.extend_from_slice(b"Content-Length: ");
            push_u64_decimal(&mut wire, content_length);
            wire.extend_from_slice(b"\r\nConnection: keep-alive\r\n\r\n");
            WireWriteOutcome::KeepAlive
        }
        crate::wire_framing::DownstreamFraming::KnownLengthClose { content_length } => {
            wire.extend_from_slice(b"Content-Length: ");
            push_u64_decimal(&mut wire, content_length);
            wire.extend_from_slice(b"\r\nConnection: close\r\n\r\n");
            WireWriteOutcome::Close
        }
        crate::wire_framing::DownstreamFraming::ChunkedClose => {
            // Cap034: one request per admission — close after response.
            wire.extend_from_slice(b"Connection: close\r\n");
            wire.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
            WireWriteOutcome::Close
        }
    };
    (wire, outcome)
}

fn push_u64_decimal(buf: &mut Vec<u8>, mut n: u64) {
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    buf.extend_from_slice(&tmp[i..]);
}

/// Legacy encoder retained for byte-equivalence tests only.
#[cfg(test)]
fn encode_proxy_response_head_format_legacy(
    parts: &http::response::Parts,
    client_close: bool,
) -> Vec<u8> {
    encode_proxy_response_head(parts, client_close).0
}

async fn write_streaming_proxy_response_wire<S: AsyncWrite + Unpin>(
    stream: &mut S,
    parts: http::response::Parts,
    mut body: BoxBody,
    client_close: bool,
) -> Result<WireWriteOutcome> {
    #[cfg(feature = "p8-app-attribution")]
    crate::p8_app_attribution::note_request();
    #[cfg(feature = "p8-app-attribution")]
    let handler_timer = crate::p8_app_attribution::ScopeTimer::start();

    let framing = crate::wire_framing::decide_downstream_framing(&parts, client_close);
    let (wire, outcome) = encode_proxy_response_head_with_framing(&parts, framing);
    let content_length = framing.content_length();

    // Diagnostic-only full-buffer path (P8T STEP_3). Not product default.
    #[cfg(feature = "p8-transport-diag")]
    {
        use http_body_util::BodyExt;

        let mut pending = Vec::new();
        while let Some(frame) = body
            .frame()
            .await
            .transpose()
            .map_err(|_| io::Error::other("upstream body frame error"))?
        {
            if let Ok(chunk) = frame.into_data() {
                pending.extend_from_slice(&chunk);
            }
        }
        if let Some(expected) = content_length {
            if pending.len() as u64 != expected {
                return Err(io::Error::other("content-length mismatch"));
            }
            wire_io::write_all_async(stream, &wire).await?;
            wire_io::write_all_async(stream, &pending).await?;
            stream.flush().await?;
        } else {
            crate::streaming_wire_writer::write_finite_te_single_buffer(stream, &wire, &pending)
                .await?;
        }
        #[cfg(feature = "p8-app-attribution")]
        {
            let total = handler_timer.elapsed_ns();
            crate::p8_app_attribution::note_completed_stream(total, total);
        }
        return Ok(outcome);
    }

    // P8T-FIX A/B baseline: pre-fix 16 KiB batching (holds frames until SSE_FLUSH_CHUNK).
    #[cfg(all(not(feature = "p8-transport-diag"), feature = "p8t-fix-baseline"))]
    {
        if content_length.is_some() {
            write_streaming_cl_progressive(stream, &wire, &mut body, content_length.unwrap())
                .await?;
        } else {
            wire_io::write_all_async(stream, &wire).await?;
            let mut pending = Vec::new();
            let mut chunks_since_flush = 0usize;
            loop {
                let frame = body
                    .frame()
                    .await
                    .transpose()
                    .map_err(|_| io::Error::other("upstream body frame error"))?;
                let Some(frame) = frame else {
                    break;
                };
                if let Ok(chunk) = frame.into_data() {
                    pending.extend_from_slice(&chunk);
                    while pending.len() >= SSE_FLUSH_CHUNK {
                        write_chunk(stream, &pending[..SSE_FLUSH_CHUNK], false).await?;
                        pending.drain(..SSE_FLUSH_CHUNK);
                        chunks_since_flush += 1;
                        if chunks_since_flush >= SSE_FLUSH_INTERVAL {
                            stream.flush().await?;
                            chunks_since_flush = 0;
                        }
                    }
                }
            }
            if !pending.is_empty() {
                write_chunk(stream, &pending, true).await?;
            }
            stream.write_all(b"0\r\n\r\n").await?;
            stream.flush().await?;
        }
        #[cfg(feature = "p8-app-attribution")]
        {
            let total = handler_timer.elapsed_ns();
            crate::p8_app_attribution::note_completed_stream(total, total);
        }
        return Ok(outcome);
    }

    // P8T-FIX product default: progressive per-frame write + flush.
    #[cfg(all(not(feature = "p8-transport-diag"), not(feature = "p8t-fix-baseline")))]
    {
        if let Some(expected) = content_length {
            write_streaming_cl_progressive(stream, &wire, &mut body, expected).await?;
        } else {
            write_streaming_te_progressive(stream, &wire, &mut body).await?;
        }
        #[cfg(feature = "p8-app-attribution")]
        {
            let total = handler_timer.elapsed_ns();
            crate::p8_app_attribution::note_completed_stream(total, total);
        }
        Ok(outcome)
    }
}

/// Known-length body: stream bytes and fail closed if count ≠ Content-Length.
///
/// For small CL (P4 1 KiB class), write headers first (Cap030 TTBF), coalesce the
/// upstream body into one buffer, then a single body write — cuts per-frame poll
/// and Bytes churn under keep-alive load. Larger CL stays progressive.
/// Default coalesce is OFF (Netcup A/B NEUTRAL); opt-in `EXYONQ_PROXY_CL_COALESCE=1`.
async fn write_streaming_cl_progressive<S: AsyncWrite + Unpin>(
    stream: &mut S,
    headers: &[u8],
    body: &mut BoxBody,
    content_length: u64,
) -> Result<()> {
    write_streaming_cl_progressive_any(stream, headers, body, content_length).await
}

async fn write_streaming_cl_progressive_any<S, B>(
    stream: &mut S,
    headers: &[u8],
    body: &mut B,
    content_length: u64,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    if small_cl_coalesce_enabled() && content_length <= SMALL_CL_COALESCE_MAX {
        return write_streaming_cl_coalesce_small_any(stream, headers, body, content_length).await;
    }
    write_streaming_cl_frame_progressive_any(stream, headers, body, content_length).await
}

/// Cap030: headers are the first downstream write; body may then be coalesced for small CL.
const SMALL_CL_COALESCE_MAX: u64 = 16 * 1024;

fn small_cl_coalesce_enabled() -> bool {
    use std::sync::atomic::{AtomicI8, Ordering};
    static CACHE: AtomicI8 = AtomicI8::new(-1);
    let cached = CACHE.load(Ordering::Relaxed);
    if cached >= 0 {
        return cached != 0;
    }
    // Default OFF: Netcup A/B p4-cl-coalesce-ab showed NEUTRAL/slight regress vs progressive
    // (B_vs_A ≈ −0.9% RPS within noise). Opt-in: EXYONQ_PROXY_CL_COALESCE=1.
    let on = matches!(
        std::env::var("EXYONQ_PROXY_CL_COALESCE").ok().as_deref(),
        Some("1") | Some("true") | Some("on")
    );
    CACHE.store(if on { 1 } else { 0 }, Ordering::Relaxed);
    on
}

async fn write_streaming_cl_coalesce_small_any<S, B>(
    stream: &mut S,
    headers: &[u8],
    body: &mut B,
    content_length: u64,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    use http_body_util::BodyExt;
    wire_io::write_all_async(stream, headers).await?;
    let cap = usize::try_from(content_length).unwrap_or(usize::MAX);
    let mut pending = Vec::with_capacity(cap);
    loop {
        let frame = body
            .frame()
            .await
            .transpose()
            .map_err(|_| io::Error::other("upstream body frame error"))?;
        let Some(frame) = frame else {
            break;
        };
        if let Ok(chunk) = frame.into_data() {
            let next = pending
                .len()
                .checked_add(chunk.len())
                .ok_or_else(|| io::Error::other("content-length overflow"))?;
            if next as u64 > content_length {
                return Err(io::Error::other("content-length overrun"));
            }
            pending.extend_from_slice(&chunk);
        }
    }
    if pending.len() as u64 != content_length {
        return Err(io::Error::other("content-length mismatch"));
    }
    if !pending.is_empty() {
        wire_io::write_all_async(stream, &pending).await?;
    }
    stream.flush().await?;
    Ok(())
}

async fn write_streaming_cl_frame_progressive_any<S, B>(
    stream: &mut S,
    headers: &[u8],
    body: &mut B,
    content_length: u64,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    use http_body_util::BodyExt;
    wire_io::write_all_async(stream, headers).await?;
    let mut written: u64 = 0;
    loop {
        let frame = body
            .frame()
            .await
            .transpose()
            .map_err(|_| io::Error::other("upstream body frame error"))?;
        let Some(frame) = frame else {
            break;
        };
        if let Ok(chunk) = frame.into_data() {
            let n = chunk.len() as u64;
            written = written
                .checked_add(n)
                .ok_or_else(|| io::Error::other("content-length overflow"))?;
            if written > content_length {
                return Err(io::Error::other("content-length overrun"));
            }
            wire_io::write_all_async(stream, &chunk).await?;
        }
    }
    if written != content_length {
        return Err(io::Error::other("content-length mismatch"));
    }
    stream.flush().await?;
    Ok(())
}

/// P8T-FIX product path: progressive-safe stream writer.
///
/// Emits each upstream data frame immediately (TE frame+data via `write_chunk`)
/// so sparse SSE events are not held until `SSE_FLUSH_CHUNK`.
///
/// Finite Ready-drain coalesce (EXPERIMENT_4) was measured on Netcup P8O-A and
/// **regressed** vs 16 KiB-batch coalesce baseline (~−7% RPS). It is not the
/// product default — retained only as a unit-test helper. Competitive
/// single-write shape remains diagnostic (`p8-transport-diag`) or future
/// platform writev/cork (separate admit).
#[cfg(any(test, not(feature = "p8-transport-diag")))]
async fn write_streaming_te_progressive<S: AsyncWrite + Unpin>(
    stream: &mut S,
    headers: &[u8],
    body: &mut BoxBody,
) -> Result<()> {
    write_streaming_te_progressive_any(stream, headers, body).await
}

#[cfg(any(test, not(feature = "p8-transport-diag")))]
async fn write_streaming_te_progressive_any<S, B>(
    stream: &mut S,
    headers: &[u8],
    body: &mut B,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    use http_body_util::BodyExt;
    wire_io::write_all_async(stream, headers).await?;

    loop {
        let frame = body
            .frame()
            .await
            .transpose()
            .map_err(|_| io::Error::other("upstream body frame error"))?;
        let Some(frame) = frame else {
            break;
        };
        if let Ok(chunk) = frame.into_data() {
            if chunk.is_empty() {
                continue;
            }
            let mut offset = 0usize;
            while offset < chunk.len() {
                let end = (offset + SSE_FLUSH_CHUNK).min(chunk.len());
                let last = end == chunk.len();
                write_chunk(stream, &chunk[offset..end], last).await?;
                offset = end;
            }
        }
    }

    stream.write_all(b"0\r\n\r\n").await?;
    stream.flush().await?;
    Ok(())
}

/// Retained for unit tests + future finite-cap experiments (not product default).
#[cfg(test)]
async fn write_finite_headers_frame_trailer<S: AsyncWrite + Unpin>(
    stream: &mut S,
    headers: &[u8],
    body: &[u8],
) -> Result<()> {
    let mut hex = [0u8; 24];
    let hex_len = write_chunk_hex_len(body.len(), &mut hex);
    let total = headers.len() + hex_len + 2 + body.len() + 2 + 5;
    const STACK_MAX: usize = 8192;
    if total <= STACK_MAX {
        let mut wire = [0u8; STACK_MAX];
        let mut n = 0usize;
        wire[n..n + headers.len()].copy_from_slice(headers);
        n += headers.len();
        if !body.is_empty() {
            wire[n..n + hex_len].copy_from_slice(&hex[..hex_len]);
            n += hex_len;
            wire[n..n + 2].copy_from_slice(b"\r\n");
            n += 2;
            wire[n..n + body.len()].copy_from_slice(body);
            n += body.len();
            wire[n..n + 2].copy_from_slice(b"\r\n");
            n += 2;
        }
        wire[n..n + 5].copy_from_slice(b"0\r\n\r\n");
        n += 5;
        stream.write_all(&wire[..n]).await?;
    } else {
        let mut wire = Vec::with_capacity(total);
        wire.extend_from_slice(headers);
        if !body.is_empty() {
            wire.extend_from_slice(&hex[..hex_len]);
            wire.extend_from_slice(b"\r\n");
            wire.extend_from_slice(body);
            wire.extend_from_slice(b"\r\n");
        }
        wire.extend_from_slice(b"0\r\n\r\n");
        stream.write_all(&wire).await?;
    }
    stream.flush().await?;
    Ok(())
}
pub(crate) fn write_chunk_hex_len(len: usize, out: &mut [u8; 24]) -> usize {
    if len == 0 {
        out[0] = b'0';
        return 1;
    }
    let mut digits = [0u8; 16];
    let mut n = len;
    let mut count = 0usize;
    while n > 0 {
        let d = (n & 0xf) as u8;
        digits[count] = b"0123456789abcdef"[d as usize];
        n >>= 4;
        count += 1;
    }
    for i in 0..count {
        out[i] = digits[count - 1 - i];
    }
    count
}

#[cfg(any(test, not(feature = "p8-transport-diag")))]
async fn write_chunk<S: AsyncWrite + Unpin>(
    stream: &mut S,
    data: &[u8],
    flush: bool,
) -> Result<()> {
    // Product default: writev hex|CRLF|data|CRLF — borrows `data` without assembling a
    // contiguous TE buffer (avoids per-chunk malloc + body copy on the P4 1 KiB path).
    // Feature `p8tp-vectored` remains as an explicit alias; assemble path is gone.
    #[cfg(feature = "p8-app-attribution")]
    let frame_timer = crate::p8_app_attribution::ScopeTimer::start();
    let mut header = [0u8; 24];
    let hex_len = write_chunk_hex_len(data.len(), &mut header);
    #[cfg(feature = "p8-app-attribution")]
    let frame_ns = frame_timer.elapsed_ns();
    #[cfg(feature = "p8-app-attribution")]
    let write_timer = crate::p8_app_attribution::ScopeTimer::start();

    let write_result = {
        use std::io::IoSlice;
        let mut bufs = [
            IoSlice::new(&header[..hex_len]),
            IoSlice::new(b"\r\n"),
            IoSlice::new(data),
            IoSlice::new(b"\r\n"),
        ];
        let mut slices: &mut [IoSlice<'_>] = &mut bufs;
        let mut result = Ok(());
        while !slices.is_empty() {
            match stream.write_vectored(slices).await {
                Ok(0) => {
                    result = Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "write_vectored returned 0",
                    ));
                    break;
                }
                Ok(n) => IoSlice::advance_slices(&mut slices, n),
                Err(err) => {
                    result = Err(err);
                    break;
                }
            }
        }
        result
    };

    #[cfg(feature = "p8-app-attribution")]
    {
        let write_ns = write_timer.elapsed_ns();
        if write_result.is_err() {
            crate::p8_app_attribution::note_client_disconnect();
        } else {
            crate::p8_app_attribution::note_write_chunk(data.len(), frame_ns, write_ns, flush);
        }
    }
    write_result?;
    if flush {
        #[cfg(feature = "p8-app-attribution")]
        let flush_timer = crate::p8_app_attribution::ScopeTimer::start();
        let flush_result = stream.flush().await;
        #[cfg(feature = "p8-app-attribution")]
        crate::p8_app_attribution::note_flush(flush_timer.elapsed_ns());
        flush_result?;
    }
    Ok(())
}

/// Fixed error responses for wire fail-closed paths (LA-PROXY-SILENT-OK-001).
async fn write_fixed_close<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status_line_and_body: &'static [u8],
) -> Result<()> {
    wire_io::write_all_async(stream, status_line_and_body).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

pub async fn serve<S>(
    cluster_id: u32,
    generation: u64,
    mut stream: S,
    first_head: Bytes,
    _carry: Bytes,
    x_forwarded_for: HeaderValue,
    #[cfg(target_os = "linux")] peer_fd: Option<RawFd>,
) -> Result<u16>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let rt = runtime();
    // Generation mismatch: fail-closed with 503 — never silent Ok without a response.
    if rt.cluster_generation() != generation {
        write_fixed_close(
            &mut stream,
            b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 19\r\n\r\nService Unavailable",
        )
        .await?;
        exyonq_module_api::wire_record_response(503);
        return Ok(503);
    }

    let pinned_host = host_header_raw(first_head.as_ref()).map(|h| h.to_vec());
    let mut head = first_head;
    let mut admission_status: u16 = 200;
    // The core wire owner admits the first request before WAF. This loop owns every
    // subsequent keepalive request, so it admits those here before upstream dispatch.
    let mut first_request_was_admitted = true;
    // Resolve once per admission. Peer health is rechecked on Hyper fallback /
    // connect failure; P4 KA stays on the same upstream for the TCP lifetime
    // (matches common reverse-proxy connection affinity).
    let mut cached_target: Option<crate::upstream_target::UpstreamTarget> = None;

    loop {
        let this_host = host_header_raw(head.as_ref()).map(|h| h.to_vec());
        if let Some(pinned) = pinned_host.as_deref() {
            let same = this_host
                .as_deref()
                .is_some_and(|next| pinned.eq_ignore_ascii_case(next));
            if !same && !first_request_was_admitted {
                let _ = stream.shutdown().await;
                return Ok(admission_status);
            }
        }

        let Some(path_and_query) = parse_get_path_and_query(head.as_ref()) else {
            write_fixed_close(
                &mut stream,
                b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 11\r\n\r\nBad Request",
            )
            .await?;
            exyonq_module_api::wire_record_response(400);
            return Ok(400);
        };

        if first_request_was_admitted {
            first_request_was_admitted = false;
        } else {
            let client_ip = x_forwarded_for.to_str().unwrap_or("127.0.0.1");
            if let exyonq_module_api::WireAdmit::Reject429 { retry_after_secs } =
                exyonq_module_api::wire_admit_for_path(client_ip, path_and_query.split('?').next().unwrap_or(path_and_query))
            {
                let reject = exyonq_module_api::rate_limit_reject_wire(retry_after_secs);
                wire_io::write_all_async(&mut stream, &reject).await?;
                let _ = stream.shutdown().await;
                exyonq_module_api::wire_record_response(429);
                return Ok(429);
            }
        }

        // Cap024/dispatch alignment: no eligible upstream (incl. all unhealthy) → 503, not 404.
        if cached_target.is_none() {
            match rt.upstream_for_cluster(cluster_id) {
                Some(t) => {
                    cached_target = Some(t);
                }
                None => {
                    write_fixed_close(
                        &mut stream,
                        b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 19\r\n\r\nService Unavailable",
                    )
                    .await?;
                    exyonq_module_api::wire_record_response(503);
                    return Ok(503);
                }
            }
        }
        if let (Some(target), Some(raw)) = (cached_target.as_mut(), this_host.as_deref()) {
            if let Ok(value) = HeaderValue::from_bytes(raw) {
                target.host = Some(value);
            }
        }
        let Some(target) = cached_target.as_ref() else {
            write_fixed_close(
                &mut stream,
                b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 19\r\n\r\nService Unavailable",
            )
            .await?;
            exyonq_module_api::wire_record_response(503);
            return Ok(503);
        };

        let client_close = connection_close_requested(head.as_ref());
        let started = std::time::Instant::now();
        let (last_status, outcome) = if crate::raw_upstream::raw_upstream_get_enabled() {
            match crate::raw_upstream::stream_get_to_client(
                target,
                path_and_query,
                Some(&x_forwarded_for),
                &mut stream,
                client_close,
            )
            .await
            {
                Ok(status) => {
                    let outcome = if client_close {
                        WireWriteOutcome::Close
                    } else {
                        WireWriteOutcome::KeepAlive
                    };
                    (status, outcome)
                }
                Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                    write_fixed_close(
                        &mut stream,
                        b"HTTP/1.1 504 Gateway Timeout\r\nConnection: close\r\nContent-Length: 15\r\n\r\nGateway Timeout",
                    )
                    .await?;
                    exyonq_module_api::wire_record_response(504);
                    return Ok(504);
                }
                Err(_) => {
                    let response = rt
                        .forward_get_for_wire(
                            cluster_id,
                            target,
                            path_and_query,
                            Some(&x_forwarded_for),
                        )
                        .await;
                    let status = response.status().as_u16();
                    let outcome =
                        write_proxy_response_wire(&mut stream, response, client_close).await?;
                    (status, outcome)
                }
            }
        } else {
            let response = rt
                .forward_get_for_wire(cluster_id, target, path_and_query, Some(&x_forwarded_for))
                .await;
            let status = response.status().as_u16();
            let outcome = write_proxy_response_wire(&mut stream, response, client_close).await?;
            (status, outcome)
        };
        admission_status = last_status;
        let path = path_and_query.split('?').next().unwrap_or(path_and_query);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        exyonq_module_api::wire_record_exchange("GET", path, last_status, elapsed_ms);
        match outcome {
            WireWriteOutcome::Close => {
                // Cap034 one-shot: half-close by default. Abortive linger is
                // opt-in via EXYONQ_PROXY_ABORTIVE_CLOSE (P5 A/B, Linux).
                #[cfg(target_os = "linux")]
                finish_close_admission(&mut stream, peer_fd).await;
                #[cfg(not(target_os = "linux"))]
                finish_close_admission(&mut stream).await;
                return Ok(last_status);
            }
            WireWriteOutcome::KeepAlive => {
                match read_next_get_head(&mut stream).await {
                    Ok(Some(next)) => {
                        // LA-001: Host switch on the same TCP must not reuse a frozen cluster.
                        // Fail closed by ending the admission when Host changes.
                        // Compare ASCII-case-insensitive without allocating lowercase copies.
                        let next_host = host_header_raw(next.as_ref());
                        let host_ok = match (pinned_host.as_deref(), next_host) {
                            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                            (None, None) => true,
                            _ => false,
                        };
                        if !host_ok {
                            let _ = stream.shutdown().await;
                            return Ok(last_status);
                        }
                        if !might_use_proxy_wire(next.as_ref()) {
                            let _ = stream.shutdown().await;
                            return Ok(last_status);
                        }
                        head = next;
                    }
                    Ok(None) | Err(_) => {
                        let _ = stream.shutdown().await;
                        return Ok(last_status);
                    }
                }
            }
        }
    }
}

fn host_header_raw(head: &[u8]) -> Option<&[u8]> {
    for line in head.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.len() >= 5 && line[..5].eq_ignore_ascii_case(b"host:") {
            let mut v = &line[5..];
            while v.first().copied() == Some(b' ') || v.first().copied() == Some(b'\t') {
                v = &v[1..];
            }
            return Some(v);
        }
    }
    None
}

async fn read_next_get_head<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Option<Bytes>> {
    use tokio::io::AsyncReadExt;
    let mut buf = bytes::BytesMut::with_capacity(512);
    let deadline = tokio::time::Instant::now() + wire_io::header_read_timeout();
    loop {
        if let Some(end) = wire_io::find_header_end(&buf) {
            let head_end = end + 4;
            if head_end > wire_io::MAX_HEADER {
                return Err(io::Error::other("headers too large"));
            }
            let head = buf.split_to(head_end).freeze();
            // Trailing body bytes on a GET are unexpected; drop the connection.
            if !buf.is_empty() {
                return Ok(None);
            }
            return Ok(Some(head));
        }
        if buf.len() >= wire_io::MAX_HEADER {
            return Err(io::Error::other("headers too large"));
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "header read timeout",
            ));
        }
        let mut tmp = [0u8; 1024];
        let n = match tokio::time::timeout(remaining, stream.read(&mut tmp)).await {
            Ok(Ok(0)) => return Ok(None),
            Ok(Ok(n)) => n,
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "header read timeout",
                ))
            }
        };
        buf.extend_from_slice(&tmp[..n]);
    }
}

// Cap034: header completeness / partial-read / head-body split for the first
// request live in `core::server::io::read_until_headers_*` before dispatch.
// Keep-alive re-entry for known-length responses is handled above with Host
// stability checks (LA-001 fail-closed on Host switch).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ProxyRuntime;
    use exyonq_module_api::proxy_dispatch::ProxyCompiledSlot;
    use http_body_util::BodyExt;
    use hyper::body::{Body, Frame};
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    fn install_test_runtime() -> Arc<ProxyRuntime> {
        let rt = Arc::new(ProxyRuntime::new());
        pin_runtime(Arc::clone(&rt));
        rt.bind_compiled_slots(
            1,
            &[ProxyCompiledSlot::legacy_single(
                0,
                "backend",
                "http://127.0.0.1:9000",
                Duration::from_millis(500),
            )],
        );
        rt
    }

    #[test]
    fn wire_proxy_get_api_resolves_predicate() {
        let head = b"GET /api/users HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert!(might_use_proxy_wire(head));
        let path = parse_get_path_and_query(head).expect("path");
        assert_eq!(path, "/api/users");
    }

    #[test]
    fn might_use_rejects_invalid_te_cl_combo() {
        let head =
            b"GET /api/x HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n";
        assert!(!might_use_proxy_wire(head));
    }

    #[test]
    fn might_use_rejects_invalid_te_cl_combo_lowercase() {
        let head =
            b"GET /api/x HTTP/1.1\r\ntransfer-encoding: chunked\r\ncontent-length: 0\r\n\r\n";
        assert!(!might_use_proxy_wire(head));
    }

    #[tokio::test]
    async fn serve_generation_mismatch_writes_503() {
        let rt = Arc::new(ProxyRuntime::new());
        pin_runtime(Arc::clone(&rt));
        // Bound generation 0; serve with mismatched generation 99.
        rt.bind_compiled_slots(1, &[]);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
            let mut buf = vec![0u8; 256];
            let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                .await
                .expect("read");
            buf.truncate(n);
            buf
        });
        let (server, _) = listener.accept().await.expect("accept");
        #[cfg(target_os = "linux")]
        let peer_fd = Some(std::os::fd::AsRawFd::as_raw_fd(&server));
        let status = serve(
            0,
            99,
            server,
            Bytes::from_static(b"GET /api/x HTTP/1.1\r\nHost: x\r\n\r\n"),
            Bytes::new(),
            HeaderValue::from_static("127.0.0.1"),
            #[cfg(target_os = "linux")]
            peer_fd,
        )
        .await
        .expect("serve");
        assert_eq!(status, 503);
        let body = client.await.expect("join");
        let s = String::from_utf8_lossy(&body);
        assert!(s.starts_with("HTTP/1.1 503"), "{s}");
    }

    #[tokio::test]
    async fn serve_path_parse_miss_writes_400() {
        // LA-PROXY-PATH400-COV-001: non-UTF-8 path → fail-closed 400 (not silent Ok).
        let rt = Arc::new(ProxyRuntime::new());
        pin_runtime(Arc::clone(&rt));
        rt.bind_compiled_slots(1, &[]);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
            let mut buf = vec![0u8; 256];
            let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                .await
                .expect("read");
            buf.truncate(n);
            buf
        });
        let (server, _) = listener.accept().await.expect("accept");
        #[cfg(target_os = "linux")]
        let peer_fd = Some(std::os::fd::AsRawFd::as_raw_fd(&server));
        let head = Bytes::from(vec![
            b'G', b'E', b'T', b' ', b'/', b'a', b'p', b'i', b'/', 0xff, b' ', b'H', b'T', b'T',
            b'P', b'/', b'1', b'.', b'1', b'\r', b'\n', b'H', b'o', b's', b't', b':', b' ', b'x',
            b'\r', b'\n', b'\r', b'\n',
        ]);
        assert!(
            might_use_proxy_wire(head.as_ref()),
            "eligibility must admit before path parse"
        );
        assert!(
            parse_get_path_and_query(head.as_ref()).is_none(),
            "path must fail UTF-8"
        );
        let status = serve(
            0,
            1,
            server,
            head,
            Bytes::new(),
            HeaderValue::from_static("127.0.0.1"),
            #[cfg(target_os = "linux")]
            peer_fd,
        )
        .await
        .expect("serve");
        assert_eq!(status, 400);
        let body = client.await.expect("join");
        let s = String::from_utf8_lossy(&body);
        assert!(s.starts_with("HTTP/1.1 400"), "{s}");
    }

    #[test]
    fn might_use_rejects_websocket_upgrade() {
        let head = b"GET /api/ws-echo HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
        assert!(!might_use_proxy_wire(head));
    }

    #[test]
    fn might_use_rejects_websocket_connection_close_upgrade() {
        let head = b"GET /api/ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: close, Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
        assert!(!might_use_proxy_wire(head));
    }

    #[tokio::test]
    async fn upstream_for_cluster_after_bind() {
        let rt = install_test_runtime();
        let target = rt.upstream_for_cluster(0).expect("cluster 0");
        assert!(target
            .uri_for("/api/health")
            .to_string()
            .contains("127.0.0.1"));
    }

    #[test]
    fn cluster_generation_tracks_bind() {
        let rt = install_test_runtime();
        assert_eq!(rt.cluster_generation(), 1);
        rt.bind_compiled_slots(
            2,
            &[ProxyCompiledSlot::legacy_single(
                0,
                "backend",
                "http://127.0.0.1:9001",
                Duration::from_millis(500),
            )],
        );
        assert_eq!(rt.cluster_generation(), 2);
    }

    #[test]
    fn write_chunk_hex_len_encodes_sizes() {
        let mut buf = [0u8; 24];
        assert_eq!(write_chunk_hex_len(0, &mut buf), 1);
        assert_eq!(buf[0], b'0');
        assert_eq!(write_chunk_hex_len(12, &mut buf), 1);
        assert_eq!(buf[0], b'c');
        assert_eq!(write_chunk_hex_len(425, &mut buf), 3);
        assert_eq!(&buf[..3], b"1a9");
    }

    #[tokio::test]
    async fn write_chunk_emits_single_te_frame() {
        let mut out = Vec::new();
        write_chunk(&mut out, b"data: hello\n\n", false)
            .await
            .expect("write");
        assert_eq!(out, b"d\r\ndata: hello\n\n\r\n");
    }

    /// P4_PROXY_1K regression: 1024-byte body must TE-frame correctly via writev
    /// without requiring a heap-assembled contiguous buffer (STACK_MAX=512 used to
    /// force Vec alloc + body copy on this size).
    #[tokio::test]
    async fn write_chunk_te_frames_1k_body_without_assemble() {
        let body = vec![0xABu8; 1024];
        let mut out = Vec::new();
        write_chunk(&mut out, &body, true).await.expect("write 1k");
        // 1024 == 0x400
        assert!(out.starts_with(b"400\r\n"));
        assert_eq!(&out[5..5 + 1024], body.as_slice());
        assert_eq!(&out[5 + 1024..], b"\r\n");
    }

    #[tokio::test]
    async fn write_finite_headers_frame_trailer_single_write() {
        let headers = b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n";
        let body = b"data: chunk-0\n\ndata: chunk-1\n\n";
        let mut out = Vec::new();
        write_finite_headers_frame_trailer(&mut out, headers, body)
            .await
            .expect("write");
        let expected_prefix = b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert!(out.starts_with(expected_prefix));
        assert!(out.ends_with(b"0\r\n\r\n"));
        assert!(out.windows(body.len()).any(|w| w == body));
    }

    #[tokio::test]
    async fn finite_helper_coalesces_under_cap() {
        let headers = b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n";
        let payload = b"data: chunk-0\n\ndata: chunk-1\n\n";
        let mut out = Vec::new();
        write_finite_headers_frame_trailer(&mut out, headers, payload)
            .await
            .expect("finite");
        assert!(out.starts_with(headers));
        assert!(out.ends_with(b"0\r\n\r\n"));
        assert!(out.windows(payload.len()).any(|w| w == payload));
    }

    #[tokio::test]
    async fn progressive_emits_before_eos() {
        use std::convert::Infallible;
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::sync::mpsc;

        struct ChanBody {
            rx: mpsc::Receiver<Option<Bytes>>,
        }
        impl Body for ChanBody {
            type Data = Bytes;
            type Error = Infallible;
            fn poll_frame(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<Option<std::result::Result<Frame<Self::Data>, Self::Error>>> {
                match self.rx.poll_recv(cx) {
                    Poll::Ready(Some(Some(b))) => Poll::Ready(Some(Ok(Frame::data(b)))),
                    Poll::Ready(Some(None)) | Poll::Ready(None) => Poll::Ready(None),
                    Poll::Pending => Poll::Pending,
                }
            }
        }

        struct ProbeWrite {
            inner: Vec<u8>,
            saw_one_before_two: Arc<AtomicBool>,
        }
        impl AsyncWrite for ProbeWrite {
            fn poll_write(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::result::Result<usize, io::Error>> {
                self.inner.extend_from_slice(buf);
                let s = String::from_utf8_lossy(&self.inner);
                if s.contains("data: one") && !s.contains("data: two") {
                    self.saw_one_before_two.store(true, Ordering::SeqCst);
                }
                Poll::Ready(Ok(buf.len()))
            }
            fn poll_flush(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::result::Result<(), io::Error>> {
                Poll::Ready(Ok(()))
            }
            fn poll_shutdown(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::result::Result<(), io::Error>> {
                Poll::Ready(Ok(()))
            }
        }

        let flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Option<Bytes>>(4);
        tokio::spawn(async move {
            let _ = tx.send(Some(Bytes::from_static(b"data: one\n\n"))).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = tx.send(Some(Bytes::from_static(b"data: two\n\n"))).await;
            let _ = tx.send(None).await;
        });

        let headers = b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n";
        let mut body: BoxBody = ChanBody { rx }.map_err(|n| match n {}).boxed();
        let mut out = ProbeWrite {
            inner: Vec::new(),
            saw_one_before_two: Arc::clone(&flag),
        };
        write_streaming_te_progressive(&mut out, headers, &mut body)
            .await
            .expect("dual");
        let s = String::from_utf8_lossy(&out.inner);
        assert!(s.contains("data: one"));
        assert!(s.contains("data: two"));
        assert!(s.ends_with("0\r\n\r\n"));
        assert!(
            flag.load(Ordering::SeqCst),
            "expected progressive write of first event before second"
        );
    }

    /// Cap034 fallback: missing Content-Length still uses TE chunked + Connection: close.
    #[tokio::test]
    async fn wire_proxy_response_without_cl_advertises_connection_close() {
        struct CollectWrite(Vec<u8>);
        impl AsyncWrite for CollectWrite {
            fn poll_write(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::result::Result<usize, io::Error>> {
                self.0.extend_from_slice(buf);
                Poll::Ready(Ok(buf.len()))
            }
            fn poll_flush(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::result::Result<(), io::Error>> {
                Poll::Ready(Ok(()))
            }
            fn poll_shutdown(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::result::Result<(), io::Error>> {
                Poll::Ready(Ok(()))
            }
        }

        let body: BoxBody = http_body_util::Full::new(Bytes::from_static(b"ok"))
            .map_err(|never| match never {})
            .boxed();
        let (parts, _) = http::Response::builder()
            .status(200)
            .header("Content-Type", "application/octet-stream")
            .body(())
            .expect("response")
            .into_parts();
        let mut out = CollectWrite(Vec::new());
        let outcome = write_streaming_proxy_response_wire(&mut out, parts, body, false)
            .await
            .expect("write");
        assert_eq!(outcome, WireWriteOutcome::Close);
        let s = String::from_utf8_lossy(&out.0);
        assert!(
            s.contains("Connection: close\r\n"),
            "missing Connection: close in wire headers: {s}"
        );
        assert!(
            s.contains("Transfer-Encoding: chunked\r\n"),
            "expected TE chunked: {s}"
        );
        assert!(s.contains("ok"), "body missing: {s}");
    }

    /// Known-length upstream: Content-Length + keep-alive (product fix for P4/P5/P6).
    #[tokio::test]
    async fn wire_proxy_response_with_cl_advertises_keepalive() {
        struct CollectWrite(Vec<u8>);
        impl AsyncWrite for CollectWrite {
            fn poll_write(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::result::Result<usize, io::Error>> {
                self.0.extend_from_slice(buf);
                Poll::Ready(Ok(buf.len()))
            }
            fn poll_flush(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::result::Result<(), io::Error>> {
                Poll::Ready(Ok(()))
            }
            fn poll_shutdown(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::result::Result<(), io::Error>> {
                Poll::Ready(Ok(()))
            }
        }

        let body: BoxBody = http_body_util::Full::new(Bytes::from_static(b"ok"))
            .map_err(|never| match never {})
            .boxed();
        let (parts, _) = http::Response::builder()
            .status(200)
            .header("Content-Type", "application/octet-stream")
            .header("Content-Length", "2")
            .body(())
            .expect("response")
            .into_parts();
        let mut out = CollectWrite(Vec::new());
        let outcome = write_streaming_proxy_response_wire(&mut out, parts, body, false)
            .await
            .expect("write");
        assert_eq!(outcome, WireWriteOutcome::KeepAlive);
        let s = String::from_utf8_lossy(&out.0);
        assert!(
            s.contains("Connection: keep-alive\r\n"),
            "missing keep-alive: {s}"
        );
        assert!(
            s.contains("Content-Length: 2\r\n"),
            "missing Content-Length: {s}"
        );
        assert!(
            !s.contains("Transfer-Encoding:"),
            "TE must not appear on known-length: {s}"
        );
        assert!(s.contains("ok"), "body missing: {s}");
    }

    fn parts_with(status: u16, headers: &[(&str, &str)]) -> http::response::Parts {
        let mut b = http::Response::builder().status(status);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(()).expect("response").into_parts().0
    }

    #[test]
    fn encode_head_known_length_emits_cl_keepalive() {
        let parts = parts_with(
            200,
            &[
                ("Content-Type", "application/octet-stream"),
                ("Content-Length", "1024"),
                ("Server", "upstream"),
                ("X-Trace", "abc"),
            ],
        );
        let (wire, outcome) = encode_proxy_response_head(&parts, false);
        assert_eq!(outcome, WireWriteOutcome::KeepAlive);
        assert_eq!(
            wire,
            encode_proxy_response_head_format_legacy(&parts, false)
        );
        let s = String::from_utf8_lossy(&wire);
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(
            s.contains("content-type: application/octet-stream\r\n")
                || s.contains("Content-Type: application/octet-stream\r\n")
        );
        assert!(s.contains("Content-Length: 1024\r\n"));
        assert!(s.contains("Connection: keep-alive\r\n"));
        assert!(!s.contains("Transfer-Encoding:"));
        assert!(s.ends_with("Connection: keep-alive\r\n\r\n") || s.contains("\r\n\r\n"));
    }

    #[test]
    fn encode_head_byte_equivalent_non_200_and_minimal() {
        let parts = parts_with(502, &[("Content-Type", "text/plain")]);
        let (wire, outcome) = encode_proxy_response_head(&parts, false);
        assert_eq!(outcome, WireWriteOutcome::Close);
        assert_eq!(
            wire,
            encode_proxy_response_head_format_legacy(&parts, false)
        );
        let s = String::from_utf8_lossy(&wire);
        assert!(s.starts_with("HTTP/1.1 502 Bad Gateway\r\n"));
        assert!(s.contains("Connection: close\r\n"));
        assert!(s.ends_with("Transfer-Encoding: chunked\r\n\r\n"));

        let empty = parts_with(200, &[]);
        let (empty_wire, empty_outcome) = encode_proxy_response_head(&empty, false);
        assert_eq!(empty_outcome, WireWriteOutcome::Close);
        assert_eq!(
            encode_proxy_response_head_format_legacy(&empty, false),
            empty_wire
        );
    }

    #[test]
    fn encode_head_te_present_stays_chunked_close_even_with_cl() {
        for cl in ["0", "1024", "1048576"] {
            let parts = parts_with(
                200,
                &[
                    ("X-A", "1"),
                    ("Connection", "keep-alive"),
                    ("X-B", "2"),
                    ("Transfer-Encoding", "chunked"),
                    ("Content-Length", cl),
                    ("X-C", "3"),
                ],
            );
            let (wire, outcome) = encode_proxy_response_head(&parts, false);
            assert_eq!(outcome, WireWriteOutcome::Close);
            assert_eq!(
                wire,
                encode_proxy_response_head_format_legacy(&parts, false)
            );
            let s = String::from_utf8_lossy(&wire).to_ascii_lowercase();
            // Upstream CL is stripped; Cap034 emits TE+close (not passthrough CL).
            assert!(!s.contains("content-length:"), "cl={cl} leaked into wire");
            assert_eq!(s.matches("transfer-encoding:").count(), 1);
            assert_eq!(s.matches("connection:").count(), 1);
            assert!(s.contains("connection: close\r\n"));
            let ia = s.find("x-a: 1").expect("x-a");
            let ib = s.find("x-b: 2").expect("x-b");
            let ic = s.find("x-c: 3").expect("x-c");
            assert!(ia < ib && ib < ic, "retained header order");
        }
    }

    #[test]
    fn encode_head_supports_edge_header_values_already_in_contract() {
        let parts = parts_with(
            200,
            &[
                ("X-Empty", ""),
                ("X-Space", "a b"),
                ("X-Punct", "a=b;c=\"d\""),
            ],
        );
        let (wire, outcome) = encode_proxy_response_head(&parts, false);
        assert_eq!(outcome, WireWriteOutcome::Close);
        assert_eq!(
            wire,
            encode_proxy_response_head_format_legacy(&parts, false)
        );
        let s = String::from_utf8_lossy(&wire);
        assert!(s.contains("X-Empty: \r\n") || s.contains("x-empty: \r\n"));
        assert!(s.contains("X-Space: a b\r\n") || s.contains("x-space: a b\r\n"));
    }

    #[test]
    fn encode_head_rejects_crlf_header_value_at_construction() {
        // http::HeaderValue forbids CR/LF — product must not invent a bypass.
        assert!(hyper::header::HeaderValue::from_str("evil\r\nX-Injected: 1").is_err());
        assert!(hyper::header::HeaderValue::from_bytes(b"evil\r\nX-Injected: 1").is_err());
    }

    #[test]
    fn encode_head_skips_non_utf8_header_values_like_legacy() {
        let mut parts = parts_with(200, &[("X-Ok", "yes")]);
        let raw = hyper::header::HeaderValue::from_bytes(&[0xff, 0xfe]).expect("raw bytes value");
        parts.headers.insert("X-Bin", raw);
        let a = encode_proxy_response_head_format_legacy(&parts, false);
        let (b, outcome) = encode_proxy_response_head(&parts, false);
        assert_eq!(outcome, WireWriteOutcome::Close);
        assert_eq!(a, b);
        let s = String::from_utf8_lossy(&b);
        assert!(s.contains("X-Ok: yes\r\n") || s.contains("x-ok: yes\r\n"));
        assert!(!s.to_ascii_lowercase().contains("x-bin:"));
    }
}
