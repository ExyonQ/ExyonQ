//! send / write on raw sockets and fds (Linux).

use std::io;
use std::os::fd::RawFd;

const MSG_NOSIGNAL: i32 = 0x4000;

/// Write the full buffer with `send(..., MSG_NOSIGNAL)`, falling back to write loops.
pub fn send_all_nosignal(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        let written = unsafe {
            // SAFETY: `fd` open for write; `buf` live for the syscall.
            libc::send(
                fd,
                buf.as_ptr() as *const libc::c_void,
                buf.len(),
                MSG_NOSIGNAL,
            )
        };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "send returned 0"));
        }
        buf = &buf[written as usize..];
    }
    Ok(())
}

/// Write every buffer with one `sendmsg` (`MSG_NOSIGNAL`), then continue on a short write.
///
/// `WouldBlock` before any byte is returned as-is so the caller can retry the whole
/// response. `WouldBlock` after a short write is `WriteZero`: some bytes are already
/// on the socket, and retrying the original buffers would duplicate them.
pub fn send_vectored_nosignal(fd: RawFd, bufs: &[&[u8]]) -> io::Result<()> {
    if bufs.is_empty() {
        return Ok(());
    }
    if bufs.len() > 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "send_vectored_nosignal accepts at most two buffers",
        ));
    }
    let total: usize = bufs.iter().map(|b| b.len()).sum();
    let mut sent = 0usize;
    while sent < total {
        let mut iov = [
            libc::iovec {
                iov_base: std::ptr::null_mut(),
                iov_len: 0,
            },
            libc::iovec {
                iov_base: std::ptr::null_mut(),
                iov_len: 0,
            },
        ];
        let mut n_iov = 0usize;
        let mut skip = sent;
        for buf in bufs {
            if skip >= buf.len() {
                skip -= buf.len();
                continue;
            }
            let start = skip;
            skip = 0;
            iov[n_iov] = libc::iovec {
                iov_base: buf[start..].as_ptr() as *mut libc::c_void,
                iov_len: buf.len() - start,
            };
            n_iov += 1;
        }
        // musl's libc crate hides `msghdr` padding, so a struct literal does not compile there.
        let mut msg: libc::msghdr = unsafe {
            // SAFETY: `msghdr` is a plain header. All-zero is a valid message with
            // no name and no ancillary data; `msg_iov` is assigned before `sendmsg`.
            std::mem::zeroed()
        };
        msg.msg_iov = iov.as_mut_ptr();
        msg.msg_iovlen = n_iov as _;
        let n = unsafe {
            // SAFETY: `fd` is open for write; each iov points into a live `buf` slice;
            // `msg` does not carry a control buffer.
            libc::sendmsg(fd, &msg, MSG_NOSIGNAL)
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if err.kind() == io::ErrorKind::WouldBlock {
                if sent == 0 {
                    return Err(err);
                }
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "partial vectored send",
                ));
            }
            return Err(err);
        }
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "sendmsg returned 0",
            ));
        }
        sent += n as usize;
    }
    Ok(())
}

/// Write the full buffer with `write(2)`.
pub fn write_all_fd(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        let written = unsafe {
            // SAFETY: `fd` open for write; `buf` live for the syscall.
            libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len())
        };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0"));
        }
        buf = &buf[written as usize..];
    }
    Ok(())
}

/// Set `SO_LINGER(1, 0)` so a subsequent close sends RST (no TIME-WAIT).
///
/// Best-effort: returns `Ok(())` even when the kernel rejects the option; callers
/// that need hard failure should check the `Result`.
pub fn set_abortive_linger(fd: RawFd) -> io::Result<()> {
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    let rc = unsafe {
        // SAFETY: `fd` is a live socket fd; `linger` is stack-local for the call.
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            &linger as *const _ as *const libc::c_void,
            std::mem::size_of_val(&linger) as libc::socklen_t,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
