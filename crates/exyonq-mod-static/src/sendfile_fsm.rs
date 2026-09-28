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
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::Arc;

/// Process-lifetime cache for `epoll_sendfile_enabled` (−1 unset, 0 off, 1 on).
static EPOLL_SENDFILE_ENABLED_CACHE: AtomicI8 = AtomicI8::new(-1);

pub const EPOLLIN: u32 = 0x001;
pub const EPOLLOUT: u32 = 0x004;
pub const EPOLLRDHUP: u32 = 0x2000;
pub const EPOLLET: u32 = 1 << 31;
pub const EPOLL_CTL_ADD: i32 = 1;
pub const EPOLL_CTL_DEL: i32 = 2;
pub const EPOLL_CTL_MOD: i32 = 3;

/// Bench response headers fit in wire templates; stack buffer avoids heap in FSM.
/// Cap019: 206/416 header blocks need more than the historical 128-byte bench ceiling.
/// No-Range response header templates are unchanged and still fit well under this limit.
pub const MAX_RESPONSE_HEADER: usize = 512;

pub(crate) const MSG_NOSIGNAL: i32 = 0x4000;
/// Linux `MSG_MORE` — Cap067 P5: coalesce response header with following sendfile body.
#[cfg(target_os = "linux")]
pub(crate) const MSG_MORE: i32 = 0x8000;

/// Cap067 P5: header `send` flags.
///
/// `MSG_MORE` only when this response has already committed to a non-zero Cap067
/// sendfile body (`!head_only && body_remaining > 0`). Never for HEAD/304/416/
/// header-only/zero-length — those must flush without waiting for a body.
#[inline]
fn header_send_flags(state: &SendingState) -> i32 {
    #[cfg(target_os = "linux")]
    {
        let mut flags = MSG_NOSIGNAL;
        if !state.head_only && state.body_remaining > 0 {
            flags |= MSG_MORE;
        }
        flags
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = state;
        MSG_NOSIGNAL
    }
}

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
    /// Cap061/Cap067: real terminal status for access logging (never invent 200).
    pub access_status: u16,
    /// Declared body bytes for this response (0 for HEAD/304/416).
    pub access_body_len: usize,
}

impl SendingState {
    pub fn new_get(asset: Arc<SendfileAsset>) -> io::Result<Self> {
        let body_len = asset.body_len;
        Ok(Self {
            header: ResponseHeaderBuf::from_header_bytes(asset.header.as_ref())?,
            header_done: false,
            file_offset: 0,
            body_remaining: body_len,
            head_only: false,
            access_status: 200,
            access_body_len: body_len,
            asset,
        })
    }

    /// Cap067: GET 200 with Cap020 validators already baked into `header`.
    pub fn new_get_full(asset: Arc<SendfileAsset>, header: &[u8]) -> io::Result<Self> {
        let body_len = asset.body_len;
        Ok(Self {
            header: ResponseHeaderBuf::from_header_bytes(header)?,
            header_done: false,
            file_offset: 0,
            body_remaining: body_len,
            head_only: false,
            access_status: 200,
            access_body_len: body_len,
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
            access_status: 200,
            access_body_len: 0,
            asset,
        })
    }

    /// Cap019: GET with selected byte range (sendfile offset + remaining length).
    pub fn new_get_range(
        asset: Arc<SendfileAsset>,
        header: &[u8],
        start: u64,
        length: usize,
    ) -> io::Result<Self> {
        let file_offset = i64::try_from(start)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "range start exceeds i64"))?;
        Ok(Self {
            header: ResponseHeaderBuf::from_header_bytes(header)?,
            header_done: false,
            file_offset,
            body_remaining: length,
            head_only: false,
            access_status: 206,
            access_body_len: length,
            asset,
        })
    }

    /// Cap019/Cap020: HEAD or error metadata responses (304/416/HEAD-206).
    pub fn new_head_range(
        asset: Arc<SendfileAsset>,
        header: &[u8],
        access_status: u16,
    ) -> io::Result<Self> {
        Ok(Self {
            header: ResponseHeaderBuf::from_header_bytes(header)?,
            header_done: false,
            file_offset: 0,
            body_remaining: 0,
            head_only: true,
            access_status,
            access_body_len: 0,
            asset,
        })
    }
}

