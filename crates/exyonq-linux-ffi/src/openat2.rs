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
        libc::RESOLVE_BENEATH,
    )
}

/// Open one directory component relative to `dir_fd` without following symlinks.
///
/// The returned [`File`] owns the new descriptor and closes it on drop.
pub fn openat_directory_nofollow(dir_fd: RawFd, name: &CStr) -> io::Result<File> {
    openat_owned(
        dir_fd,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )
}

/// Open one final read-only component relative to `dir_fd` without following symlinks.
///
/// `O_NONBLOCK` prevents an attacker-controlled special file from blocking before
/// the caller can reject it with `fstat`.
pub fn openat_readonly_nofollow(dir_fd: RawFd, name: &CStr) -> io::Result<File> {
    openat_owned(
        dir_fd,
        name,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
    )
}

fn openat_owned(dir_fd: RawFd, name: &CStr, flags: libc::c_int) -> io::Result<File> {
    let component = name.to_bytes();
    if component.is_empty() || component == b"." || component == b".." || component.contains(&b'/')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "openat path must be one normal component",
        ));
    }

    // SAFETY: `name` is NUL-terminated for the duration of the call. `openat`
    // does not take ownership of `dir_fd`; on success the returned descriptor
    // is new and exclusively transferred into `OwnedFd` below.
    let raw = unsafe { libc::openat(dir_fd, name.as_ptr(), flags) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: a successful `openat` returns a new owned descriptor.
    let owned = unsafe { OwnedFd::from_raw_fd(raw) };
    Ok(File::from(owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::fd::AsRawFd;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "exyonq-openat2-{}-{label}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn openat_walk_helpers_reject_symlink_components() {
        let root = TempDir::new("root");
        let outside = TempDir::new("outside");
        fs::create_dir(root.path().join("nested")).expect("nested dir");
        fs::write(root.path().join("nested/file.txt"), b"inside").expect("inside file");
        fs::write(outside.path().join("secret.txt"), b"outside").expect("outside file");
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))
            .expect("intermediate symlink");
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            root.path().join("final-link"),
        )
        .expect("final symlink");

        let root_file = File::open(root.path()).expect("open root");
        let nested = c"nested";
        let file = c"file.txt";
        let opened =
            openat_directory_nofollow(root_file.as_raw_fd(), nested).expect("open nested dir");
        openat_readonly_nofollow(opened.as_raw_fd(), file).expect("open nested file");

        for name in ["escape", "final-link"] {
            let bytes = [name.as_bytes(), b"\0"].concat();
            let component = CStr::from_bytes_with_nul(&bytes).expect("component cstr");
            let result = if name == "escape" {
                openat_directory_nofollow(root_file.as_raw_fd(), component)
            } else {
                openat_readonly_nofollow(root_file.as_raw_fd(), component)
            };
            let errno = result.expect_err("symlink must be rejected").raw_os_error();
            assert!(
                matches!(errno, Some(libc::ELOOP | libc::ENOTDIR)),
                "unexpected errno for {name}: {errno:?}"
            );
        }

        for invalid in [c"", c".", c"..", c"nested/file.txt"] {
            let err = openat_readonly_nofollow(root_file.as_raw_fd(), invalid)
                .expect_err("non-component path must be rejected");
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        }
    }
}
