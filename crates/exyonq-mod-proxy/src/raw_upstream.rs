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
//! Raw HTTP/1 GET to upstream (bypass Hyper client) for P4-class wire GETs.
//!
//! Default ON after Netcup A/B (+6% RPS). Opt-out: `EXYONQ_PROXY_RAW_UPSTREAM_GET=0`.
//! Scope: known-length (`Content-Length`) **or** finite `Transfer-Encoding: chunked`
//! decoded under [`RAW_TE_DECODE_MAX`] into Content-Length for Cap034 KA framing.
//! Chunked upstream sockets are never pool-checked-in (ADR-021).

use crate::upstream_target::UpstreamTarget;
use crate::wire_io::{find_header_end, MAX_HEADER};
use bytes::Bytes;
use hyper::header::HeaderValue;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// Hard cap on decoded TE body for raw→CL conversion (fail closed above).
const RAW_TE_DECODE_MAX: usize = 256 * 1024;

/// Parsed known-length upstream GET response (headers stripped of hop-by-hop).
pub struct RawGetResponse {
    pub status: u16,
    pub reason: Bytes,
    /// Pass-through headers as `name: value` lines without trailing CRLF (UTF-8 names).
    pub passthrough_headers: Vec<(Bytes, Bytes)>,
    pub content_length: u64,
    pub body: Bytes,
}

enum UpstreamBodyMode {
    ContentLength(u64),
    Chunked,
}

type ParsedResponseHead = (u16, Bytes, Vec<(Bytes, Bytes)>, UpstreamBodyMode);

/// Process-wide cap on idle raw-upstream sockets (all endpoints).
///
/// A hard per-endpoint ceiling (formerly 256) creates a deterministic
/// concurrency cliff when client concurrency exceeds that number (S1:
/// C256→C384 → exactly C−256 reconnects). A finite **global** budget keeps
/// FD use bounded without pinning the cliff to a single concurrency step.
const RAW_IDLE_GLOBAL_BUDGET: usize = 1024;

#[cfg(test)]
static TEST_IDLE_BUDGET_OVERRIDE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[inline]
fn idle_global_budget() -> usize {
    #[cfg(test)]
    {
        let over = TEST_IDLE_BUDGET_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
        if over > 0 {
            return over;
        }
    }
    RAW_IDLE_GLOBAL_BUDGET
}

#[inline]
fn pool_idle_total(guard: &HashMap<String, Vec<TcpStream>>) -> usize {
    guard.values().map(|v| v.len()).sum()
}

fn pool() -> &'static Mutex<HashMap<String, Vec<TcpStream>>> {
    static POOL: OnceLock<Mutex<HashMap<String, Vec<TcpStream>>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn raw_upstream_get_enabled() -> bool {
    use std::sync::atomic::{AtomicI8, Ordering};
    static CACHE: AtomicI8 = AtomicI8::new(-1);
    let cached = CACHE.load(Ordering::Relaxed);
    if cached >= 0 {
        return cached != 0;
    }
    // Default ON after Netcup A/B p4-raw-get-ab: B_vs_A ≈ +6.2% RPS (95.5k vs 89.9k)
    // with CL+KA framing intact. Opt-out: EXYONQ_PROXY_RAW_UPSTREAM_GET=0.
    let on = match std::env::var("EXYONQ_PROXY_RAW_UPSTREAM_GET")
        .ok()
        .as_deref()
    {
        Some("0") | Some("false") | Some("off") => false,
        Some("1") | Some("true") | Some("on") => true,
        None | Some(_) => true,
    };
    CACHE.store(if on { 1 } else { 0 }, Ordering::Relaxed);
    on
}

fn endpoint_key(target: &UpstreamTarget) -> Option<(String, String, u16)> {
    let auth = target.base_uri().authority()?;
    let host = auth.host().to_string();
    let port = auth.port_u16().unwrap_or(80);
    let key = format!("{host}:{port}");
    Some((key, host, port))
}

