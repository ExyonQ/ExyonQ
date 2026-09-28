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
//! Linux zero-copy static body delivery via sendfile(2).
#![cfg_attr(target_os = "linux", allow(dead_code))]

use super::conditional::{PreparedStaticWire, StaticValidators};
use bytes::Bytes;
#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io;
#[cfg(target_os = "linux")]
use std::os::fd::BorrowedFd;
use std::path::Path;
use std::sync::Arc;

/// sendfile only wins for larger payloads; smaller files use ordinary response bodies.
pub const SENDFILE_MIN_BYTES: usize = 65536;

#[derive(Clone, Debug)]
pub struct SendfileAsset {
    pub file: Arc<File>,
    /// Cap020 200 OK wire header (validators included). Reused when identity matches preload.
    pub header: Arc<Bytes>,
    /// Cap020 304 Not Modified wire header. Reused when identity matches preload.
    pub header_304: Arc<Bytes>,
    pub body_len: usize,
    pub content_type: &'static str,
    /// Captured at open (or reused from generation preload on identity match).
    pub validators: Arc<StaticValidators>,
}

impl SendfileAsset {
    /// Open for tests / callers without an explicit containment root.
    pub fn open(path: &Path, body_len: usize) -> io::Result<Self> {
        Self::open_under_root_prepared(path, None, body_len, None)
    }

    /// Open a regular file for sendfile under an optional Cap004 root.
    ///
    /// On Linux, when `root` is `Some`, prefer `openat2(RESOLVE_BENEATH)` from the
    /// held root directory fd (kernel-enforced containment — no `/proc/self/fd`
    /// readlink). If `openat2` is unavailable (`ENOSYS`), fall back to
    /// `open(O_NOFOLLOW)` + `/proc/self/fd` prefix check (same Cap004 guarantee).
    pub fn open_under_root(
        path: &Path,
        root: Option<OpenUnderRoot<'_>>,
        body_len: usize,
    ) -> io::Result<Self> {
        Self::open_under_root_prepared(path, root, body_len, None)
    }

    /// Cap067/S4: reuse generation-prepared Cap020 wire when opened fd identity matches.
    pub fn open_under_root_prepared(
        path: &Path,
        root: Option<OpenUnderRoot<'_>>,
        _body_len: usize,
        prepared: Option<&PreparedStaticWire>,
    ) -> io::Result<Self> {
        let (file, meta) = match root {
            Some(r) => open_regular_file_contained(path, r)?,
            None => open_regular_file_nofollow(path)?,
        };
        let opened_len = meta.len();
        let len = usize::try_from(opened_len).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "file too large for sendfile")
        })?;
        let content_type = crate::content_type_for(path);
        if let Some(prepared) = prepared {
            if prepared.identity.matches_metadata(&meta) {
                return Ok(Self {
                    file: Arc::new(file),
                    header: Arc::clone(&prepared.header_200),
                    header_304: Arc::clone(&prepared.header_304),
                    body_len: len,
                    content_type,
                    validators: Arc::clone(&prepared.validators),
                });
            }
        }
        // Identity mismatch or no preload: compute once for this open (not again in session).
        let wire = PreparedStaticWire::from_metadata(&meta, content_type);
        Ok(Self {
            file: Arc::new(file),
            header: wire.header_200,
            header_304: wire.header_304,
            body_len: len,
            content_type,
            validators: wire.validators,
        })
    }
}

/// Cap004 containment inputs for a generation-scoped static root.
#[derive(Clone, Copy, Debug)]
pub struct OpenUnderRoot<'a> {
    pub canonical_root: &'a Path,
    /// Borrowed directory fd; must not outlive the owning [`crate::StaticRoot`].
    #[cfg(target_os = "linux")]
    pub root_dir_fd: std::os::fd::BorrowedFd<'a>,
}