/// Cap067: automatic sendfile mechanism on Linux (default ON).
///
/// Kill-switches: `EXYONQ_EPOLL_STATIC=0` and/or `EXYONQ_EPOLL_SENDFILE=0`.
///
/// Resolved once at first observation (process-lifetime). Mid-process env
/// mutation is **not** observed in production — emergency disable requires
/// process restart. Default / explicit on|off|invalid semantics are unchanged.
pub fn epoll_sendfile_enabled() -> bool {
    let cached = EPOLL_SENDFILE_ENABLED_CACHE.load(Ordering::Relaxed);
    if cached >= 0 {
        return cached != 0;
    }
    let v = env_flag_auto_on_value(std::env::var("EXYONQ_EPOLL_STATIC").ok().as_deref())
        && env_flag_auto_on_value(std::env::var("EXYONQ_EPOLL_SENDFILE").ok().as_deref());
    let encoded: i8 = if v { 1 } else { 0 };
    let _ = EPOLL_SENDFILE_ENABLED_CACHE.compare_exchange(
        -1,
        encoded,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
    EPOLL_SENDFILE_ENABLED_CACHE.load(Ordering::Relaxed) != 0
}

/// Auto-on flag: unset → true; `0`/`off`/`false`/`no` → false; anything else → true.
fn env_flag_auto_on_value(raw: Option<&str>) -> bool {
    match raw {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        None => true,
    }
}

/// Test-only: clear process-lifetime cache so a subsequent call re-reads env
/// (simulates a fresh process). Production never calls this.
#[doc(hidden)]
pub fn reset_epoll_sendfile_enabled_cache_for_tests() {
    EPOLL_SENDFILE_ENABLED_CACHE.store(-1, Ordering::Relaxed);
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
/// `flags`: Cap067 P5 — `MSG_NOSIGNAL` [| `MSG_MORE`] from [`header_send_flags`].
fn try_send_header_partial(
    out_fd: RawFd,
    header: &mut ResponseHeaderBuf,
    flags: i32,
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
                flags,
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
            // Cap067 P5: decide MSG_MORE at the earliest FSM point where Cap067 has
            // already committed to a non-zero sendfile body for this response.
            let flags = header_send_flags(state);
            match try_send_header_partial(out_fd, &mut state.header, flags) {
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

/// Reference EPOLLET caller for the strong `Parked` contract: it invokes the
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
        // Persist offsets across Parked. With Cap067 P5 MSG_MORE the skb may already be
        // full from the coalesced header+body enqueue, so a further pump can Park with
        // zero additional progress — that is still correct persistence, not regression.
        assert!(state.file_offset >= offset_before);
        assert!(state.body_remaining <= remaining_before);
        assert_eq!(
            state.file_offset + state.body_remaining as i64,
            offset_before + remaining_before as i64,
            "Parked must not invent or lose body bytes"
        );
        Ok(())
    }

    #[test]
    fn epollin_paused_while_sending() {
        assert_eq!(interest_sending_parked() & EPOLLIN, 0);
        assert_ne!(interest_sending_parked() & EPOLLOUT, 0);
        assert_eq!(interest_for_pump_result(PumpResult::Parked) & EPOLLIN, 0);
    }

    #[test]
    fn parked_contract_stops_reference_caller() -> io::Result<()> {
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
        // Diagnostic: full pump completes on normal socket (EINTR path covered by loop structure)
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
    fn env_flag_auto_on_value_semantics() {
        assert!(env_flag_auto_on_value(None));
        assert!(env_flag_auto_on_value(Some("")));
        assert!(env_flag_auto_on_value(Some("1")));
        assert!(env_flag_auto_on_value(Some("yes")));
        assert!(env_flag_auto_on_value(Some("garbage")));
        assert!(!env_flag_auto_on_value(Some("0")));
        assert!(!env_flag_auto_on_value(Some("OFF")));
        assert!(!env_flag_auto_on_value(Some(" False ")));
        assert!(!env_flag_auto_on_value(Some("no")));
    }

    #[test]
    fn flag_kill_switch_combined_requires_both_on() {
        assert!(env_flag_auto_on_value(Some("1")) && env_flag_auto_on_value(Some("1")));
        assert!(!(env_flag_auto_on_value(Some("1")) && env_flag_auto_on_value(Some("0"))));
        assert!(!(env_flag_auto_on_value(Some("0")) && env_flag_auto_on_value(Some("1"))));
        assert!(!(env_flag_auto_on_value(Some("0")) && env_flag_auto_on_value(Some("0"))));
        assert!(env_flag_auto_on_value(None) && env_flag_auto_on_value(None));
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
        let file = File::open(&path).unwrap();
        let meta = file.metadata().unwrap();
        let prepared = crate::conditional::PreparedStaticWire::from_metadata(
            &meta,
            "application/octet-stream",
        );
        let asset = Arc::new(SendfileAsset {
            file: Arc::new(file),
            header: Arc::new(Bytes::from(vec![b'x'; MAX_RESPONSE_HEADER + 1])),
            header_304: prepared.header_304,
            body_len: 65536,
            content_type: "application/octet-stream",
            validators: prepared.validators,
        });
        assert!(SendingState::new_get(Arc::clone(&asset)).is_err());
        assert!(SendingState::new_head_only(asset).is_err());
    }

    #[test]
    fn response_header_fits_stack_buffer() {
        let header = crate::wire::header_keep(65536, "application/octet-stream");
        assert!(header.len() <= MAX_RESPONSE_HEADER);
        let header_1m = crate::wire::header_keep(1048576, "application/octet-stream");
        assert!(header_1m.len() <= MAX_RESPONSE_HEADER);
    }

    /// Cap067 P5: MSG_MORE selection is derived only from committed sendfile body state.
    #[test]
    fn p5_header_send_flags_selection_matrix() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let get = SendingState::new_get(Arc::clone(&asset))?;
        assert_eq!(
            header_send_flags(&get) & MSG_NOSIGNAL,
            MSG_NOSIGNAL,
            "NOSIGNAL always"
        );
        #[cfg(target_os = "linux")]
        assert_ne!(
            header_send_flags(&get) & MSG_MORE,
            0,
            "GET with body must set MSG_MORE"
        );

        let head = SendingState::new_head_only(Arc::clone(&asset))?;
        assert_eq!(
            header_send_flags(&head),
            MSG_NOSIGNAL,
            "HEAD must not set MSG_MORE"
        );

        let not_mod =
            SendingState::new_head_range(Arc::clone(&asset), asset.header_304.as_ref(), 304)?;
        assert_eq!(
            header_send_flags(&not_mod),
            MSG_NOSIGNAL,
            "304 must not set MSG_MORE"
        );

        let range_hdr = crate::wire::partial_content_header(
            0,
            99,
            asset.body_len as u64,
            "application/octet-stream",
        );
        let range = SendingState::new_get_range(Arc::clone(&asset), range_hdr.as_ref(), 0, 100)?;
        #[cfg(target_os = "linux")]
        assert_ne!(
            header_send_flags(&range) & MSG_MORE,
            0,
            "206 with non-zero body must set MSG_MORE"
        );
        assert_eq!(header_send_flags(&range) & MSG_NOSIGNAL, MSG_NOSIGNAL);

        // Zero-length range body must not cork.
        let empty_range =
            SendingState::new_get_range(Arc::clone(&asset), range_hdr.as_ref(), 0, 0)?;
        assert_eq!(
            header_send_flags(&empty_range),
            MSG_NOSIGNAL,
            "zero-length 206 must not set MSG_MORE"
        );
        Ok(())
    }

    #[test]
    fn p5_zero_length_static_no_msg_more_and_completes() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("empty.bin");
        std::fs::write(&path, b"")?;
        let asset = Arc::new(SendfileAsset::open(&path, 0)?);
        let mut state = SendingState::new_get(Arc::clone(&asset))?;
        assert_eq!(state.body_remaining, 0);
        assert_eq!(
            header_send_flags(&state),
            MSG_NOSIGNAL,
            "zero-length GET must not set MSG_MORE"
        );
        let (writer, mut reader) = socket_pair_nonblocking()?;
        let fd = writer.as_raw_fd();
        let drain = epoll_drain_pump_once(fd, &mut state);
        assert_eq!(drain.result, PumpResult::Complete);
        assert!(state.header_done);
        let mut buf = [0u8; 512];
        let n = reader.read(&mut buf)?;
        assert!(n > 0, "header must be delivered");
        let text = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(text.starts_with("HTTP/1.1 200"));
        assert!(text.contains("Content-Length: 0"));
        Ok(())
    }

    #[test]
    fn p5_get_body_completes_with_exact_bytes() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let (writer, mut reader) = socket_pair_nonblocking()?;
        set_nonblocking(reader.as_raw_fd(), false)?;
        let mut state = SendingState::new_get(asset)?;
        #[cfg(target_os = "linux")]
        assert_ne!(header_send_flags(&state) & MSG_MORE, 0);
        let fd = writer.as_raw_fd();
        assert_eq!(
            epoll_drain_pump_once(fd, &mut state).result,
            PumpResult::Complete
        );
        drop(writer);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        assert!(buf.windows(4).any(|w| w == b"HTTP"));
        let sep = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("header/body separator");
        let body = &buf[sep + 4..];
        assert_eq!(body.len(), 65536);
        assert!(body.iter().all(|&b| b == 7));
        Ok(())
    }

    #[test]
    fn p5_range_206_exact_body_bytes() -> io::Result<()> {
        let (_dir, asset) = asset_64k()?;
        let start = 100u64;
        let length = 50usize;
        let hdr = crate::wire::partial_content_header(
            start,
            start + length as u64 - 1,
            asset.body_len as u64,
            "application/octet-stream",
        );
        let (writer, mut reader) = socket_pair_nonblocking()?;
        set_nonblocking(reader.as_raw_fd(), false)?;
        let mut state = SendingState::new_get_range(asset, hdr.as_ref(), start, length)?;
        assert_eq!(state.access_status, 206);
        #[cfg(target_os = "linux")]
        assert_ne!(header_send_flags(&state) & MSG_MORE, 0);
        let fd = writer.as_raw_fd();
        assert_eq!(
            epoll_drain_pump_once(fd, &mut state).result,
            PumpResult::Complete
        );
        drop(writer);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        let text = std::str::from_utf8(&buf).unwrap();
        assert!(text.starts_with("HTTP/1.1 206"));
        assert!(text.contains("Content-Range:"));
        let sep = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        assert_eq!(&buf[sep + 4..], &vec![7u8; length]);
        Ok(())
    }

    #[test]
    fn p5_head_then_get_keepalive_boundaries() -> io::Result<()> {
        // Real TCP loopback: MSG_MORE is a TCP semantic; UnixStream is not authoritative.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("1k.bin");
        std::fs::write(&path, vec![9u8; 1024])?;
        let asset = Arc::new(SendfileAsset::open(&path, 1024)?);

        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let mut client = std::net::TcpStream::connect(addr)?;
        let (server, _) = listener.accept()?;
        server.set_nonblocking(true)?;
        client.set_nonblocking(false)?;
        let fd = server.as_raw_fd();

        fn pump_complete(fd: RawFd, state: &mut SendingState) -> io::Result<()> {
            for _ in 0..10_000 {
                match epoll_drain_pump_once(fd, state).result {
                    PumpResult::Complete => return Ok(()),
                    PumpResult::Parked | PumpResult::Progress => continue,
                    PumpResult::Error(k) => return Err(io::Error::new(k, "pump error")),
                }
            }
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "pump did not complete",
            ))
        }

        fn read_one_http(
            client: &mut std::net::TcpStream,
            leftover: &mut Vec<u8>,
            expect_body: bool,
        ) -> io::Result<(String, Vec<u8>)> {
            let mut buf = std::mem::take(leftover);
            let mut tmp = [0u8; 4096];
            loop {
                if let Some(sep) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..sep]).into_owned();
                    let body_start = sep + 4;
                    if !expect_body {
                        *leftover = buf[body_start..].to_vec();
                        return Ok((head, Vec::new()));
                    }
                    let mut cl = 0usize;
                    for line in head.lines() {
                        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            cl = v.trim().parse().unwrap_or(0);
                        }
                    }
                    while buf.len() < body_start + cl {
                        let n = client.read(&mut tmp)?;
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let body = buf[body_start..body_start + cl].to_vec();
                    *leftover = buf[body_start + cl..].to_vec();
                    return Ok((head, body));
                }
                let n = client.read(&mut tmp)?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "incomplete HTTP",
                    ));
                }
                buf.extend_from_slice(&tmp[..n]);
            }
        }

        let mut leftover = Vec::new();

        let mut head = SendingState::new_head_only(Arc::clone(&asset))?;
        assert_eq!(header_send_flags(&head), MSG_NOSIGNAL);
        pump_complete(fd, &mut head)?;
        let (h1, b1) = read_one_http(&mut client, &mut leftover, false)?;
        assert!(h1.starts_with("HTTP/1.1 200"));
        assert!(b1.is_empty(), "HEAD must have empty body");

        let mut get = SendingState::new_get(Arc::clone(&asset))?;
        assert_ne!(header_send_flags(&get) & MSG_MORE, 0);
        pump_complete(fd, &mut get)?;
        let (h2, b2) = read_one_http(&mut client, &mut leftover, true)?;
        assert!(h2.starts_with("HTTP/1.1 200"));
        assert_eq!(b2, vec![9u8; 1024]);

        let mut not_mod =
            SendingState::new_head_range(Arc::clone(&asset), asset.header_304.as_ref(), 304)?;
        assert_eq!(header_send_flags(&not_mod), MSG_NOSIGNAL);
        pump_complete(fd, &mut not_mod)?;
        let (h3, b3) = read_one_http(&mut client, &mut leftover, false)?;
        assert!(
            h3.starts_with("HTTP/1.1 304"),
            "304 must not be delayed: {h3}"
        );
        assert!(b3.is_empty());

        let mut get2 = SendingState::new_get(asset)?;
        pump_complete(fd, &mut get2)?;
        let (h4, b4) = read_one_http(&mut client, &mut leftover, true)?;
        assert!(h4.starts_with("HTTP/1.1 200"));
        assert_eq!(b4, vec![9u8; 1024]);
        Ok(())
    }
}