fn addr_cache() -> &'static Mutex<HashMap<String, SocketAddr>> {
    static CACHE: OnceLock<Mutex<HashMap<String, SocketAddr>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn checkout(key: &str, host: &str, port: u16) -> io::Result<TcpStream> {
    if let Some(stream) = pool()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(key)
        .and_then(|v| v.pop())
    {
        return Ok(stream);
    }
    let cached = addr_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .copied();
    let addr = if let Some(a) = cached {
        a
    } else {
        let mut it = tokio::net::lookup_host((host, port)).await?;
        let a = it
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "upstream resolve empty"))?;
        addr_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), a);
        a
    };
    let stream = TcpStream::connect(addr).await?;
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

fn checkin(key: String, stream: TcpStream) {
    let mut guard = pool().lock().unwrap_or_else(|e| e.into_inner());
    let idle_n = pool_idle_total(&guard);
    // Defined eviction when the global budget is exhausted: reject the
    // newest checkin (drop this socket). No per-endpoint hard cap.
    if idle_n >= idle_global_budget() {
        return;
    }
    guard.entry(key).or_default().push(stream);
}

/// Hot-path raw GET: CL body, or finite TE decoded to CL. Errors → Hyper fallback.
/// The whole dial, write, and read is bounded by the upstream timeout so a
/// paused peer cannot pin the client until the kernel gives up.
pub async fn get_known_length(
    target: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
) -> io::Result<RawGetResponse> {
    // timeout_ms = 0 is already elapsed. A zero tokio timeout still polls the
    // future first and a localhost upstream can answer before the timer wins.
    if target.timeout.is_zero() {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "upstream timeout"));
    }
    match tokio::time::timeout(
        target.timeout,
        get_known_length_within(target, path_and_query, x_forwarded_for),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "upstream timeout")),
    }
}

/// Write the upstream response to the client as bytes arrive.
///
/// A known Content-Length is declared up front and each read is flushed.
/// Chunked upstream is re-framed chunk-by-chunk. The first body byte is not
/// held until the upstream finishes.
pub async fn stream_get_to_client<C>(
    target: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    client: &mut C,
    client_close: bool,
) -> io::Result<u16>
where
    C: AsyncWrite + Unpin,
{
    if target.timeout.is_zero() {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "upstream timeout"));
    }
    let (key, host, port) = endpoint_key(target)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "upstream missing authority"))?;
    let mut upstream = checkout(&key, &host, port).await?;
    let result = tokio::time::timeout(
        target.timeout,
        stream_on(
            &mut upstream,
            target,
            path_and_query,
            x_forwarded_for,
            client,
            client_close,
        ),
    )
    .await;
    match result {
        Ok(Ok((status, poolable))) => {
            if poolable {
                checkin(key, upstream);
            }
            Ok(status)
        }
        Ok(Err(err)) => Err(err),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "upstream timeout")),
    }
}