/// Open a regular file without following a final symlink (Cap004 TOCTOU hardening).
/// Returns `(file, metadata)` from one fstat after open.
#[cfg(target_os = "linux")]
pub(crate) fn open_regular_file_nofollow(path: &Path) -> io::Result<(File, std::fs::Metadata)> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "static sendfile target is not a regular file",
        ));
    }
    Ok((file, meta))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn open_regular_file_nofollow(path: &Path) -> io::Result<(File, std::fs::Metadata)> {
    let file = File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "static sendfile target is not a regular file",
        ));
    }
    Ok((file, meta))
}

#[cfg(target_os = "linux")]
fn open_regular_file_contained(
    path: &Path,
    root: OpenUnderRoot<'_>,
) -> io::Result<(File, std::fs::Metadata)> {
    match try_openat2_beneath(path, root) {
        Ok(pair) => Ok(pair),
        Err(Openat2Outcome::Unavailable) => {
            let (file, meta) = open_regular_file_nofollow(path)?;
            ensure_opened_under_root(&file, root.canonical_root)?;
            Ok((file, meta))
        }
        Err(Openat2Outcome::Io(err)) => Err(err),
    }
}

/// Cap067 FD-reuse: pathname identity under root dirfd without opening for serve.
///
/// Detects rename-replace / delete / recreate by `(dev,ino,len,mtime)`. Serving still
/// requires Cap004 `openat2(RESOLVE_BENEATH)` on miss. Uses the same relative path under
/// the generation root dirfd with `AT_SYMLINK_NOFOLLOW`.
#[cfg(target_os = "linux")]
pub(crate) fn relative_c_path(path: &Path, canonical_root: &Path) -> io::Result<CString> {
    use std::os::unix::ffi::OsStrExt;

    let rel = match path.strip_prefix(canonical_root) {
        Ok(r) if !r.as_os_str().is_empty() => r,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "static stat path equals root",
            ));
        }
        Err(_) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "static stat escaped configured root",
            ));
        }
    };
    if rel
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "static stat escaped configured root",
        ));
    }
    CString::new(rel.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "static path contains interior NUL",
        )
    })
}

