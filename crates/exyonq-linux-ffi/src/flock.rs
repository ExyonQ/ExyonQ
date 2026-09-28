//! `flock(2)` exclusive lock helper (CFD single-process gate).

use std::io;
use std::os::fd::RawFd;

/// Non-blocking exclusive flock. Returns `Ok(true)` if locked, `Ok(false)` if would block.
pub fn try_flock_exclusive(fd: RawFd) -> io::Result<bool> {
    let rc = unsafe {
        // SAFETY: `fd` must remain open for the lock lifetime owned by the caller.
        libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB)
    };
    if rc == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) || err.kind() == io::ErrorKind::WouldBlock {
        return Ok(false);
    }
    Err(err)
}