async fn stream_on<U, C>(
    upstream: &mut U,
    target: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    client: &mut C,
    client_close: bool,
) -> io::Result<(u16, bool)>
where
    U: AsyncRead + AsyncWrite + Unpin,
    C: AsyncWrite + Unpin,
{
    let mut req_buf = [0u8; 512];
    let mut n = 0usize;
    push_slice(&mut req_buf, &mut n, b"GET ")?;
    push_slice(&mut req_buf, &mut n, path_and_query.as_bytes())?;
    push_slice(&mut req_buf, &mut n, b" HTTP/1.1\r\n")?;
    if let Some(host) = &target.host {
        push_slice(&mut req_buf, &mut n, b"Host: ")?;
        push_slice(&mut req_buf, &mut n, host.as_bytes())?;
        push_slice(&mut req_buf, &mut n, b"\r\n")?;
    } else if let Some(auth) = target.base_uri().authority() {
        push_slice(&mut req_buf, &mut n, b"Host: ")?;
        push_slice(&mut req_buf, &mut n, auth.as_str().as_bytes())?;
        push_slice(&mut req_buf, &mut n, b"\r\n")?;
    }
    if let Some(xff) = x_forwarded_for {
        push_slice(&mut req_buf, &mut n, b"X-Forwarded-For: ")?;
        push_slice(&mut req_buf, &mut n, xff.as_bytes())?;
        push_slice(&mut req_buf, &mut n, b"\r\n")?;
    }
    push_slice(&mut req_buf, &mut n, b"Connection: keep-alive\r\n\r\n")?;
    upstream.write_all(&req_buf[..n]).await?;

    let mut buf = [0u8; MAX_HEADER];
    let mut filled = 0usize;
    let header_end = loop {
        if filled >= MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "headers too large",
            ));
        }
        let n = upstream.read(&mut buf[filled..]).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "upstream closed before headers",
            ));
        }
        filled += n;
        if let Some(end) = find_header_end(&buf[..filled]) {
            break end;
        }
    };
    let head = &buf[..header_end];
    let after_headers = header_end + 4;
    let (status, reason, passthrough, mode) = parse_response_head(head)?;
    let mut pending = Vec::new();
    if after_headers < filled {
        pending.extend_from_slice(&buf[after_headers..filled]);
    }

    match mode {
        UpstreamBodyMode::ContentLength(content_length) => {
            let already = (content_length as usize).min(pending.len());
            if already as u64 == content_length {
                // The whole body arrived with the headers. One write keeps a
                // small response atomic for the client.
                write_response_head(
                    client,
                    status,
                    &reason,
                    &passthrough,
                    Some(content_length),
                    client_close,
                    &pending[..already],
                )
                .await?;
                return Ok((status, true));
            }
            write_response_head(
                client,
                status,
                &reason,
                &passthrough,
                Some(content_length),
                client_close,
                b"",
            )
            .await?;
            let mut got = 0u64;
            if already > 0 {
                client.write_all(&pending[..already]).await?;
                client.flush().await?;
                got += already as u64;
            }
            while got < content_length {
                let mut chunk = [0u8; 2048];
                let n = upstream.read(&mut chunk).await?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "upstream closed before body",
                    ));
                }
                let need = (content_length - got) as usize;
                let n = n.min(need);
                client.write_all(&chunk[..n]).await?;
                client.flush().await?;
                got += n as u64;
            }
            Ok((status, true))
        }
        UpstreamBodyMode::Chunked => {
            write_response_head(
                client,
                status,
                &reason,
                &passthrough,
                None,
                client_close,
                b"",
            )
            .await?;
            stream_chunked(upstream, &mut pending, client).await?;
            Ok((status, false))
        }
    }
}

async fn write_response_head<C: AsyncWrite + Unpin>(
    client: &mut C,
    status: u16,
    reason: &[u8],
    passthrough: &[(Bytes, Bytes)],
    content_length: Option<u64>,
    client_close: bool,
    body_prefix: &[u8],
) -> io::Result<()> {
    let mut wire = Vec::with_capacity(256);
    wire.extend_from_slice(b"HTTP/1.1 ");
    wire.extend_from_slice(status.to_string().as_bytes());
    if !reason.is_empty() {
        wire.push(b' ');
        wire.extend_from_slice(reason);
    }
    wire.extend_from_slice(b"\r\n");
    for (name, value) in passthrough {
        wire.extend_from_slice(name);
        wire.extend_from_slice(b": ");
        wire.extend_from_slice(value);
        wire.extend_from_slice(b"\r\n");
    }
    if let Some(len) = content_length {
        wire.extend_from_slice(b"Content-Length: ");
        wire.extend_from_slice(len.to_string().as_bytes());
        wire.extend_from_slice(b"\r\n");
    } else {
        wire.extend_from_slice(b"Transfer-Encoding: chunked\r\n");
    }
    if client_close {
        wire.extend_from_slice(b"Connection: close\r\n\r\n");
    } else {
        wire.extend_from_slice(b"Connection: keep-alive\r\n\r\n");
    }
    wire.extend_from_slice(body_prefix);
    client.write_all(&wire).await?;
    client.flush().await
}