/// Read a final-component-no-follow identity relative to a held root directory fd.
#[cfg(target_os = "linux")]
pub(crate) fn identity_from_c_str(
    rel: &CStr,
    root_fd: BorrowedFd<'_>,
) -> io::Result<crate::conditional::ValidatorIdentity> {
    use std::os::fd::AsRawFd;

    // SAFETY: all-zero is a valid initial bit pattern for libc::stat, which is
    // immediately initialized by a successful fstatat call before it is read.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: rel is NUL-terminated, root_fd is borrowed and valid for this call,
    // and st points to writable storage for one libc::stat.
    let rc = unsafe {
        libc::fstatat(
            root_fd.as_raw_fd(),
            rel.as_ptr(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if (st.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "static sendfile target is not a regular file",
        ));
    }
    Ok(crate::conditional::ValidatorIdentity {
        #[cfg(unix)]
        dev: st.st_dev,
        #[cfg(unix)]
        ino: st.st_ino,
        len: st.st_size as u64,
        mtime_secs: st.st_mtime as u64,
        mtime_nsecs: st.st_mtime_nsec as u32,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn identity_under_root(
    path: &Path,
    root: OpenUnderRoot<'_>,
) -> io::Result<crate::conditional::ValidatorIdentity> {
    let rel = relative_c_path(path, root.canonical_root)?;
    identity_from_c_str(rel.as_c_str(), root.root_dir_fd)
}

#[cfg(not(target_os = "linux"))]
fn open_regular_file_contained(
    path: &Path,
    _root: OpenUnderRoot<'_>,
) -> io::Result<(File, std::fs::Metadata)> {
    open_regular_file_nofollow(path)
}

#[cfg(target_os = "linux")]
enum Openat2Outcome {
    Unavailable,
    Io(io::Error),
}

/// 0 = unknown, 1 = available, 2 = ENOSYS / unavailable for this process.
#[cfg(target_os = "linux")]
static OPENAT2_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[cfg(target_os = "linux")]
fn try_openat2_beneath(
    path: &Path,
    root: OpenUnderRoot<'_>,
) -> Result<(File, std::fs::Metadata), Openat2Outcome> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{FromRawFd, OwnedFd};
    use std::sync::atomic::Ordering;

    if OPENAT2_STATE.load(Ordering::Relaxed) == 2 {
        return Err(Openat2Outcome::Unavailable);
    }

    let rel = match path.strip_prefix(root.canonical_root) {
        Ok(r) if !r.as_os_str().is_empty() => r,
        Ok(_) => {
            return Err(Openat2Outcome::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "static open path equals root",
            )));
        }
        Err(_) => {
            return Err(Openat2Outcome::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "static open escaped configured root",
            )));
        }
    };

    let c_path = match std::ffi::CString::new(rel.as_os_str().as_bytes()) {
        Ok(c) => c,
        Err(_) => {
            return Err(Openat2Outcome::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "static path contains interior NUL",
            )));
        }
    };

    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC) as libc::__u64;
    how.mode = 0;
    how.resolve = libc::RESOLVE_BENEATH;

    let raw = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root.root_dir_fd.as_raw_fd(),
            c_path.as_ptr(),
            &how as *const libc::open_how,
            std::mem::size_of::<libc::open_how>(),
        )
    };

    if raw < 0 {
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(e) if e == libc::ENOSYS => {
                OPENAT2_STATE.store(2, Ordering::Relaxed);
                return Err(Openat2Outcome::Unavailable);
            }
            // Kernel containment rejection (escape / .. / absolute outside).
            Some(e) if e == libc::EXDEV || e == libc::ELOOP || e == libc::ENOTDIR => {
                return Err(Openat2Outcome::Io(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "static open escaped configured root",
                )));
            }
            _ => return Err(Openat2Outcome::Io(err)),
        }
    }

    OPENAT2_STATE.store(1, Ordering::Relaxed);
    let file = File::from(unsafe { OwnedFd::from_raw_fd(raw as i32) });
    let meta = file.metadata().map_err(Openat2Outcome::Io)?;
    if !meta.is_file() {
        return Err(Openat2Outcome::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "static sendfile target is not a regular file",
        )));
    }
    Ok((file, meta))
}

/// Confirm the opened fd's kernel path remains under `canonical_root`.
///
/// Fallback only: used when `openat2` is unavailable. `O_NOFOLLOW` alone does not
/// block intermediate-directory symlink escape after a preload path-cache hit.
#[cfg(target_os = "linux")]
fn ensure_opened_under_root(file: &File, canonical_root: &Path) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    let mut resolved = std::fs::read_link(format!("/proc/self/fd/{fd}"))?;
    // Kernel may suffix deleted files: `path (deleted)`.
    if let Some(raw) = resolved.to_str() {
        if let Some(stripped) = raw.strip_suffix(" (deleted)") {
            resolved = Path::new(stripped).to_path_buf();
        }
    }
    if !resolved.starts_with(canonical_root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "static open escaped configured root",
        ));
    }
    Ok(())
}

/// Non-blocking sendfile body progress (ADR-025 Phase 2 PR #1).
/// KD2: exported for core epoll FSM until KD2.3 moves epoll_sendfile into this crate.
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub enum NbSendfileOutcome {
    /// Sent at least one byte; caller may loop (EPOLLET drain) while socket accepts writes.
    Progress,
    /// `EAGAIN`/`WouldBlock` — offset and remaining are persisted; park until `EPOLLOUT`.
    Parked,
    /// All body bytes sent for this response.
    Complete,
}

/// Optional operator clamp for sendfile request size (`EXYONQ_SENDFILE_CHUNK`).
///
/// Default: **no artificial application chunk** — each attempt requests
/// `min(remaining, platform_safe_max)`. An env value ≥ 64 KiB may clamp the
/// request for ops/testing; it must not be used as a benchmark-shaped default.
#[cfg(target_os = "linux")]
pub(crate) fn sendfile_optional_count_clamp() -> Option<usize> {
    static CLAMP: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *CLAMP.get_or_init(|| {
        std::env::var("EXYONQ_SENDFILE_CHUNK")
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|&n| n >= 65536)
    })
}

