//! Small safe Unix filesystem syscall capsule for race-free control-socket ownership.

use std::ffi::CStr;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

/// Descriptor-relative metadata captured without following the final component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntryMetadata {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub uid: u32,
}

impl EntryMetadata {
    // libc file-type constants are `u16` on Darwin and `u32` on Linux.
    #[allow(clippy::useless_conversion)]
    #[must_use]
    pub fn is_directory(self) -> bool {
        self.mode & u32::from(libc::S_IFMT) == u32::from(libc::S_IFDIR)
    }

    #[allow(clippy::useless_conversion)]
    #[must_use]
    pub fn is_regular(self) -> bool {
        self.mode & u32::from(libc::S_IFMT) == u32::from(libc::S_IFREG)
    }

    #[allow(clippy::useless_conversion)]
    #[must_use]
    pub fn is_socket(self) -> bool {
        self.mode & u32::from(libc::S_IFMT) == u32::from(libc::S_IFSOCK)
    }

    #[allow(clippy::useless_conversion)]
    #[must_use]
    pub fn is_symlink(self) -> bool {
        self.mode & u32::from(libc::S_IFMT) == u32::from(libc::S_IFLNK)
    }

    #[must_use]
    pub fn permissions(self) -> u32 {
        self.mode & 0o7777
    }
}

fn validate_component(name: &CStr) -> io::Result<()> {
    let bytes = name.to_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must be one normal component",
        ));
    }
    Ok(())
}

/// Return the effective Unix user id of this process.
#[must_use]
pub fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and no memory effects visible to Rust.
    unsafe { libc::geteuid() }
}

/// Inspect one directory entry without following its final symlink.
#[allow(clippy::unnecessary_cast)] // `st_dev` / `st_mode` widths differ between Darwin and Linux.
pub fn metadata_at_nofollow(directory: BorrowedFd<'_>, name: &CStr) -> io::Result<EntryMetadata> {
    validate_component(name)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `stat` points to writable storage, `name` is NUL-terminated, the
    // borrowed descriptor remains live, and `fstatat` does not retain pointers.
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful `fstatat` initialized the complete `stat` value.
    let stat = unsafe { stat.assume_init() };
    Ok(EntryMetadata {
        dev: stat.st_dev as u64,
        ino: stat.st_ino,
        mode: stat.st_mode as u32,
        uid: stat.st_uid,
    })
}

/// Open one existing directory entry without following symlinks.
pub fn open_directory_at(directory: BorrowedFd<'_>, name: &CStr) -> io::Result<File> {
    open_at(
        directory,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0,
    )
}

/// Open or create a regular lease file without following a symlink.
pub fn open_or_create_lease_at(
    directory: BorrowedFd<'_>,
    name: &CStr,
    mode: u32,
) -> io::Result<File> {
    open_at(
        directory,
        name,
        libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        mode,
    )
}

/// Create and open a new regular file, failing if its name already exists.
pub fn create_file_exclusive_at(
    directory: BorrowedFd<'_>,
    name: &CStr,
    mode: u32,
) -> io::Result<File> {
    open_at(
        directory,
        name,
        libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        mode,
    )
}

fn open_at(
    directory: BorrowedFd<'_>,
    name: &CStr,
    flags: libc::c_int,
    mode: u32,
) -> io::Result<File> {
    validate_component(name)?;
    // SAFETY: `name` is NUL-terminated for the call, `directory` remains live,
    // and a successful `openat` returns a new descriptor owned by the caller.
    let raw = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags,
            mode as libc::c_uint,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful `openat` returned a fresh owned descriptor.
    let owned = unsafe { OwnedFd::from_raw_fd(raw) };
    Ok(File::from(owned))
}

/// Force the mode of an opened filesystem object without pathname lookup.
pub fn set_fd_mode(file: BorrowedFd<'_>, mode: u32) -> io::Result<()> {
    // SAFETY: `file` is live for the call; `fchmod` does not retain it.
    let result = unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Create one directory component relative to an opened parent.
pub fn create_directory_at(directory: BorrowedFd<'_>, name: &CStr, mode: u32) -> io::Result<()> {
    validate_component(name)?;
    // SAFETY: `name` is NUL-terminated and the borrowed descriptor remains live.
    let result =
        unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), mode as libc::mode_t) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Remove one non-directory entry relative to an opened parent.
pub fn unlink_file_at(directory: BorrowedFd<'_>, name: &CStr) -> io::Result<()> {
    unlink_at(directory, name, 0)
}

/// Remove one directory entry relative to an opened parent.
pub fn unlink_directory_at(directory: BorrowedFd<'_>, name: &CStr) -> io::Result<()> {
    unlink_at(directory, name, libc::AT_REMOVEDIR)
}

