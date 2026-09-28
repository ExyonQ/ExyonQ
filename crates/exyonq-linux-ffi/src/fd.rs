//! Owned / raw FD helpers (Linux).

use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

/// Duplicate an open fd. Caller owns the returned [`OwnedFd`].
pub fn dup_raw_fd(fd: RawFd) -> io::Result<OwnedFd> {
    let dup_fd = unsafe {
        // SAFETY: `fd` must be open; `dup` returns a new independent fd or -1.
        libc::dup(fd)
    };
    if dup_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe {
        // SAFETY: `dup_fd` is a freshly duplicated open fd exclusive to this OwnedFd.
        OwnedFd::from_raw_fd(dup_fd)
    })
}

/// Close a raw fd. Prefer [`OwnedFd`] drop when possible.
pub fn close_raw_fd(fd: RawFd) {
    unsafe {
        // SAFETY: caller guarantees `fd` is open and not used after this call.
        let _ = libc::close(fd);
    }
}