/// Largest count safe for one `sendfile64` call: remaining bytes, optionally
/// ops-clamped, and never above `isize::MAX` (ssize_t return domain).
#[cfg(target_os = "linux")]
#[inline]
pub(crate) fn sendfile_request_count(remaining: usize) -> usize {
    // sendfile64 returns ssize_t; keep count representable as a successful transfer.
    const PLATFORM_SAFE_MAX: usize = isize::MAX as usize;
    let mut n = remaining.min(PLATFORM_SAFE_MAX);
    if let Some(clamp) = sendfile_optional_count_clamp() {
        n = n.min(clamp);
    }
    n
}

/// One non-blocking `sendfile64` step. Loops `EINTR` in-place; returns `Parked` on `EAGAIN`.
/// KD2: exported for core epoll FSM until KD2.3.
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn sendfile_body_nb(
    out_fd: i32,
    in_fd: i32,
    file_offset: &mut i64,
    body_remaining: &mut usize,
) -> Result<NbSendfileOutcome, io::Error> {
    if *body_remaining == 0 {
        return Ok(NbSendfileOutcome::Complete);
    }
    loop {
        let count = sendfile_request_count(*body_remaining);
        // count is always > 0 when body_remaining > 0 (PLATFORM_SAFE_MAX >> 0).
        let sent = unsafe { libc::sendfile64(out_fd, in_fd, file_offset, count) };
        if sent < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if err.kind() == io::ErrorKind::WouldBlock {
                return Ok(NbSendfileOutcome::Parked);
            }
            return Err(err);
        }
        if sent == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "sendfile returned 0",
            ));
        }
        let sent_usize = usize::try_from(sent)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "sendfile sent negative"))?;
        if sent_usize > *body_remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sendfile sent more than remaining",
            ));
        }
        *body_remaining -= sent_usize;
        if *body_remaining == 0 {
            return Ok(NbSendfileOutcome::Complete);
        }
        // Partial kernel write: offset advanced by sendfile64; park/progress to caller.
        return Ok(NbSendfileOutcome::Progress);
    }
}

#[cfg(target_os = "linux")]
pub fn write_sendfile_head_only_fd(out_fd: i32, asset: &SendfileAsset) -> io::Result<()> {
    crate::fd_io::write_response_fd(out_fd, asset.header.as_ref())
}

#[cfg(target_os = "linux")]
pub async fn write_sendfile_head_only_try_io(
    stream: &mut tokio::net::TcpStream,
    asset: &SendfileAsset,
) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    use tokio::io::Interest;

    let out_fd = stream.as_raw_fd();
    stream.try_io(Interest::WRITABLE, || {
        write_sendfile_head_only_fd(out_fd, asset)
    })
}

#[cfg(target_os = "linux")]
pub fn write_sendfile_response_fd(out_fd: i32, asset: &SendfileAsset) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    write_sendfile_fd(
        out_fd,
        asset.file.as_raw_fd(),
        asset.header.as_ref(),
        asset.body_len,
    )
}

