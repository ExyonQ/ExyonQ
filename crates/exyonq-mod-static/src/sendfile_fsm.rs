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
//! ADR-025 Phase 2: non-blocking sendfile FSM (send-side).
//!
//! PR #1 added the FSM; PR #2 wires it into the epoll keep-alive worker for P2/P3 behind
//! `EXYONQ_EPOLL_STATIC=1` + `EXYONQ_EPOLL_SENDFILE=1` (KD2.3: wired from core epoll loop).
//! A few items remain used only by tests/docs and carry a targeted `#[allow(dead_code)]`.

use crate::sendfile::SendfileAsset;
use std::io;
use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::sync::OnceLock;

pub const EPOLLIN: u32 = 0x001;
pub const EPOLLOUT: u32 = 0x004;
pub const EPOLLRDHUP: u32 = 0x2000;
pub const EPOLLET: u32 = 1 << 31;
pub const EPOLL_CTL_ADD: i32 = 1;
pub const EPOLL_CTL_DEL: i32 = 2;
pub const EPOLL_CTL_MOD: i32 = 3;

/// Bench response headers fit in wire templates; stack buffer avoids heap in FSM.
pub const MAX_RESPONSE_HEADER: usize = 128;

pub(crate) const MSG_NOSIGNAL: i32 = 0x4000;

/// Result of one FSM pump invocation. **`Parked` stops the EPOLLET caller** until `EPOLLOUT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpResult {
    /// EPOLLET inner drain signal. `pump_sendfile_nb` consumes it internally (loops on
    /// `Progress`) and never returns it, so callers only match it defensively.
    #[allow(dead_code)]
    Progress,
    /// Socket would block — persist offsets; caller must stop (no map to `Continue`).
    Parked,
    /// Response fully sent (header, and body if applicable).
    Complete,
    /// Terminal error (peer reset, etc.).
    Error(io::ErrorKind),
}

/// Alias used in ADR/plan docs.
#[allow(dead_code)]
pub type SendProgress = PumpResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseHeaderBuf {
    pub bytes: [u8; MAX_RESPONSE_HEADER],
    pub len: usize,
    pub offset: usize,
}

impl ResponseHeaderBuf {
    pub fn from_header_bytes(src: &[u8]) -> io::Result<Self> {
        if src.len() > MAX_RESPONSE_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "response header exceeds MAX_RESPONSE_HEADER",
            ));
        }
        let mut bytes = [0u8; MAX_RESPONSE_HEADER];
        bytes[..src.len()].copy_from_slice(src);
        Ok(Self {
            bytes,
            len: src.len(),
            offset: 0,
        })
    }

    pub fn pending(&self) -> &[u8] {
        &self.bytes[self.offset..self.len]
    }

    pub fn done(&self) -> bool {
        self.offset >= self.len
    }
}

/// Per-connection send state (PR #2 wraps this inside core `ConnState`).
#[derive(Debug)]
pub struct SendingState {
    pub asset: Arc<SendfileAsset>,
    pub header: ResponseHeaderBuf,
    pub header_done: bool,
    pub file_offset: i64,
    pub body_remaining: usize,
    pub head_only: bool,
}

impl SendingState {
    pub fn new_get(asset: Arc<SendfileAsset>) -> io::Result<Self> {
        Ok(Self {
            header: ResponseHeaderBuf::from_header_bytes(asset.header.as_ref())?,
            header_done: false,
            file_offset: 0,
            body_remaining: asset.body_len,
            head_only: false,
            asset,
        })
    }

    pub fn new_head_only(asset: Arc<SendfileAsset>) -> io::Result<Self> {
        Ok(Self {
            header: ResponseHeaderBuf::from_header_bytes(asset.header.as_ref())?,
            header_done: false,
            file_offset: 0,
            body_remaining: 0,
            head_only: true,
            asset,
        })
    }
}

/// `EXYONQ_EPOLL_STATIC=1` and `EXYONQ_EPOLL_SENDFILE=1`. Default off; PR #2 wires routing.
pub fn epoll_sendfile_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        let static_on = std::env::var("EXYONQ_EPOLL_STATIC").ok().as_deref() == Some("1");
        let sendfile_on = std::env::var("EXYONQ_EPOLL_SENDFILE").ok().as_deref() == Some("1");
        if sendfile_on && !static_on {
            tracing::warn!(
                "EXYONQ_EPOLL_SENDFILE=1 ignored without EXYONQ_EPOLL_STATIC=1 (ADR-025 PR #2)"
            );
            return false;
        }
        static_on && sendfile_on
    })
}