async fn stream_chunked<U, C>(
    upstream: &mut U,
    pending: &mut Vec<u8>,
    client: &mut C,
) -> io::Result<()>
where
    U: AsyncRead + Unpin,
    C: AsyncWrite + Unpin,
{
    loop {
        let size_line = read_crlf_line(upstream, pending).await?;
        if size_line.contains(&b';') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk extensions rejected",
            ));
        }
        let size = parse_hex_usize(&size_line)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
        if size == 0 {
            let line = read_crlf_line(upstream, pending).await?;
            if !line.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "chunk trailers rejected",
                ));
            }
            client.write_all(b"0\r\n\r\n").await?;
            client.flush().await?;
            return Ok(());
        }
        while pending.len() < size + 2 {
            let mut chunk = [0u8; 4096];
            let n = upstream.read(&mut chunk).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "upstream closed mid-chunk",
                ));
            }
            pending.extend_from_slice(&chunk[..n]);
        }
        let mut frame = Vec::with_capacity(size + 16);
        frame.extend_from_slice(format!("{size:x}\r\n").as_bytes());
        frame.extend_from_slice(&pending[..size]);
        frame.extend_from_slice(b"\r\n");
        if pending.get(size) != Some(&b'\r') || pending.get(size + 1) != Some(&b'\n') {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad chunk CRLF"));
        }
        client.write_all(&frame).await?;
        client.flush().await?;
        pending.drain(..size + 2);
    }
}

async fn get_known_length_within(
    target: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
) -> io::Result<RawGetResponse> {
    let (key, host, port) = endpoint_key(target)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "upstream missing authority"))?;

    let mut stream = checkout(&key, &host, port).await?;
    match get_known_length_on(&mut stream, target, path_and_query, x_forwarded_for).await {
        Ok((resp, poolable)) => {
            if poolable {
                checkin(key, stream);
            }
            // else: drop stream — TE decode must not recycle chunked upstream sockets.
            Ok(resp)
        }
        Err(err) => Err(err),
    }
}

fn push_slice(dst: &mut [u8], off: &mut usize, src: &[u8]) -> io::Result<()> {
    let end = off
        .checked_add(src.len())
        .filter(|&e| e <= dst.len())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "request too large"))?;
    let slot = dst
        .get_mut(*off..end)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "request too large"))?;
    slot.copy_from_slice(src);
    *off = end;
    Ok(())
}

async fn get_known_length_on(
    stream: &mut TcpStream,
    target: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
) -> io::Result<(RawGetResponse, bool)> {
    // Stack request buffer: P4 `/api/` + Host + optional XFF fits well under 512 B.
    let mut req_buf = [0u8; 512];
    let mut n = 0usize;
    push_slice(&mut req_buf, &mut n, b"GET ")?;
    push_slice(&mut req_buf, &mut n, path_and_query.as_bytes())?;
    push_slice(&mut req_buf, &mut n, b" HTTP/1.1\r\n")?;
    if let Some(host) = &target.host {
        push_slice(&mut req_buf, &mut n, b"Host: ")?;
        push_slice(&mut req_buf, &mut n, host.as_bytes())?;
        push_slice(&mut req_buf, &mut n, b"\r\n")?;
    } else if let Some(auth) = target.base_uri().authority() {
        push_slice(&mut req_buf, &mut n, b"Host: ")?;
        push_slice(&mut req_buf, &mut n, auth.as_str().as_bytes())?;
        push_slice(&mut req_buf, &mut n, b"\r\n")?;
    }
    if let Some(xff) = x_forwarded_for {
        push_slice(&mut req_buf, &mut n, b"X-Forwarded-For: ")?;
        push_slice(&mut req_buf, &mut n, xff.as_bytes())?;
        push_slice(&mut req_buf, &mut n, b"\r\n")?;
    }
    push_slice(&mut req_buf, &mut n, b"Connection: keep-alive\r\n\r\n")?;
    stream.write_all(&req_buf[..n]).await?;

    let mut buf = [0u8; MAX_HEADER];
    let mut filled = 0usize;
    let header_end = loop {
        if filled >= MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "headers too large",
            ));
        }
        let n = stream.read(&mut buf[filled..]).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "upstream closed before headers",
            ));
        }
        filled += n;
        if let Some(end) = find_header_end(&buf[..filled]) {
            break end;
        }
    };

    let head = &buf[..header_end];
    let after_headers = header_end + 4;
    let (status, reason, passthrough, mode) = parse_response_head(head)?;

    let mut pending = Vec::new();
    if after_headers < filled {
        pending.extend_from_slice(&buf[after_headers..filled]);
    }

    match mode {
        UpstreamBodyMode::ContentLength(content_length) => {
            let mut body = Vec::with_capacity(content_length as usize);
            body.extend_from_slice(&pending);
            while (body.len() as u64) < content_length {
                let mut chunk = [0u8; 2048];
                let n = stream.read(&mut chunk).await?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "upstream closed before body",
                    ));
                }
                let need = (content_length as usize).saturating_sub(body.len());
                body.extend_from_slice(&chunk[..n.min(need)]);
            }
            if body.len() as u64 != content_length {
                return Err(io::Error::other("content-length mismatch"));
            }
            Ok((
                RawGetResponse {
                    status,
                    reason,
                    passthrough_headers: passthrough,
                    content_length,
                    body: Bytes::from(body),
                },
                true,
            ))
        }
        UpstreamBodyMode::Chunked => {
            let body = decode_chunked_body(stream, pending).await?;
            let content_length = body.len() as u64;
            Ok((
                RawGetResponse {
                    status,
                    reason,
                    passthrough_headers: passthrough,
                    content_length,
                    body: Bytes::from(body),
                },
                false,
            ))
        }
    }
}

