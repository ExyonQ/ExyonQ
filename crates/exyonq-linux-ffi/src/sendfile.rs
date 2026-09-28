//! `sendfile64` step helper (Linux).

use std::io;
use std::os::fd::RawFd;

/// Result of one non-blocking sendfile attempt.
#[derive(Debug)]
pub enum SendfileStep {
    /// Transferred `n` bytes; file offset advanced by the kernel.
    Progress(usize),
    /// Would block (`EAGAIN` / `EWOULDBLOCK`).
    Parked,
    /// Transfer complete for this call's remaining count (including 0-length EOF).
    Done,
}

/// One `sendfile64` step. Retries `EINTR`. Does not loop on partial progress.
pub fn sendfile64_step(
    out_fd: RawFd,
    in_fd: RawFd,
    file_offset: &mut i64,
    count: usize,
) -> io::Result<SendfileStep> {
    if count == 0 {
        return Ok(SendfileStep::Done);
    }
    loop {
        let sent = unsafe {
            // SAFETY: both fds open for the duration; `file_offset` points to valid i64.
            libc::sendfile64(out_fd, in_fd, file_offset, count)
        };
        if sent < 0 {
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                Some(e) if e == libc::EINTR => continue,
                Some(e) if e == libc::EAGAIN || e == libc::EWOULDBLOCK => {
                    return Ok(SendfileStep::Parked);
                }
                _ => return Err(err),
            }
        }
        if sent == 0 {
            return Ok(SendfileStep::Done);
        }
        return Ok(SendfileStep::Progress(sent as usize));
    }
}