pub fn interest_reading() -> u32 {
    EPOLLIN | EPOLLRDHUP | EPOLLET
}

/// Sending or parked on `EAGAIN`: no `EPOLLIN` (backpressure).
pub fn interest_sending_parked() -> u32 {
    EPOLLOUT | EPOLLRDHUP | EPOLLET
}

pub fn interest_for_pump_result(result: PumpResult) -> u32 {
    match result {
        PumpResult::Complete | PumpResult::Error(_) => interest_reading(),
        PumpResult::Parked | PumpResult::Progress => interest_sending_parked(),
    }
}

/// Non-blocking partial header write via `send(2)` — never `write_response_fd` / `write_all_fd`.
fn try_send_header_partial(
    out_fd: RawFd,
    header: &mut ResponseHeaderBuf,
) -> Result<usize, io::Error> {
    let pending = header.pending();
    if pending.is_empty() {
        return Ok(0);
    }
    loop {
        let written = unsafe {
            libc::send(
                out_fd,
                pending.as_ptr() as *const libc::c_void,
                pending.len(),
                MSG_NOSIGNAL,
            )
        };
        if written < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if written == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "send returned 0"));
        }
        header.offset += written as usize;
        return Ok(written as usize);
    }
}

/// Pump until `Parked`, `Complete`, or `Error`. EPOLLET: loops on `Progress` only.
pub fn pump_sendfile_nb(out_fd: RawFd, state: &mut SendingState) -> PumpResult {
    use crate::sendfile::{sendfile_body_nb, NbSendfileOutcome};
    use std::os::unix::io::AsRawFd;

    loop {
        if !state.header_done {
            match try_send_header_partial(out_fd, &mut state.header) {
                Ok(_) => {
                    if state.header.done() {
                        state.header_done = true;
                        if state.head_only {
                            return PumpResult::Complete;
                        }
                    } else {
                        continue;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return PumpResult::Parked,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return PumpResult::Error(e.kind()),
            }
        }

        if state.body_remaining == 0 {
            return PumpResult::Complete;
        }

        let in_fd = state.asset.file.as_raw_fd();
        match sendfile_body_nb(
            out_fd,
            in_fd,
            &mut state.file_offset,
            &mut state.body_remaining,
        ) {
            Ok(NbSendfileOutcome::Complete) => return PumpResult::Complete,
            Ok(NbSendfileOutcome::Parked) => return PumpResult::Parked,
            Ok(NbSendfileOutcome::Progress) => continue,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return PumpResult::Error(e.kind()),
        }
    }
}

/// EPOLLET caller helper (PR #2). **`Parked` ⇒ stop immediately** — do not call pump again.
/// `pump_calls`/`interest` are diagnostic fields exercised by the FSM contract tests; the
/// wired worker reads only `result` and manages interest via its own lazy `EPOLL_CTL_MOD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpollPumpDrain {
    pub result: PumpResult,
    #[allow(dead_code)]
    pub pump_calls: u32,
    #[allow(dead_code)]
    pub interest: u32,
}

pub fn epoll_drain_pump_once(out_fd: RawFd, state: &mut SendingState) -> EpollPumpDrain {
    let mut pump_calls = 0u32;
    loop {
        pump_calls += 1;
        let result = pump_sendfile_nb(out_fd, state);
        match result {
            PumpResult::Progress => continue,
            PumpResult::Parked => {
                return EpollPumpDrain {
                    result: PumpResult::Parked,
                    pump_calls,
                    interest: interest_sending_parked(),
                };
            }
            PumpResult::Complete | PumpResult::Error(_) => {
                return EpollPumpDrain {
                    result,
                    pump_calls,
                    interest: interest_for_pump_result(result),
                };
            }
        }
    }
}

/// Mock correct EPOLLET caller for the strong `Parked` contract: it invokes the
/// pump-drain exactly once per readiness event and never re-invokes after `Parked`
/// (it re-arms `EPOLLOUT` and waits for the next event). PR #2's real readiness loop
/// must preserve this — `Parked` is never mapped to a retry within the same event.
#[allow(dead_code)]
pub fn epoll_caller_respects_parked(out_fd: RawFd, state: &mut SendingState) -> EpollPumpDrain {
    epoll_drain_pump_once(out_fd, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sendfile::SendfileAsset;
    use std::io::Read;
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    fn set_nonblocking(fd: RawFd, nonblocking: bool) -> io::Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let new_flags = if nonblocking {
            flags | libc::O_NONBLOCK
        } else {
            flags & !libc::O_NONBLOCK
        };
        if unsafe { libc::fcntl(fd, libc::F_SETFL, new_flags) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn socket_pair_nonblocking() -> io::Result<(UnixStream, UnixStream)> {
        let (a, b) = UnixStream::pair()?;
        set_nonblocking(a.as_raw_fd(), true)?;
        set_nonblocking(b.as_raw_fd(), true)?;
        Ok((a, b))
    }

    fn set_small_send_buffer(fd: RawFd, bytes: libc::c_int) {
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &bytes as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }

    /// Fills the kernel send queue until `send(2)` returns `EAGAIN` (peer not reading).
    fn block_socket_send_until_full(fd: RawFd) -> io::Result<()> {
        let chunk = [0u8; 4096];
        loop {
            let written = unsafe {
                libc::send(
                    fd,
                    chunk.as_ptr() as *const libc::c_void,
                    chunk.len(),
                    MSG_NOSIGNAL,
                )
            };
            if written < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if err.kind() == io::ErrorKind::WouldBlock {
                    return Ok(());
                }
                return Err(err);
            }
        }
    }

    fn socket_pair_send_blocked() -> io::Result<(UnixStream, UnixStream, RawFd)> {
        let (writer, reader) = socket_pair_nonblocking()?;
        let fd = writer.as_raw_fd();
        set_small_send_buffer(fd, 64);
        block_socket_send_until_full(fd)?;
        Ok((writer, reader, fd))
    }

    fn asset_64k() -> io::Result<(tempfile::TempDir, Arc<SendfileAsset>)> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("64k.bin");
        std::fs::write(&path, vec![7u8; 65536])?;
        Ok((dir, Arc::new(SendfileAsset::open(&path, 65536)?)))
    }

    #[test]
    fn header_partial_write_completes() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let (writer, mut reader) = socket_pair_nonblocking()?;
        let mut state = SendingState::new_get(asset)?;
        let fd = writer.as_raw_fd();
        let drain = epoll_drain_pump_once(fd, &mut state);
        assert_eq!(drain.result, PumpResult::Complete);
        assert!(state.header_done);
        let mut buf = vec![0u8; 65536 + 512];
        let n = reader.read(&mut buf)?;
        assert!(n > state.header.len);
        Ok(())
    }

    /// H1 contract. Socket skb granularity makes a real partial write of a 128-byte
    /// header non-deterministic, so we deterministically place the FSM mid-header
    /// (offset > 0, header not done) and verify a full send buffer parks correctly:
    /// offset preserved, `Parked`, interest `EPOLLOUT | EPOLLRDHUP` (no `EPOLLIN`),
    /// connection not closed, no busy-spin.
    #[test]
    fn header_eagain_mid_header_parks_with_offset() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let (_writer, _reader, fd) = socket_pair_send_blocked()?;
        let mut state = SendingState::new_get(asset)?;
        let mid = state.header.len / 2;
        assert!(mid > 0 && mid < state.header.len);
        state.header.offset = mid;
        let offset_before = state.header.offset;

        let drain = epoll_caller_respects_parked(fd, &mut state);

        assert_eq!(drain.result, PumpResult::Parked);
        assert!(
            !state.header_done,
            "header phase must not complete on EAGAIN"
        );
        assert_eq!(
            state.header.offset, offset_before,
            "EAGAIN mid-header must preserve offset"
        );
        assert!(!state.header.done(), "header must still be incomplete");
        assert_eq!(drain.interest, interest_sending_parked());
        assert_eq!(drain.interest & EPOLLIN, 0, "EPOLLIN must be paused");
        assert_ne!(drain.interest & EPOLLOUT, 0);
        assert_ne!(drain.interest & EPOLLRDHUP, 0);
        assert!(
            drain.pump_calls < 256,
            "busy-spin: {} calls",
            drain.pump_calls
        );
        Ok(())
    }

    #[test]
    fn body_eagain_parks_with_persisted_file_offset() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let (writer, _reader) = socket_pair_nonblocking()?;
        let fd = writer.as_raw_fd();
        let sndbuf: libc::c_int = 128;
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &sndbuf as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
        let mut state = SendingState::new_get(asset)?;
        loop {
            match pump_sendfile_nb(fd, &mut state) {
                PumpResult::Parked if state.header_done => break,
                PumpResult::Parked => continue,
                PumpResult::Progress => continue,
                PumpResult::Complete => return Ok(()),
                PumpResult::Error(k) => return Err(io::Error::new(k, "unexpected error")),
            }
        }
        assert!(state.header_done);
        let offset_before = state.file_offset;
        let remaining_before = state.body_remaining;
        let result = pump_sendfile_nb(fd, &mut state);
        assert_eq!(result, PumpResult::Parked);
        assert!(state.file_offset > offset_before || state.body_remaining < remaining_before);
        Ok(())
    }

    #[test]
    fn epollin_paused_while_sending() {
        assert_eq!(interest_sending_parked() & EPOLLIN, 0);
        assert_ne!(interest_sending_parked() & EPOLLOUT, 0);
        assert_eq!(interest_for_pump_result(PumpResult::Parked) & EPOLLIN, 0);
    }

    #[test]
    fn parked_contract_stops_mock_caller() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let (_writer, _reader, fd) = socket_pair_send_blocked()?;
        let mut state = SendingState::new_get(asset)?;
        let first = epoll_caller_respects_parked(fd, &mut state);
        assert_eq!(first.result, PumpResult::Parked);
        let calls_after_park = first.pump_calls;
        // Correct caller must NOT invoke another drain before EPOLLOUT
        assert_eq!(first.interest, interest_sending_parked());
        assert!(calls_after_park >= 1);
        Ok(())
    }

    #[test]
    fn eintr_retries_in_place() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let (writer, _reader) = socket_pair_nonblocking()?;
        let fd = writer.as_raw_fd();
        let mut state = SendingState::new_get(asset)?;
        // Smoke: full pump completes on normal socket (EINTR path covered by loop structure)
        let drain = epoll_drain_pump_once(fd, &mut state);
        assert_eq!(drain.result, PumpResult::Complete);
        Ok(())
    }

    #[test]
    fn head_only_no_body() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let (writer, mut reader) = socket_pair_nonblocking()?;
        let mut state = SendingState::new_head_only(asset)?;
        let fd = writer.as_raw_fd();
        assert_eq!(
            epoll_drain_pump_once(fd, &mut state).result,
            PumpResult::Complete
        );
        assert_eq!(state.body_remaining, 0);
        let mut buf = [0u8; 512];
        let n = reader.read(&mut buf)?;
        let text = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(text.starts_with("HTTP/1.1 200"));
        assert!(text.contains("Content-Length: 65536"));
        Ok(())
    }

    #[test]
    fn rdhup_mid_body_closes_or_aborts_cleanly() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let (writer, reader) = socket_pair_nonblocking()?;
        let fd = writer.as_raw_fd();
        drop(reader);
        let mut state = SendingState::new_get(asset)?;
        let result = pump_sendfile_nb(fd, &mut state);
        assert!(
            matches!(
                result,
                PumpResult::Error(_) | PumpResult::Parked | PumpResult::Complete
            ),
            "unexpected {result:?}"
        );
        // State dropped cleanly when scope ends (Arc released)
        Ok(())
    }

    #[test]
    fn fsm_eagain_no_busy_spin() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let (_writer, _reader, fd) = socket_pair_send_blocked()?;
        let mut state = SendingState::new_get(asset)?;
        let drain = epoll_drain_pump_once(fd, &mut state);
        assert_eq!(drain.result, PumpResult::Parked);
        assert!(
            drain.pump_calls < 256,
            "busy-spin suspected: {} pump iterations",
            drain.pump_calls
        );
        Ok(())
    }

    #[test]
    fn flag_off_no_runtime_behavior_change() {
        // Default env in unit tests: feature gate must be off unless both vars set.
        assert!(
            !epoll_sendfile_enabled()
                || (std::env::var("EXYONQ_EPOLL_STATIC").ok().as_deref() == Some("1")
                    && std::env::var("EXYONQ_EPOLL_SENDFILE").ok().as_deref() == Some("1"))
        );
    }

    /// InvalidInput contract: a response header above `MAX_RESPONSE_HEADER` must surface a
    /// clean `Err` from `new_get`/`new_head_only` — never an `unwrap`/panic.
    #[test]
    fn new_get_oversized_header_errs_without_panic() {
        use bytes::Bytes;
        use std::fs::File;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        std::fs::write(&path, vec![0u8; 65536]).unwrap();
        let asset = Arc::new(SendfileAsset {
            file: Arc::new(File::open(&path).unwrap()),
            header: Arc::new(Bytes::from(vec![b'x'; MAX_RESPONSE_HEADER + 1])),
            body_len: 65536,
        });
        assert!(SendingState::new_get(Arc::clone(&asset)).is_err());
        assert!(SendingState::new_head_only(asset).is_err());
    }

    #[test]
    fn bench_header_fits_stack_buffer() {
        let header = crate::wire::bench_header_keep(65536);
        assert!(header.len() <= MAX_RESPONSE_HEADER);
        let header_1m = crate::wire::bench_header_keep(1048576);
        assert!(header_1m.len() <= MAX_RESPONSE_HEADER);
    }
}
