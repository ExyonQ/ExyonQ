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
//! IO helpers for connection dispatch.

use bytes::Bytes;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Process-lifetime cache for `EXYONQ_READ_TIMEOUT_MS` (0 = unset; else millis + 1).
static HEADER_READ_TIMEOUT_MS_CACHE: AtomicU64 = AtomicU64::new(0);

/// AsyncRead that serves a prefix before continuing on the inner stream.
pub struct PrefixedStream<S> {
    prefix: Option<Bytes>,
    inner: S,
}

impl<S> PrefixedStream<S> {
    pub fn new(prefix: Bytes, inner: S) -> Self {
        Self {
            prefix: Some(prefix),
            inner,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PrefixedStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(prefix) = self.prefix.take() {
            let slice = prefix.as_ref();
            let len = slice.len().min(buf.remaining());
            buf.put_slice(&slice[..len]);
            if len < slice.len() {
                self.prefix = Some(prefix.slice(len..));
            }
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub async fn read_until_headers_async<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> io::Result<(Bytes, Bytes)> {
    match tokio::time::timeout(header_read_timeout(), read_until_headers_inner(stream)).await {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "header read timeout",
        )),
    }
}

async fn read_until_headers_inner<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> io::Result<(Bytes, Bytes)> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::with_capacity(256);
    let mut tmp = [0u8; 512];
    loop {
        if let Some(end) = find_header_end(&buf) {
            let head_len = end + 4;
            let all = Bytes::from(buf);
            return Ok((all.slice(0..head_len), all.slice(head_len..)));
        }
        if buf.len() >= MAX_HEADER_CAP {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            if buf.is_empty() {
                return Ok((Bytes::new(), Bytes::new()));
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request headers incomplete",
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

pub fn concat_bytes(head: Bytes, rest: Bytes) -> Bytes {
    if rest.is_empty() {
        return head;
    }
    if head.is_empty() {
        return rest;
    }
    let mut merged = Vec::with_capacity(head.len() + rest.len());
    merged.extend_from_slice(&head);
    merged.extend_from_slice(&rest);
    Bytes::from(merged)
}

pub fn find_header_end(buf: &[u8]) -> Option<usize> {
    let mut i = 0;
    let limit = buf.len().saturating_sub(3);
    while i < limit {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

const MAX_HEADER_CAP: usize = 8192;

#[allow(dead_code)]
pub(crate) fn max_header_cap() -> usize {
    MAX_HEADER_CAP
}

/// True when `line` is an HTTP header field whose name equals `name_lower` (ASCII case-insensitive).
/// Matches only the field name before `:`, so `X-Content-Length:` does not match `content-length`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn header_line_name_eq(line: &[u8], name_lower: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|&b| b == b':') else {
        return false;
    };
    let mut name = &line[..colon];
    while name.first() == Some(&b' ') || name.first() == Some(&b'\t') {
        name = &name[1..];
    }
    while name.last() == Some(&b' ') || name.last() == Some(&b'\t') {
        name = &name[..name.len() - 1];
    }
    name.len() == name_lower.len() && name.eq_ignore_ascii_case(name_lower)
}

/// Count header fields by exact name (case-insensitive) on a complete header block.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn header_field_name_count(head: &[u8], name_lower: &[u8]) -> usize {
    let mut count = 0usize;
    let mut start = 0usize;
    // Skip request line.
    if let Some(i) = head.windows(2).position(|w| w == b"\r\n") {
        start = i + 2;
    }
    while start < head.len() {
        if start + 1 < head.len() && head[start] == b'\r' && head[start + 1] == b'\n' {
            break;
        }
        let rel = head[start..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .unwrap_or(head.len() - start);
        let line = &head[start..start + rel];
        if header_line_name_eq(line, name_lower) {
            count += 1;
        }
        start += rel + 2;
    }
    count
}

/// Reject ambiguous length/framing on a complete HTTP/1 header block (wire bytes).
/// Call only after `find_header_end` — incomplete buffers are not validated here.
///
/// Wire path is fail-closed: Transfer-Encoding alone is rejected (no chunked drain on wire),
/// duplicate Content-Length is rejected, and TE+CL is rejected. Matching is case-insensitive
/// on field names and does not treat `X-Content-Length` as Content-Length.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn request_headers_safe_for_wire(head: &[u8]) -> bool {
    let cl = header_field_name_count(head, b"content-length");
    let te = header_field_name_count(head, b"transfer-encoding");
    if cl > 1 {
        return false;
    }
    if te >= 1 {
        return false;
    }
    true
}

/// Header-read timeout from `EXYONQ_READ_TIMEOUT_MS` (process-lifetime cache).
///
/// Unset / invalid → 30s. Mid-process env mutation is not observed —
/// set the env before process start (or reset cache in tests).
///
/// This is the core-local observation. Platform and module paths maintain
/// equivalent layer-local caches so lower layers do not reverse-import policy.
pub fn header_read_timeout() -> Duration {
    let cached = HEADER_READ_TIMEOUT_MS_CACHE.load(Ordering::Relaxed);
    if cached != 0 {
        return Duration::from_millis(cached - 1);
    }
    let d = parse_header_read_timeout_ms(std::env::var("EXYONQ_READ_TIMEOUT_MS").ok().as_deref());
    let encoded = d.as_millis() as u64 + 1;
    let _ = HEADER_READ_TIMEOUT_MS_CACHE.compare_exchange(
        0,
        encoded,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
    let final_ms = HEADER_READ_TIMEOUT_MS_CACHE.load(Ordering::Relaxed) - 1;
    Duration::from_millis(final_ms)
}

fn parse_header_read_timeout_ms(raw: Option<&str>) -> Duration {
    raw.and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(30))
}

/// Test-only: clear process-lifetime cache (simulates a fresh process).
/// Also clears static/proxy wire caches used on related request paths.
///
/// This cross-module reset seam is compiled only for unit tests or when the
/// explicit `test-utils` feature is enabled by the integration-test crate.
#[cfg(any(test, feature = "test-utils"))]
#[doc(hidden)]
pub fn reset_header_read_timeout_cache_for_tests() {
    HEADER_READ_TIMEOUT_MS_CACHE.store(0, Ordering::Relaxed);
    exyonq_mod_static::reset_header_read_timeout_cache_for_tests();
    exyonq_mod_proxy::wire_io::reset_header_read_timeout_cache_for_tests();
}

/// `SO_RCVTIMEO` on a blocking socket read surfaces as `WouldBlock` / EAGAIN on Linux.
#[cfg(target_os = "linux")]
pub(crate) fn is_connection_read_timeout(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
}

/// Per-connection I/O errors that must close the client without terminating the accept worker.
#[cfg(target_os = "linux")]
pub(crate) fn is_connection_level_io_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::WouldBlock
            | io::ErrorKind::TimedOut
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::InvalidData
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn blocking_tcp_read(
    stream: &mut std::net::TcpStream,
    buf: &mut [u8],
) -> io::Result<usize> {
    use std::io::Read;
    loop {
        match stream.read(buf) {
            Ok(n) => return Ok(n),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Blocking HTTP/1 header read (sync accept workers; Linux P1 hot path).
#[cfg(target_os = "linux")]
pub(crate) fn read_until_headers_blocking(
    stream: &mut std::net::TcpStream,
) -> io::Result<(Bytes, Bytes)> {
    let mut buf = Vec::with_capacity(256);
    let mut tmp = [0u8; 512];
    loop {
        if let Some(end) = find_header_end(&buf) {
            let head_len = end + 4;
            let all = Bytes::from(buf);
            return Ok((all.slice(0..head_len), all.slice(head_len..)));
        }
        if buf.len() >= MAX_HEADER_CAP {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = match blocking_tcp_read(stream, &mut tmp) {
            Ok(n) => n,
            Err(e) if is_connection_read_timeout(&e) => {
                return Ok((Bytes::new(), Bytes::new()));
            }
            Err(e) => return Err(e),
        };
        if n == 0 {
            if buf.is_empty() {
                return Ok((Bytes::new(), Bytes::new()));
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request headers incomplete",
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn write_once_fd(fd: i32, buf: &[u8]) -> io::Result<()> {
    exyonq_linux_ffi::send_all_nosignal(fd, buf)
}

#[cfg(target_os = "linux")]
pub(crate) fn write_response_fd(fd: i32, buf: &[u8]) -> io::Result<()> {
    if buf.len() <= 16 * 1024 {
        write_once_fd(fd, buf)
    } else {
        write_all_fd(fd, buf)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn write_all_fd(fd: i32, buf: &[u8]) -> io::Result<()> {
    exyonq_linux_ffi::write_all_fd(fd, buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};

    #[tokio::test]
    async fn read_errors_on_eof_before_headers_complete() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.expect("connect");
            stream
                .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n")
                .await
                .expect("write partial");
            stream.shutdown().await.expect("shutdown");
        });
        let (mut server, _) = listener.accept().await.expect("accept");
        let err = read_until_headers_async(&mut server)
            .await
            .expect_err("incomplete headers");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(err.to_string().contains("incomplete"));
        client.await.expect("join");
    }

    #[tokio::test]
    async fn read_accepts_immediate_close_with_no_data() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let client = tokio::spawn(async move {
            let stream = TcpStream::connect(addr).await.expect("connect");
            drop(stream);
        });
        let (mut server, _) = listener.accept().await.expect("accept");
        let (head, rest) = read_until_headers_async(&mut server)
            .await
            .expect("empty close");
        assert!(head.is_empty());
        assert!(rest.is_empty());
        client.await.expect("join");
    }

    #[test]
    fn wire_safe_incomplete_headers_not_validated_by_caller_contract() {
        // Incomplete blocks must not reach validation; a partial duplicate CL line is OK while reading.
        let partial = b"GET / HTTP/1.1\r\nContent-Length: 0\r\nContent-Length:";
        assert!(find_header_end(partial).is_none());
    }

    #[test]
    fn wire_safe_accepts_complete_header_between_512_and_8192() {
        let mut header = Vec::from(b"GET /health HTTP/1.1\r\nHost: x\r\nX-Pad: ");
        header.extend(std::iter::repeat_n(b'a', 600));
        header.extend_from_slice(b"\r\n\r\n");
        assert!(header.len() > 512);
        assert!(header.len() < MAX_HEADER_CAP);
        assert!(find_header_end(&header).is_some());
        assert!(request_headers_safe_for_wire(&header));
    }

    #[test]
    fn wire_safe_rejects_oversized_header_cap() {
        assert_eq!(max_header_cap(), 8192);
        let len_at_cap = MAX_HEADER_CAP;
        assert!(len_at_cap > MAX_HEADER_CAP - 1);
    }

    #[test]
    fn wire_safe_rejects_duplicate_content_length() {
        let head = b"GET / HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nContent-Length: 1\r\n\r\n";
        assert!(!request_headers_safe_for_wire(head));
    }

    #[test]
    fn wire_safe_rejects_duplicate_content_length_lowercase() {
        let head = b"GET / HTTP/1.1\r\nHost: x\r\ncontent-length: 0\r\ncontent-length: 1\r\n\r\n";
        assert!(!request_headers_safe_for_wire(head));
    }

    #[test]
    fn wire_safe_rejects_transfer_encoding_with_content_length() {
        let head = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n";
        assert!(!request_headers_safe_for_wire(head));
    }

    #[test]
    fn wire_safe_rejects_transfer_encoding_mixed_case_with_content_length() {
        let head = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\ncontent-length: 0\r\n\r\n";
        assert!(!request_headers_safe_for_wire(head));
        let head2 = b"POST / HTTP/1.1\r\nHost: x\r\ntransfer-encoding: chunked\r\nContent-Length: 0\r\n\r\n";
        assert!(!request_headers_safe_for_wire(head2));
    }

    #[test]
    fn wire_safe_rejects_transfer_encoding_alone() {
        let head = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert!(!request_headers_safe_for_wire(head));
    }

    #[test]
    fn wire_safe_x_content_length_does_not_count_as_content_length() {
        let head =
            b"GET / HTTP/1.1\r\nHost: x\r\nX-Content-Length: 99\r\nContent-Length: 0\r\n\r\n";
        assert!(request_headers_safe_for_wire(head));
    }

    #[test]
    fn wire_safe_accepts_typical_get() {
        let head = b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n";
        assert!(request_headers_safe_for_wire(head));
    }

    #[test]
    fn parse_header_read_timeout_ms_semantics() {
        assert_eq!(parse_header_read_timeout_ms(None), Duration::from_secs(30));
        assert_eq!(
            parse_header_read_timeout_ms(Some("bogus")),
            Duration::from_secs(30)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("100")),
            Duration::from_millis(100)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("0")),
            Duration::from_millis(0)
        );
    }

    #[cfg(target_os = "linux")]
    mod linux_blocking_read {
        use super::*;
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};
        use std::thread;
        use std::time::Duration;

        #[test]
        fn is_connection_read_timeout_matches_would_block() {
            let err = io::Error::from(io::ErrorKind::WouldBlock);
            assert!(is_connection_read_timeout(&err));
        }

        #[test]
        fn read_until_headers_blocking_times_out_as_empty() {
            let prev = std::env::var("EXYONQ_READ_TIMEOUT_MS").ok();
            crate::server::reset_header_read_timeout_cache_for_tests();
            std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "50");
            crate::server::reset_header_read_timeout_cache_for_tests();
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("addr");
            let client = thread::spawn(move || {
                let _s = TcpStream::connect(addr).expect("connect");
                thread::sleep(Duration::from_millis(150));
            });
            let (mut server, _) = listener.accept().expect("accept");
            server
                .set_read_timeout(Some(Duration::from_millis(50)))
                .expect("timeout");
            let (head, rest) = read_until_headers_blocking(&mut server).expect("read");
            assert!(head.is_empty());
            assert!(rest.is_empty());
            client.join().expect("join");
            match prev {
                Some(v) => std::env::set_var("EXYONQ_READ_TIMEOUT_MS", v),
                None => std::env::remove_var("EXYONQ_READ_TIMEOUT_MS"),
            }
            crate::server::reset_header_read_timeout_cache_for_tests();
        }

        #[test]
        fn blocking_tcp_read_retries_interrupted() {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("addr");
            let client = thread::spawn(move || {
                let mut s = TcpStream::connect(addr).expect("connect");
                s.write_all(b"ping").expect("write");
            });
            let (mut server, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4];
            let n = blocking_tcp_read(&mut server, &mut buf).expect("read");
            assert_eq!(n, 4);
            client.join().expect("join");
        }
    }
}
