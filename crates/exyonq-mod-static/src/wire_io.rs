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
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

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

pub fn header_read_timeout() -> Duration {
    std::env::var("EXYONQ_READ_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(30))
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