#[cfg(target_os = "linux")]
pub async fn write_sendfile_response_try_io(
    stream: &mut tokio::net::TcpStream,
    asset: &SendfileAsset,
) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    use tokio::io::Interest;

    let out_fd = stream.as_raw_fd();
    let in_fd = asset.file.as_raw_fd();
    let header = asset.header.as_ref();
    let mut remaining = asset.body_len;
    let mut offset: i64 = 0;

    stream.try_io(Interest::WRITABLE, || {
        crate::fd_io::write_response_fd(out_fd, header)
    })?;

    while remaining > 0 {
        let count = sendfile_request_count(remaining);
        let result = stream.try_io(Interest::WRITABLE, || {
            let sent = unsafe { libc::sendfile64(out_fd, in_fd, &mut offset, count) };
            if sent < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock {
                    return Err(err);
                }
                return Err(err);
            }
            if sent == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "sendfile returned 0",
                ));
            }
            let sent_usize = usize::try_from(sent).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "sendfile sent negative")
            })?;
            if sent_usize > remaining {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "sendfile sent more than remaining",
                ));
            }
            remaining -= sent_usize;
            Ok(())
        });
        match result {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub async fn write_sendfile_response(
    stream: tokio::net::TcpStream,
    asset: &SendfileAsset,
) -> io::Result<tokio::net::TcpStream> {
    let mut stream = stream;
    write_sendfile_response_try_io(&mut stream, asset).await?;
    Ok(stream)
}

#[cfg(not(target_os = "linux"))]
pub async fn write_sendfile_response(
    _stream: tokio::net::TcpStream,
    _asset: &SendfileAsset,
) -> io::Result<tokio::net::TcpStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "sendfile is only supported on Linux",
    ))
}

#[cfg(target_os = "linux")]
fn write_response(out_fd: i32, in_fd: i32, header: &[u8], body_len: usize) -> io::Result<()> {
    write_sendfile_fd(out_fd, in_fd, header, body_len)
}

/// Header + sendfile body on raw fds (P2/P3 blocking hot path).
/// KD2: exported for core static_conn until KD2.3.
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn write_sendfile_fd(
    out_fd: i32,
    in_fd: i32,
    header: &[u8],
    body_len: usize,
) -> io::Result<()> {
    crate::fd_io::write_response_fd(out_fd, header)?;
    sendfile_body_blocking(out_fd, in_fd, body_len)
}

/// Cap019: header + sendfile of a selected byte range (offset + length).
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn write_ranged_sendfile_fd(
    out_fd: i32,
    in_fd: i32,
    header: &[u8],
    start: u64,
    body_len: usize,
) -> io::Result<()> {
    let offset = i64::try_from(start)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "range start exceeds i64"))?;
    crate::fd_io::write_response_fd(out_fd, header)?;
    sendfile_body_blocking_from(out_fd, in_fd, offset, body_len)
}

#[cfg(target_os = "linux")]
fn sendfile_body_blocking(out_fd: i32, in_fd: i32, remaining: usize) -> io::Result<()> {
    sendfile_body_blocking_from(out_fd, in_fd, 0, remaining)
}