/// Decode HTTP/1.1 chunked body into application bytes. Fail closed on trailers,
/// extensions, size overflow, or total > [`RAW_TE_DECODE_MAX`].
async fn decode_chunked_body<R: AsyncRead + Unpin>(
    stream: &mut R,
    mut pending: Vec<u8>,
) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let size_line = read_crlf_line(stream, &mut pending).await?;
        // Reject chunk extensions (`size;ext`).
        if size_line.contains(&b';') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk extensions rejected",
            ));
        }
        let size = parse_hex_usize(&size_line)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
        if size == 0 {
            // A trailer-free message ends with an empty line immediately after the zero chunk.
            // Any non-empty line is a trailer field: reject it and discard this upstream socket
            // rather than inventing Content-Length semantics after ignoring metadata.
            let line = read_crlf_line(stream, &mut pending).await?;
            if !line.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "chunk trailers rejected",
                ));
            }
            break;
        }
        if out.len().saturating_add(size) > RAW_TE_DECODE_MAX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunked body exceeds raw TE decode cap",
            ));
        }
        while pending.len() < size + 2 {
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "upstream closed mid-chunk",
                ));
            }
            pending.extend_from_slice(&chunk[..n]);
        }
        out.extend_from_slice(&pending[..size]);
        pending.drain(..size);
        // Expect CRLF after chunk data.
        if pending.len() < 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing chunk CRLF",
            ));
        }
        if pending[0] != b'\r' || pending[1] != b'\n' {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad chunk CRLF"));
        }
        pending.drain(..2);
    }
    Ok(out)
}

async fn read_crlf_line<R: AsyncRead + Unpin>(
    stream: &mut R,
    pending: &mut Vec<u8>,
) -> io::Result<Vec<u8>> {
    loop {
        if let Some(pos) = pending.iter().position(|&b| b == b'\n') {
            let mut line = pending.drain(..=pos).collect::<Vec<_>>();
            if !line.ends_with(b"\r\n") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "chunk line missing CRLF",
                ));
            }
            line.truncate(line.len() - 2);
            return Ok(line);
        }
        if pending.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk line too long",
            ));
        }
        let mut chunk = [0u8; 1024];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "upstream closed before chunk line",
            ));
        }
        pending.extend_from_slice(&chunk[..n]);
    }
}

fn parse_hex_usize(raw: &[u8]) -> Option<usize> {
    if raw.is_empty() || raw.len() > 8 {
        return None;
    }
    let mut n: usize = 0;
    for &b in raw {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        n = n.checked_mul(16)?.checked_add(usize::from(digit))?;
    }
    Some(n)
}