fn unlink_at(directory: BorrowedFd<'_>, name: &CStr, flags: libc::c_int) -> io::Result<()> {
    validate_component(name)?;
    // SAFETY: `name` is NUL-terminated and the borrowed descriptor remains live.
    let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), flags) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Atomically rename without replacing an occupied destination.
pub fn rename_noreplace_at(
    old_directory: BorrowedFd<'_>,
    old_name: &CStr,
    new_directory: BorrowedFd<'_>,
    new_name: &CStr,
) -> io::Result<()> {
    rename_with_flags(
        old_directory,
        old_name,
        new_directory,
        new_name,
        RenameOperation::NoReplace,
    )
}

/// Atomically exchange two existing directory entries.
pub fn rename_exchange_at(
    old_directory: BorrowedFd<'_>,
    old_name: &CStr,
    new_directory: BorrowedFd<'_>,
    new_name: &CStr,
) -> io::Result<()> {
    rename_with_flags(
        old_directory,
        old_name,
        new_directory,
        new_name,
        RenameOperation::Exchange,
    )
}

#[derive(Clone, Copy)]
enum RenameOperation {
    NoReplace,
    Exchange,
}

#[cfg(target_os = "linux")]
fn rename_with_flags(
    old_directory: BorrowedFd<'_>,
    old_name: &CStr,
    new_directory: BorrowedFd<'_>,
    new_name: &CStr,
    operation: RenameOperation,
) -> io::Result<()> {
    validate_component(old_name)?;
    validate_component(new_name)?;
    let flags = match operation {
        RenameOperation::NoReplace => libc::RENAME_NOREPLACE,
        RenameOperation::Exchange => libc::RENAME_EXCHANGE,
    };
    // SAFETY: both names are NUL-terminated, both descriptors remain live, and
    // `renameat2` retains neither pointers nor descriptors after the syscall.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            old_directory.as_raw_fd(),
            old_name.as_ptr(),
            new_directory.as_raw_fd(),
            new_name.as_ptr(),
            flags,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_with_flags(
    old_directory: BorrowedFd<'_>,
    old_name: &CStr,
    new_directory: BorrowedFd<'_>,
    new_name: &CStr,
    operation: RenameOperation,
) -> io::Result<()> {
    validate_component(old_name)?;
    validate_component(new_name)?;
    let flags = match operation {
        RenameOperation::NoReplace => libc::RENAME_EXCL,
        RenameOperation::Exchange => libc::RENAME_SWAP,
    };
    // SAFETY: both names are NUL-terminated, both descriptors remain live, and
    // `renameatx_np` retains neither pointers nor descriptors after the call.
    let result = unsafe {
        libc::renameatx_np(
            old_directory.as_raw_fd(),
            old_name.as_ptr(),
            new_directory.as_raw_fd(),
            new_name.as_ptr(),
            flags,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_with_flags(
    _old_directory: BorrowedFd<'_>,
    old_name: &CStr,
    _new_directory: BorrowedFd<'_>,
    new_name: &CStr,
    _operation: RenameOperation,
) -> io::Result<()> {
    validate_component(old_name)?;
    validate_component(new_name)?;
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace/exchange rename is unavailable on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::AsFd;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("exyonq-unix-fs-{}-{id}", std::process::id()));
            std::fs::create_dir(&path).expect("create temp directory");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn atomic_exchange_and_noreplace_are_descriptor_relative() {
        let temp = TempDir::new();
        let directory = File::open(&temp.0).expect("open temp directory");
        let mut left =
            create_file_exclusive_at(directory.as_fd(), c"left", 0o600).expect("create left");
        let mut right =
            create_file_exclusive_at(directory.as_fd(), c"right", 0o600).expect("create right");
        left.write_all(b"left").expect("write left");
        right.write_all(b"right").expect("write right");
        drop((left, right));

        let err = rename_noreplace_at(directory.as_fd(), c"left", directory.as_fd(), c"right")
            .expect_err("occupied destination must not be replaced");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);

        rename_exchange_at(directory.as_fd(), c"left", directory.as_fd(), c"right")
            .expect("atomic exchange");
        assert_eq!(std::fs::read(temp.0.join("left")).unwrap(), b"right");
        assert_eq!(std::fs::read(temp.0.join("right")).unwrap(), b"left");

        rename_noreplace_at(directory.as_fd(), c"left", directory.as_fd(), c"moved")
            .expect("no-replace move");
        assert!(!temp.0.join("left").exists());
        let metadata = metadata_at_nofollow(directory.as_fd(), c"moved").unwrap();
        assert!(metadata.is_regular());
        unlink_file_at(directory.as_fd(), c"moved").unwrap();

        let mut contents = Vec::new();
        File::open(temp.0.join("right"))
            .unwrap()
            .read_to_end(&mut contents)
            .unwrap();
        assert_eq!(contents, b"left");
    }
}