#[cfg(target_os = "linux")]
fn sendfile_body_blocking_from(
    out_fd: i32,
    in_fd: i32,
    mut offset: i64,
    mut remaining: usize,
) -> io::Result<()> {
    while remaining > 0 {
        let count = sendfile_request_count(remaining);
        let sent = unsafe { libc::sendfile64(out_fd, in_fd, &mut offset, count) };
        if sent < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if sent == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "sendfile returned 0",
            ));
        }
        let sent_usize = usize::try_from(sent)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "sendfile sent negative"))?;
        if sent_usize > remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sendfile sent more than remaining",
            ));
        }
        remaining -= sent_usize;
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod cause2_request_count_tests {
    use super::*;

    #[test]
    fn request_count_defaults_to_full_remaining_without_env_clamp() {
        // Unset optional clamp for this process unit (OnceLock may already be
        // populated in other tests — assert platform math independently).
        let one_mib = 1024 * 1024;
        let n = one_mib.min(isize::MAX as usize);
        assert_eq!(n, one_mib);
        // With no clamp, sendfile_request_count must equal remaining for 1 MiB.
        // If EXYONQ_SENDFILE_CHUNK is set in the environment, skip equality.
        if std::env::var("EXYONQ_SENDFILE_CHUNK").is_err() {
            assert_eq!(sendfile_request_count(one_mib), one_mib);
            assert_eq!(sendfile_request_count(4096), 4096);
        }
    }

    #[test]
    fn request_count_never_exceeds_remaining_or_ssize_max() {
        let remaining = 12345usize;
        let n = sendfile_request_count(remaining);
        assert!(n > 0);
        assert!(n <= remaining);
        assert!(n <= isize::MAX as usize);
    }

    #[test]
    fn one_mib_blocking_sendfile_completes_exact_bytes() -> io::Result<()> {
        use std::io::Read;
        use std::os::unix::io::AsRawFd;
        use std::os::unix::net::UnixStream;

        let dir = tempfile::tempdir()?;
        let path = dir.path().join("1m.bin");
        let payload = vec![0x5Au8; 1024 * 1024];
        std::fs::write(&path, &payload)?;
        let asset = SendfileAsset::open(&path, payload.len())?;
        let (writer, mut reader) = UnixStream::pair()?;
        writer.set_nonblocking(false)?;
        reader.set_nonblocking(false)?;
        // Enlarge buffers so a single sendfile can complete when kernel allows.
        unsafe {
            let fd = writer.as_raw_fd();
            let sz: libc::c_int = 2 * 1024 * 1024;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &sz as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
            let rfd = reader.as_raw_fd();
            libc::setsockopt(
                rfd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &sz as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
        let out_fd = writer.as_raw_fd();
        let in_fd = asset.file.as_raw_fd();
        write_sendfile_fd(out_fd, in_fd, asset.header.as_ref(), asset.body_len)?;
        drop(writer);
        let mut got = Vec::new();
        reader.read_to_end(&mut got)?;
        assert!(got.starts_with(b"HTTP/1.1 200"));
        let sep = got.windows(4).position(|w| w == b"\r\n\r\n").expect("hdr");
        let body = &got[sep + 4..];
        assert_eq!(body.len(), payload.len());
        assert_eq!(body, payload.as_slice());
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod identity_under_root_tests {
    use super::*;
    use std::os::fd::AsFd;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn relative_path_rejects_escape_root_and_nul() {
        let root = Path::new("/srv/exyonq-static");

        assert_eq!(
            relative_c_path(&root.join("assets/app.js"), root)
                .expect("contained path")
                .as_bytes(),
            b"assets/app.js"
        );
        assert_eq!(
            relative_c_path(Path::new("/srv/outside/app.js"), root)
                .expect_err("outside path")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            relative_c_path(&root.join("../outside/app.js"), root)
                .expect_err("lexical parent escape")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            relative_c_path(root, root)
                .expect_err("root-equal path")
                .kind(),
            io::ErrorKind::InvalidInput
        );

        let nul_path = root.join(std::ffi::OsString::from_vec(b"bad\0name".to_vec()));
        assert_eq!(
            relative_c_path(&nul_path, root)
                .expect_err("interior NUL")
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn identity_rejects_final_symlink_and_non_regular_target() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        std::fs::write(root.join("regular"), b"ok")?;
        std::os::unix::fs::symlink("regular", root.join("link"))?;
        std::fs::create_dir(root.join("directory"))?;
        let root_dir = File::open(root)?;

        assert!(identity_from_c_str(c"regular", root_dir.as_fd()).is_ok());
        assert_eq!(
            identity_from_c_str(c"link", root_dir.as_fd())
                .expect_err("final symlink")
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            identity_from_c_str(c"directory", root_dir.as_fd())
                .expect_err("directory")
                .kind(),
            io::ErrorKind::InvalidInput
        );
        Ok(())
    }

    #[test]
    fn identity_tracks_atomic_rename_drift() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        let live = root.join("asset.bin");
        std::fs::write(&live, b"inode-a")?;
        let root_dir = File::open(root)?;

        let before = identity_from_c_str(c"asset.bin", root_dir.as_fd())?;
        let replacement = root.join("replacement.bin");
        std::fs::write(&replacement, b"inode-b-content")?;
        std::fs::rename(replacement, &live)?;
        let after = identity_from_c_str(c"asset.bin", root_dir.as_fd())?;

        assert_ne!((before.dev, before.ino), (after.dev, after.ino));
        assert_eq!(after.len, b"inode-b-content".len() as u64);
        Ok(())
    }
}