fn parse_response_head(head: &[u8]) -> io::Result<ParsedResponseHead> {
    let mut lines = head.split(|&b| b == b'\n');
    let status_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty status line"))?;
    let status_line = status_line.strip_suffix(b"\r").unwrap_or(status_line);
    let mut parts = status_line.split(|&b| b == b' ');
    let _http = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad status"))?;
    let code_raw = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad status code"))?;
    let status = parse_u16_digits(code_raw)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad status code"))?;
    let reason_raw = parts.next().unwrap_or(b"OK");
    let reason = if reason_raw == b"OK" {
        Bytes::from_static(b"OK")
    } else {
        Bytes::copy_from_slice(reason_raw)
    };

    let mut content_length: Option<u64> = None;
    let mut te = false;
    let mut passthrough = Vec::with_capacity(2);
    for line in lines {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            continue;
        };
        let name = &line[..colon];
        let mut value = &line[colon + 1..];
        while value.first().copied() == Some(b' ') || value.first().copied() == Some(b'\t') {
            value = &value[1..];
        }
        if name.eq_ignore_ascii_case(b"content-length") {
            content_length = Some(
                parse_u64_digits(value)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "cl parse"))?,
            );
        } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
            if !value.eq_ignore_ascii_case(b"chunked") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported transfer-encoding",
                ));
            }
            te = true;
        } else if name.eq_ignore_ascii_case(b"connection")
            || name.eq_ignore_ascii_case(b"keep-alive")
            || name.eq_ignore_ascii_case(b"proxy-connection")
            || name.eq_ignore_ascii_case(b"upgrade")
            || name.eq_ignore_ascii_case(b"date")
        {
            // Drop hop-by-hop + Date (rewritten by rivals too; cuts Bytes churn).
        } else if name.eq_ignore_ascii_case(b"content-type")
            && value.eq_ignore_ascii_case(b"application/octet-stream")
        {
            passthrough.push((
                Bytes::from_static(b"content-type"),
                Bytes::from_static(b"application/octet-stream"),
            ));
        } else if name.eq_ignore_ascii_case(b"content-type")
            && value.eq_ignore_ascii_case(b"text/event-stream")
        {
            passthrough.push((
                Bytes::from_static(b"content-type"),
                Bytes::from_static(b"text/event-stream"),
            ));
        } else {
            passthrough.push((Bytes::copy_from_slice(name), Bytes::copy_from_slice(value)));
        }
    }
    // CL+TE together → fail closed (smuggling).
    if te && content_length.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "content-length with transfer-encoding",
        ));
    }
    if te {
        return Ok((status, reason, passthrough, UpstreamBodyMode::Chunked));
    }
    let content_length = content_length.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "raw path requires content-length",
        )
    })?;
    Ok((
        status,
        reason,
        passthrough,
        UpstreamBodyMode::ContentLength(content_length),
    ))
}

#[inline]
fn parse_u16_digits(raw: &[u8]) -> Option<u16> {
    if raw.is_empty() || raw.len() > 5 {
        return None;
    }
    let mut n: u32 = 0;
    for &b in raw {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + u32::from(b - b'0');
    }
    u16::try_from(n).ok()
}

