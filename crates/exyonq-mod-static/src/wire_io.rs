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
//! Wire connection I/O helpers (KD2.3) — module-local only.
#![cfg_attr(target_os = "linux", allow(dead_code))]

use bytes::Bytes;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Process-lifetime cache for `EXYONQ_READ_TIMEOUT_MS` (0 = unset; else millis + 1).
static HEADER_READ_TIMEOUT_MS_CACHE: AtomicU64 = AtomicU64::new(0);

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

pub fn append_bytes(left: Bytes, right: &[u8]) -> Bytes {
    if left.is_empty() {
        return Bytes::copy_from_slice(right);
    }
    if right.is_empty() {
        return left;
    }
    if left.len() + right.len() <= 512 {
        let mut merged = [0u8; 512];
        merged[..left.len()].copy_from_slice(&left);
        merged[left.len()..left.len() + right.len()].copy_from_slice(right);
        return Bytes::copy_from_slice(&merged[..left.len() + right.len()]);
    }
    let mut merged = bytes::BytesMut::with_capacity(left.len() + right.len());
    merged.extend_from_slice(&left);
    merged.extend_from_slice(right);
    merged.freeze()
}

/// Header-read / keep-alive idle timeout from `EXYONQ_READ_TIMEOUT_MS`.
///
/// Resolved once at first observation (process-lifetime). Unset → 30s;
/// invalid / non-u64 → 30s; explicit millis → that duration. Mid-process
/// env mutation is not observed — set before process start (tests may reset).
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
#[cfg(any(test, feature = "test-utils"))]
#[doc(hidden)]
pub fn reset_header_read_timeout_cache_for_tests() {
    HEADER_READ_TIMEOUT_MS_CACHE.store(0, Ordering::Relaxed);
}

/// `SO_RCVTIMEO` on a blocking socket read surfaces as `WouldBlock` / EAGAIN on Linux.
#[cfg(target_os = "linux")]
pub fn is_connection_read_timeout(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
}

#[cfg(target_os = "linux")]
pub fn blocking_tcp_read(stream: &mut std::net::TcpStream, buf: &mut [u8]) -> io::Result<usize> {
    use std::io::Read;
    loop {
        match stream.read(buf) {
            Ok(n) => return Ok(n),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

pub async fn read_tcp(stream: &mut tokio::net::TcpStream, buf: &mut [u8]) -> io::Result<usize> {
    stream.read(buf).await
}

#[cfg(target_os = "linux")]
pub fn write_response_fd(fd: i32, buf: &[u8]) -> io::Result<()> {
    crate::fd_io::write_response_fd(fd, buf)
}

pub async fn write_all_async<S: AsyncWrite + Unpin>(stream: &mut S, buf: &[u8]) -> io::Result<()> {
    stream.write_all(buf).await
}

#[cfg(target_os = "linux")]
pub async fn write_all_try_io(stream: &mut tokio::net::TcpStream, buf: &[u8]) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    use tokio::io::Interest;

    let fd = stream.as_raw_fd();
    stream
        .try_io(Interest::WRITABLE, || write_response_fd(fd, buf))
        .map_err(|_| io::Error::other("write interrupted"))?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub async fn write_all_try_io(stream: &mut tokio::net::TcpStream, buf: &[u8]) -> io::Result<()> {
    write_all_async(stream, buf).await
}

#[cfg(test)]
mod header_read_timeout_tests {
    use super::*;

    #[test]
    fn parse_header_read_timeout_ms_semantics() {
        assert_eq!(parse_header_read_timeout_ms(None), Duration::from_secs(30));
        assert_eq!(
            parse_header_read_timeout_ms(Some("")),
            Duration::from_secs(30)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("not-a-number")),
            Duration::from_secs(30)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("-1")),
            Duration::from_secs(30)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("0")),
            Duration::from_millis(0)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("100")),
            Duration::from_millis(100)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("50")),
            Duration::from_millis(50)
        );
    }
}
