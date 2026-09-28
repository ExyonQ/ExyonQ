//! Descriptor-relative hard-link publication for Unix filesystem objects.

use std::ffi::CStr;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

/// Create a hard link without replacing an existing destination.
///
/// Both paths are resolved relative to their supplied directory descriptors.
/// `linkat(2)` is atomic and returns `AlreadyExists` when `new_path` is occupied.
pub fn hard_link_at(
    old_dir: BorrowedFd<'_>,
    old_path: &CStr,
    new_dir: BorrowedFd<'_>,
    new_path: &CStr,
) -> io::Result<()> {
    let result = unsafe {
        // SAFETY: both C strings are NUL-terminated for the duration of the call;
        // borrowed descriptors remain valid and linkat does not retain pointers.
        libc::linkat(
            old_dir.as_raw_fd(),
            old_path.as_ptr(),
            new_dir.as_raw_fd(),
            new_path.as_ptr(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