#[inline]
fn parse_u64_digits(raw: &[u8]) -> Option<u64> {
    if raw.is_empty() || raw.len() > 20 {
        return None;
    }
    let mut n: u64 = 0;
    for &b in raw {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add(u64::from(b - b'0'))?;
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UpstreamDescriptor, UpstreamTarget};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Barrier;

    /// Serialize tests that touch the process-global raw idle pool.
    static POOL_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn pool_bucket_len(key: &str) -> usize {
        let guard = pool().lock().unwrap_or_else(|e| e.into_inner());
        guard.get(key).map(|v| v.len()).unwrap_or(0)
    }

    fn pool_clear_key(key: &str) {
        let mut guard = pool().lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(key);
    }

    fn set_test_idle_budget(n: usize) {
        TEST_IDLE_BUDGET_OVERRIDE.store(n, Ordering::Relaxed);
    }

    fn pool_idle_sum() -> usize {
        let guard = pool().lock().unwrap_or_else(|e| e.into_inner());
        pool_idle_total(&guard)
    }

    /// Keep-alive upstream; `accepts` counts TCP accepts (new connections).
    async fn spawn_ka_upstream(accepts: Arc<AtomicUsize>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    continue;
                };
                accepts.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let mut pending = 0usize;
                    loop {
                        let header_end = loop {
                            if let Some(end) =
                                buf[..pending].windows(4).position(|w| w == b"\r\n\r\n")
                            {
                                break end;
                            }
                            if pending >= buf.len() {
                                return;
                            }
                            match stream.read(&mut buf[pending..]).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => pending += n,
                            }
                        };
                        let consumed = header_end + 4;
                        if consumed < pending {
                            buf.copy_within(consumed..pending, 0);
                            pending -= consumed;
                        } else {
                            pending = 0;
                        }
                        let body = b"ok";
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                            body.len()
                        );
                        if stream.write_all(resp.as_bytes()).await.is_err() {
                            return;
                        }
                        if stream.write_all(body).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        port
    }

    fn target_for(port: u16) -> UpstreamTarget {
        let desc = UpstreamDescriptor {
            cluster_id: 0,
            upstream_name: "s1-falsify".into(),
            target: format!("http://127.0.0.1:{port}"),
            timeout: Duration::from_secs(30),
            host: Some("127.0.0.1".into()),
            max_connect_retries: 1,
        };
        UpstreamTarget::from_descriptor(&desc).unwrap()
    }

    /// Concurrent wave: all tasks checkout before any checkin, then KA GET.
    async fn ka_hold_wave(target: &UpstreamTarget, key: &str, host: &str, port: u16, c: usize) {
        let start = Arc::new(Barrier::new(c));
        let held = Arc::new(Barrier::new(c));
        let mut handles = Vec::with_capacity(c);
        for _ in 0..c {
            let start = Arc::clone(&start);
            let held = Arc::clone(&held);
            let target = target.clone();
            let key = key.to_string();
            let host = host.to_string();
            handles.push(tokio::spawn(async move {
                start.wait().await;
                let mut stream = checkout(&key, &host, port).await.expect("checkout");
                held.wait().await;
                let (resp, poolable) = get_known_length_on(&mut stream, &target, "/", None)
                    .await
                    .expect("ka get");
                assert_eq!(resp.status, 200);
                assert!(poolable, "CL keep-alive must be poolable");
                checkin(key, stream);
            }));
        }
        for h in handles {
            h.await.expect("join");
        }
    }

    async fn mint_idle_pair(listener: &TcpListener) -> (TcpStream, TcpStream) {
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (client, server)
    }

    #[test]
    fn parse_simple_200() {
        let head = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Type: text/plain\r\nConnection: keep-alive\r\n";
        let (status, reason, headers, mode) = parse_response_head(head).unwrap();
        assert_eq!(status, 200);
        assert_eq!(&reason[..], b"OK");
        assert!(matches!(mode, UpstreamBodyMode::ContentLength(3)));
        assert_eq!(headers.len(), 1);
        assert_eq!(&headers[0].0[..], b"Content-Type");
    }

    #[test]
    fn parse_te_chunked() {
        let head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/event-stream\r\n\r\n";
        let (status, _, headers, mode) = parse_response_head(head).unwrap();
        assert_eq!(status, 200);
        assert!(matches!(mode, UpstreamBodyMode::Chunked));
        assert_eq!(headers.len(), 1);
        assert_eq!(&headers[0].1[..], b"text/event-stream");
    }

    #[test]
    fn rejects_cl_plus_te() {
        let head = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert!(parse_response_head(head).is_err());
    }

    #[test]
    fn parse_hex_chunk_size() {
        assert_eq!(parse_hex_usize(b"f"), Some(15));
        assert_eq!(parse_hex_usize(b"10"), Some(16));
        assert_eq!(parse_hex_usize(b"0"), Some(0));
        assert!(parse_hex_usize(b"gg").is_none());
    }

    #[tokio::test]
    async fn decodes_chunked_body_with_empty_trailer_section() {
        let (mut reader, _writer) = tokio::io::duplex(64);
        let body = decode_chunked_body(&mut reader, b"3\r\nabc\r\n0\r\n\r\n".to_vec())
            .await
            .expect("valid trailer-free chunked body");
        assert_eq!(body, b"abc");
    }

    #[tokio::test]
    async fn rejects_chunked_trailer_fields() {
        let (mut reader, _writer) = tokio::io::duplex(64);
        let err = decode_chunked_body(
            &mut reader,
            b"3\r\nabc\r\n0\r\nDigest: sha-256=value\r\n\r\n".to_vec(),
        )
        .await
        .expect_err("trailers must not be silently discarded");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "chunk trailers rejected");
    }

    #[tokio::test]
    async fn rejects_missing_terminal_empty_line() {
        let (mut reader, writer) = tokio::io::duplex(64);
        drop(writer);
        let err = decode_chunked_body(&mut reader, b"0\r\n".to_vec())
            .await
            .expect_err("zero chunk without terminal CRLF is incomplete");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn rejects_lf_only_chunk_framing() {
        let (mut reader, _writer) = tokio::io::duplex(64);
        let err = decode_chunked_body(&mut reader, b"1\na\r\n0\r\n\r\n".to_vec())
            .await
            .expect_err("chunk size lines require CRLF");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "chunk line missing CRLF");
    }

    #[test]
    fn default_on_when_unset_semantics() {
        let parse = |v: Option<&str>| match v {
            Some("0") | Some("false") | Some("off") => false,
            Some("1") | Some("true") | Some("on") => true,
            None | Some(_) => true,
        };
        assert!(parse(None));
        assert!(parse(Some("1")));
        assert!(!parse(Some("0")));
    }

    /// S1: one endpoint may retain >256 idle while under the global budget.
    /// Mutex serializes pool mutation across async test steps (not production hot path).
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn s1_per_endpoint_may_exceed_256_under_global_budget() {
        let _g = POOL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_test_idle_budget(0);
        let key = "s1-unit-no-per-ep-cap";
        pool_clear_key(key);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut peers = Vec::with_capacity(257);
        for i in 0..257u32 {
            let (client, server) = mint_idle_pair(&listener).await;
            peers.push(server);
            checkin(key.to_string(), client);
            assert_eq!(pool_bucket_len(key), (i + 1) as usize);
        }
        assert_eq!(pool_bucket_len(key), 257);
        pool_clear_key(key);
        drop(peers);
    }

    /// S1: when the global budget is exhausted, the newest checkin is dropped.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn s1_global_budget_rejects_newest_when_full() {
        let _g = POOL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_test_idle_budget(4);
        let key_a = "s1-budget-a";
        let key_b = "s1-budget-b";
        pool_clear_key(key_a);
        pool_clear_key(key_b);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut peers = Vec::new();
        for key in [key_a, key_a, key_b, key_b] {
            let (client, server) = mint_idle_pair(&listener).await;
            peers.push(server);
            checkin(key.to_string(), client);
        }
        assert_eq!(pool_idle_sum(), 4);
        let (client, server) = mint_idle_pair(&listener).await;
        peers.push(server);
        checkin(key_a.to_string(), client);
        assert_eq!(pool_idle_sum(), 4);
        assert_eq!(pool_bucket_len(key_a), 2);
        pool_clear_key(key_a);
        pool_clear_key(key_b);
        drop(peers);
        set_test_idle_budget(0);
    }

    /// S1 repair: hold-wave C384 wave2 must not open new upstream accepts.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn s1_hold_wave_c384_no_reconnect_under_global_budget() {
        let _g = POOL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_test_idle_budget(0);

        async fn run_concurrency(c: usize) -> (usize, usize) {
            let accepts = Arc::new(AtomicUsize::new(0));
            let listen_port = spawn_ka_upstream(Arc::clone(&accepts)).await;
            let target = target_for(listen_port);
            let key = format!("127.0.0.1:{listen_port}");
            let host = "127.0.0.1";
            pool_clear_key(&key);

            ka_hold_wave(&target, &key, host, listen_port, c).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(pool_bucket_len(&key), c, "wave1 idle depth C={c}");
            let accepts_after_wave1 = accepts.load(Ordering::SeqCst);

            let before_wave2 = accepts.load(Ordering::SeqCst);
            ka_hold_wave(&target, &key, host, listen_port, c).await;
            let accepts_wave2 = accepts.load(Ordering::SeqCst) - before_wave2;
            pool_clear_key(&key);
            (accepts_after_wave1, accepts_wave2)
        }

        let (a256_w1, a256_w2) = run_concurrency(256).await;
        let (a384_w1, a384_w2) = run_concurrency(384).await;
        assert_eq!(a256_w1, 256);
        assert_eq!(a256_w2, 0, "C256 wave2 must reuse");
        assert_eq!(a384_w1, 384);
        assert_eq!(a384_w2, 0, "C384 wave2 must reuse under global budget");
        eprintln!(
            "S1 repair accepts: C256 w1={a256_w1} w2={a256_w2} | C384 w1={a384_w1} w2={a384_w2}"
        );
    }
}
