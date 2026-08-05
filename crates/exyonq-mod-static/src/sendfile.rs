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
//! Linux zero-copy static body delivery via sendfile(2).
#![cfg_attr(target_os = "linux", allow(dead_code))]

use super::wire;
use bytes::Bytes;
use std::fs::File;
use std::io;
use std::sync::Arc;

/// sendfile only wins for larger payloads; small bench files (P1/P7) use wired memory responses.
pub const SENDFILE_MIN_BYTES: usize = 65536;

#[derive(Clone, Debug)]
pub struct SendfileAsset {
    pub file: Arc<File>,
    pub header: Arc<Bytes>,
    pub body_len: usize,
}

impl SendfileAsset {
    pub fn open(path: &std::path::Path, body_len: usize) -> io::Result<Self> {
        Ok(Self {
            file: Arc::new(File::open(path)?),
            header: wire::bench_header_keep(body_len),
            body_len,
        })
    }
}

/// Non-blocking sendfile body progress (ADR-025 Phase 2 PR #1).
/// KD2: exported for core epoll FSM until KD2.3 moves epoll_sendfile into this crate.
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub enum NbSendfileOutcome {
    /// Sent at least one byte; caller may loop (EPOLLET drain) while socket accepts writes.
    Progress,
    /// `EAGAIN`/`WouldBlock` — offset and remaining are persisted; park until `EPOLLOUT`.
    Parked,
    /// All body bytes sent for this response.
    Complete,
}

#[cfg(target_os = "linux")]
pub(crate) fn sendfile_chunk_size() -> usize {
    static CHUNK: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CHUNK.get_or_init(|| {
        std::env::var("EXYONQ_SENDFILE_CHUNK")
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|&n| n >= 65536)
            .unwrap_or(256 * 1024)
    })
}

/// One non-blocking `sendfile64` step. Loops `EINTR` in-place; returns `Parked` on `EAGAIN`.
/// KD2: exported for core epoll FSM until KD2.3.
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn sendfile_body_nb(
    out_fd: i32,
    in_fd: i32,
    file_offset: &mut i64,
    body_remaining: &mut usize,
) -> Result<NbSendfileOutcome, io::Error> {
    if *body_remaining == 0 {
        return Ok(NbSendfileOutcome::Complete);
    }
    let chunk_size = sendfile_chunk_size();
    loop {
        let chunk = (*body_remaining).min(chunk_size);
        let sent = unsafe { libc::sendfile64(out_fd, in_fd, file_offset, chunk) };
        if sent < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if err.kind() == io::ErrorKind::WouldBlock {
                return Ok(NbSendfileOutcome::Parked);
            }
            return Err(err);
        }
        if sent == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "sendfile returned 0",
            ));
        }
        *body_remaining -= sent as usize;
        if *body_remaining == 0 {
            return Ok(NbSendfileOutcome::Complete);
        }
        return Ok(NbSendfileOutcome::Progress);
    }
}

#[cfg(target_os = "linux")]
pub fn write_bench_head_only_fd(out_fd: i32, asset: &SendfileAsset) -> io::Result<()> {
    crate::fd_io::write_response_fd(out_fd, asset.header.as_ref())
}

#[cfg(target_os = "linux")]
pub async fn write_bench_head_only_try_io(
    stream: &mut tokio::net::TcpStream,
    asset: &SendfileAsset,
) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    use tokio::io::Interest;

    let out_fd = stream.as_raw_fd();
    stream.try_io(Interest::WRITABLE, || {
        write_bench_head_only_fd(out_fd, asset)
    })
}

#[cfg(target_os = "linux")]
pub fn write_bench_response_fd(out_fd: i32, asset: &SendfileAsset) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    write_bench_sendfile_fd(
        out_fd,
        asset.file.as_raw_fd(),
        asset.header.as_ref(),
        asset.body_len,
    )
}

#[cfg(target_os = "linux")]
pub async fn write_bench_response_try_io(
    stream: &mut tokio::net::TcpStream,
    asset: &SendfileAsset,
) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    use tokio::io::Interest;

    let out_fd = stream.as_raw_fd();
    let in_fd = asset.file.as_raw_fd();
    let header = asset.header.as_ref();
    let chunk_size = sendfile_chunk_size();
    let mut remaining = asset.body_len;
    let mut offset: i64 = 0;

    stream.try_io(Interest::WRITABLE, || {
        crate::fd_io::write_response_fd(out_fd, header)
    })?;

    while remaining > 0 {
        let chunk = remaining.min(chunk_size);
        let result = stream.try_io(Interest::WRITABLE, || {
            let sent = unsafe { libc::sendfile64(out_fd, in_fd, &mut offset, chunk) };
            if sent < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock {
                    return Err(err);
                }
                return Err(err);
            }
            if sent == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "sendfile returned 0",
                ));
            }
            remaining -= sent as usize;
            Ok(())
        });
        match result {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub async fn write_bench_response(
    stream: tokio::net::TcpStream,
    asset: &SendfileAsset,
) -> io::Result<tokio::net::TcpStream> {
    let mut stream = stream;
    write_bench_response_try_io(&mut stream, asset).await?;
    Ok(stream)
}

#[cfg(not(target_os = "linux"))]
pub async fn write_bench_response(
    _stream: tokio::net::TcpStream,
    _asset: &SendfileAsset,
) -> io::Result<tokio::net::TcpStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "sendfile is only supported on Linux",
    ))
}

#[cfg(target_os = "linux")]
fn write_response(out_fd: i32, in_fd: i32, header: &[u8], body_len: usize) -> io::Result<()> {
    write_bench_sendfile_fd(out_fd, in_fd, header, body_len)
}

/// Header + sendfile body on raw fds (P2/P3 blocking hot path).
/// KD2: exported for core static_conn until KD2.3.
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn write_bench_sendfile_fd(
    out_fd: i32,
    in_fd: i32,
    header: &[u8],
    body_len: usize,
) -> io::Result<()> {
    crate::fd_io::write_response_fd(out_fd, header)?;
    sendfile_body_blocking(out_fd, in_fd, body_len)
}

#[cfg(target_os = "linux")]
fn sendfile_body_blocking(out_fd: i32, in_fd: i32, mut remaining: usize) -> io::Result<()> {
    let chunk_size = sendfile_chunk_size();
    let mut offset: i64 = 0;
    while remaining > 0 {
        let chunk = remaining.min(chunk_size);
        let sent = unsafe { libc::sendfile64(out_fd, in_fd, &mut offset, chunk) };
        if sent < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if sent == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "sendfile returned 0",
            ));
        }
        remaining -= sent as usize;
    }
    Ok(())
}
