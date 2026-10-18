//! `openat2` + resolve flags (Linux Cap004 / CFD containment).

use std::ffi::CStr;
use std::fs::File;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

/// Errors from openat2 helpers.
#[derive(Debug)]
pub enum Openat2Error {
    /// Kernel lacks `openat2` (`ENOSYS`).
    Unavailable,
    Io(io::Error),
}

/// Open under `dir_fd` with caller-supplied `flags` / `resolve` (typically includes `RESOLVE_BENEATH`).
pub fn openat2_beneath(
    dir_fd: RawFd,
    rel_c_path: &CStr,
    flags: u64,
    resolve: u64,
) -> Result<File, Openat2Error> {
    let mut how: libc::open_how = unsafe {
        // SAFETY: open_how is POD; zeroed then fields set below.
        std::mem::zeroed()
    };
    how.flags = flags;
    how.mode = 0;
    how.resolve = resolve;

    let raw = unsafe {
        // SAFETY: dir_fd open directory; path/how live for the syscall.
        libc::syscall(
            libc::SYS_openat2,
            dir_fd,
            rel_c_path.as_ptr(),
            &how as *const libc::open_how,
            std::mem::size_of::<libc::open_how>(),
        )
    };

    if raw < 0 {
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(e) if e == libc::ENOSYS => return Err(Openat2Error::Unavailable),
            Some(e) if e == libc::EXDEV || e == libc::ELOOP || e == libc::ENOTDIR => {
                return Err(Openat2Error::Io(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "static open escaped configured root",
                )));
            }
            _ => return Err(Openat2Error::Io(err)),
        }
    }

    let file = File::from(unsafe {
        // SAFETY: successful openat2 returns an exclusive open fd.
        OwnedFd::from_raw_fd(raw as i32)
    });
    Ok(file)
}

/// Cap004 default: `O_RDONLY|O_NOFOLLOW|O_CLOEXEC` + `RESOLVE_BENEATH`.
pub fn openat2_rdonly_beneath(dir_fd: RawFd, rel_c_path: &CStr) -> Result<File, Openat2Error> {
    openat2_beneath(
        dir_fd,
        rel_c_path,
        (libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC) as u64,
        libc::RESOLVE_BENEATH as u64,
    )
}
