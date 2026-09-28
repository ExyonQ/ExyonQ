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

//! Native CFD static file resolution and bounded transfer helpers.
//!
//! ## ADR-046 encoding cache — FOLLOWUP (not Cap067 remingle)
//!
//! Precompressed / static encoding cache on this plane is **FOLLOWUP** only
//! ([ADR-046](../../../../docs/adr/046-static-encoding-cache.md)). Until a CFD
//! Cap004-native coded-object open lands here:
//!
//! - Do **not** call `exyonq-mod-static` / Cap067 sendfile from this module.
//! - Product P9 gzip/brotli evidence remains Cap067-plane only.
//! - No encoding-cache API is exposed from this module until the CFD-native design lands.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use exyonq_cfd_gen::CompiledStaticPolicy;
use thiserror::Error;

const MAX_STATIC_PATH: usize = 4096;
const FALLBACK_BUF: usize = 16 * 1024;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrecompressedFollowupStatus {
    FollowupNoCap067Handoff,
}

#[cfg(test)]
const fn precompressed_followup_status() -> PrecompressedFollowupStatus {
    PrecompressedFollowupStatus::FollowupNoCap067Handoff
}

#[derive(Debug, Error)]
pub enum StaticErr {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("method not allowed")]
    MethodNotAllowed,
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug)]
pub struct OpenedStatic {
    pub file: File,
    pub len: u64,
    pub content_type: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticWrite {
    Complete,
    Pending,
}

pub fn open_static(
    policy: &CompiledStaticPolicy,
    route_prefix: &str,
    request_path: &str,
) -> Result<OpenedStatic, StaticErr> {
    let root = Path::new(&policy.document_root);
    if !root.is_absolute() {
        return Err(StaticErr::Forbidden);
    }
    let rel = resolve_relative_path(route_prefix, request_path)?;
    let wants_dir = request_path
        .split('?')
        .next()
        .unwrap_or(request_path)
        .ends_with('/');

    if wants_dir {
        return open_directory_index(root, &rel, &policy.index);
    }

    match open_confined(root, &rel)? {
        Candidate::File(opened) => Ok(opened),
        Candidate::Directory => open_directory_index(root, &rel, &policy.index),
    }
}

pub fn method_allowed(method: &str) -> Result<(), StaticErr> {
    if method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD") {
        Ok(())
    } else {
        Err(StaticErr::MethodNotAllowed)
    }
}

pub fn content_type_for(path: &Path) -> &'static str {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return "application/octet-stream";
    };
    match ext.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn send_static_body<W: Write>(
    out: &mut W,
    file: &File,
    offset: &mut u64,
    remaining: &mut u64,
) -> io::Result<StaticWrite> {
    send_static_body_pread(out, file, offset, remaining)
}

#[cfg(all(target_os = "linux", feature = "test-utils"))]
mod sendfile_test_seam {
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Once;

    /// Harness-only env (compiled out unless `test-utils` feature is enabled).
    const ENV_SENDFILE_ERRNO: &str = "EXYONQ_TEST_SENDFILE_ERRNO";
    static PENDING_ERRNO: AtomicI32 = AtomicI32::new(0);
    static ENV_ONCE: Once = Once::new();

    pub fn ensure_env_loaded() {
        ENV_ONCE.call_once(|| {
            if let Ok(raw) = std::env::var(ENV_SENDFILE_ERRNO) {
                if let Ok(errno) = raw.parse::<i32>() {
                    PENDING_ERRNO.store(errno, Ordering::SeqCst);
                }
            }
        });
    }

    /// One-shot override consumed on the next `linux_sendfile` call in this process.
    pub fn take_sendfile_errno_override() -> Option<i32> {
        ensure_env_loaded();
        let v = PENDING_ERRNO.swap(0, Ordering::SeqCst);
        if v == 0 {
            None
        } else {
            Some(v)
        }
    }

    #[cfg(test)]
    pub fn set_next_sendfile_errno(errno: i32) {
        PENDING_ERRNO.store(errno, Ordering::SeqCst);
    }
}

#[cfg(target_os = "linux")]
fn linux_sendfile(out_fd: i32, in_fd: i32, off: &mut libc::off_t, count: usize) -> isize {
    #[cfg(feature = "test-utils")]
    if let Some(errno) = sendfile_test_seam::take_sendfile_errno_override() {
        // SAFETY: POSIX thread-local errno location for this thread.
        unsafe {
            *libc::__errno_location() = errno;
        }
        return -1;
    }
    // SAFETY: caller guarantees valid fds and `off` for the duration of the call.
    unsafe { libc::sendfile(out_fd, in_fd, off, count) }
}

#[cfg(target_os = "linux")]
pub fn send_static_body_sendfile<W: Write + std::os::fd::AsRawFd>(
    out: &mut W,
    file: &File,
    offset: &mut u64,
    remaining: &mut u64,
) -> io::Result<StaticWrite> {
    use std::os::fd::AsRawFd;

    while *remaining > 0 {
        let mut off = libc::off_t::try_from(*offset)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "offset overflow"))?;
        let count = usize::try_from((*remaining).min(usize::MAX as u64)).unwrap_or(usize::MAX);
        let n = linux_sendfile(out.as_raw_fd(), file.as_raw_fd(), &mut off, count);
        if n > 0 {
            let wrote = u64::try_from(n).unwrap_or(0);
            *offset = (*offset).saturating_add(wrote);
            *remaining = (*remaining).saturating_sub(wrote);
            continue;
        }
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "static file truncated during sendfile",
            ));
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EAGAIN) => return Ok(StaticWrite::Pending),
            Some(libc::EINTR) => continue,
            Some(libc::EINVAL) | Some(libc::ENOSYS) => {
                return send_static_body_pread(out, file, offset, remaining);
            }
            _ => return Err(e),
        }
    }
    Ok(StaticWrite::Complete)
}

#[cfg(unix)]
fn send_static_body_pread<W: Write>(
    out: &mut W,
    file: &File,
    offset: &mut u64,
    remaining: &mut u64,
) -> io::Result<StaticWrite> {
    use std::os::unix::fs::FileExt;

    let mut buf = [0u8; FALLBACK_BUF];
    while *remaining > 0 {
        let want = usize::try_from((*remaining).min(buf.len() as u64)).unwrap_or(buf.len());
        let n = file.read_at(&mut buf[..want], *offset)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "static file truncated during pread",
            ));
        }
        let mut written = 0usize;
        while written < n {
            match out.write(&buf[written..n]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "write zero")),
                Ok(m) => {
                    written += m;
                    let delta = u64::try_from(m).unwrap_or(0);
                    *offset = (*offset).saturating_add(delta);
                    *remaining = (*remaining).saturating_sub(delta);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(StaticWrite::Pending),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }
    Ok(StaticWrite::Complete)
}

#[cfg(not(unix))]
fn send_static_body_pread<W: Write>(
    _out: &mut W,
    _file: &File,
    _offset: &mut u64,
    _remaining: &mut u64,
) -> io::Result<StaticWrite> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "static file transfer unsupported on this platform",
    ))
}

enum Candidate {
    File(OpenedStatic),
    Directory,
}

fn open_directory_index(
    root: &Path,
    dir_rel: &Path,
    indexes: &[String],
) -> Result<OpenedStatic, StaticErr> {
    if indexes.is_empty() {
        return Err(StaticErr::NotFound);
    }
    for index in indexes {
        if !valid_index_candidate(index) {
            return Err(StaticErr::Forbidden);
        }
        let rel = dir_rel.join(index);
        match open_confined(root, &rel) {
            Ok(Candidate::File(opened)) => return Ok(opened),
            Ok(Candidate::Directory) | Err(StaticErr::NotFound) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(StaticErr::NotFound)
}

fn open_confined(root: &Path, rel: &Path) -> Result<Candidate, StaticErr> {
    validate_confined_relative_path(rel)?;

    #[cfg(target_os = "linux")]
    {
        open_confined_linux(root, rel)
    }

    #[cfg(not(target_os = "linux"))]
    {
        open_confined_non_linux(root, rel)
    }
}

fn validate_confined_relative_path(rel: &Path) -> Result<(), StaticErr> {
    if rel.as_os_str().as_encoded_bytes().len() > MAX_STATIC_PATH {
        return Err(StaticErr::Forbidden);
    }

    let mut components = 0usize;
    for component in rel.components() {
        let Component::Normal(name) = component else {
            return Err(StaticErr::Forbidden);
        };
        if name.as_encoded_bytes().contains(&0) {
            return Err(StaticErr::Forbidden);
        }
        components += 1;
    }

    if components == 0 {
        return Err(StaticErr::Forbidden);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_confined_linux(root: &Path, rel: &Path) -> Result<Candidate, StaticErr> {
    open_confined_linux_with_openat2(root, rel, true)
}

#[cfg(target_os = "linux")]
fn open_confined_linux_with_openat2(
    root: &Path,
    rel: &Path,
    try_openat2: bool,
) -> Result<Candidate, StaticErr> {
    let root_file = open_canonical_root(root)?;
    if try_openat2 {
        if let Some(file) = openat2_confined(&root_file, rel)? {
            return classify_opened(file, rel);
        }
    }
    let file = openat_walk_confined(&root_file, rel, |_| {})?;
    classify_opened(file, rel)
}

#[cfg(target_os = "linux")]
fn open_canonical_root(root: &Path) -> Result<File, StaticErr> {
    let root_canon = root.canonicalize().map_err(map_open_err)?;
    let mut options = OpenOptions::new();
    options.read(true);
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    options.open(root_canon).map_err(map_open_err)
}

#[cfg(target_os = "linux")]
fn openat2_confined(root_file: &File, rel: &Path) -> Result<Option<File>, StaticErr> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let rel_bytes = rel.as_os_str().as_bytes();
    let rel_c = std::ffi::CString::new(rel_bytes).map_err(|_| StaticErr::Forbidden)?;
    let flags = (libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK) as u64;
    let resolve = libc::RESOLVE_BENEATH | libc::RESOLVE_NO_MAGICLINKS | libc::RESOLVE_NO_SYMLINKS;
    match exyonq_linux_ffi::openat2_beneath(root_file.as_raw_fd(), &rel_c, flags, resolve) {
        Ok(file) => Ok(Some(file)),
        Err(exyonq_linux_ffi::Openat2Error::Unavailable) => Ok(None),
        Err(exyonq_linux_ffi::Openat2Error::Io(e)) => match e.raw_os_error() {
            Some(libc::EINVAL) => Ok(None),
            _ => Err(map_open_err(e)),
        },
    }
}

#[cfg(target_os = "linux")]
fn openat_walk_confined(
    root_file: &File,
    rel: &Path,
    mut after_intermediate: impl FnMut(usize),
) -> Result<File, StaticErr> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let mut current_dir = root_file.try_clone().map_err(map_open_err)?;
    let mut components = rel.components().peekable();
    let mut depth = 0usize;

    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(StaticErr::Forbidden);
        };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| StaticErr::Forbidden)?;
        if components.peek().is_none() {
            return exyonq_linux_ffi::openat_readonly_nofollow(current_dir.as_raw_fd(), &name)
                .map_err(map_open_err);
        }

        current_dir = exyonq_linux_ffi::openat_directory_nofollow(current_dir.as_raw_fd(), &name)
            .map_err(map_open_err)?;
        depth += 1;
        after_intermediate(depth);
    }

    Err(StaticErr::Forbidden)
}

#[cfg(not(target_os = "linux"))]
fn open_confined_non_linux(root: &Path, rel: &Path) -> Result<Candidate, StaticErr> {
    // Non-Linux has no openat2 proof here. Preserve the explicit legacy
    // canonicalization behavior without presenting it as Linux-equivalent evidence.
    let root_canon = root.canonicalize().map_err(map_open_err)?;
    let joined = root_canon.join(rel);
    let final_canon = joined.canonicalize().map_err(map_open_err)?;
    if !final_canon.starts_with(&root_canon) {
        return Err(StaticErr::Forbidden);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(&final_canon).map_err(map_open_err)?;
    classify_opened(file, &final_canon)
}

fn classify_opened(file: File, rel_or_path: &Path) -> Result<Candidate, StaticErr> {
    let meta = file.metadata().map_err(map_open_err)?;
    if meta.is_dir() {
        return Ok(Candidate::Directory);
    }
    if !meta.is_file() {
        return Err(StaticErr::Forbidden);
    }
    Ok(Candidate::File(OpenedStatic {
        file,
        len: meta.len(),
        content_type: content_type_for(rel_or_path),
    }))
}

fn resolve_relative_path(route_prefix: &str, request_path: &str) -> Result<PathBuf, StaticErr> {
    let path = request_path.split('?').next().unwrap_or(request_path);
    if path.is_empty() || path.len() > MAX_STATIC_PATH || path.contains('\0') {
        return Err(StaticErr::Forbidden);
    }
    let prefix = route_prefix.trim_end_matches('/');
    let rel_uri = if prefix.is_empty() || prefix == "/" {
        path.trim_start_matches('/')
    } else if path == prefix {
        ""
    } else if let Some(rest) = path.strip_prefix(prefix).and_then(|s| s.strip_prefix('/')) {
        rest
    } else {
        return Err(StaticErr::NotFound);
    };
    let decoded = percent_decode_once(rel_uri)?;
    if decoded.len() > MAX_STATIC_PATH || decoded.contains('%') {
        return Err(StaticErr::Forbidden);
    }
    let mut rel = PathBuf::new();
    for segment in decoded.split('/') {
        if segment.is_empty() {
            continue;
        }
        if invalid_segment(segment) {
            return Err(StaticErr::Forbidden);
        }
        rel.push(segment);
    }
    Ok(rel)
}

fn invalid_segment(segment: &str) -> bool {
    segment == "."
        || segment == ".."
        || segment.starts_with('.')
        || segment.contains('\0')
        || segment.contains('\\')
        || segment.to_ascii_lowercase().ends_with(".php")
}

fn valid_index_candidate(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.starts_with('/')
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !name.contains('%')
        && !name.contains("..")
        && !name.to_ascii_lowercase().ends_with(".php")
}

fn percent_decode_once(input: &str) -> Result<String, StaticErr> {
    if !input.as_bytes().contains(&b'%') {
        return Ok(input.to_string());
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        if i + 2 >= bytes.len() {
            return Err(StaticErr::Forbidden);
        }
        let hi = hex_val(bytes[i + 1]).ok_or(StaticErr::Forbidden)?;
        let lo = hex_val(bytes[i + 2]).ok_or(StaticErr::Forbidden)?;
        out.push((hi << 4) | lo);
        i += 3;
    }
    String::from_utf8(out).map_err(|_| StaticErr::Forbidden)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn map_open_err(e: io::Error) -> StaticErr {
    match e.kind() {
        io::ErrorKind::NotFound => StaticErr::NotFound,
        io::ErrorKind::PermissionDenied => StaticErr::Forbidden,
        _ => match e.raw_os_error() {
            #[cfg(unix)]
            // openat2(RESOLVE_BENEATH) uses EXDEV when a symlink escapes the root;
            // ELOOP/EPERM cover cyclic or denied resolution. Never surface these as 500.
            Some(libc::ENOTDIR) | Some(libc::ELOOP) | Some(libc::EXDEV) | Some(libc::EPERM) => {
                StaticErr::Forbidden
            }
            // FIFO write-only without reader, unix socket nodes, and other non-regular opens.
            Some(libc::ENXIO) => StaticErr::Forbidden,
            _ => StaticErr::Io(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::fs;
    use tempfile::tempdir;

    fn policy(root: &Path) -> CompiledStaticPolicy {
        CompiledStaticPolicy {
            id: 1,
            document_root: root.to_string_lossy().into_owned(),
            flags: 0,
            index: vec!["index.html".into()],
        }
    }

    #[test]
    fn precompressed_followup_is_explicit_no_cap067_handoff() {
        assert_eq!(
            precompressed_followup_status(),
            PrecompressedFollowupStatus::FollowupNoCap067Handoff
        );
    }

    #[test]
    fn opens_regular_file_and_mime() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("app.css"), b"body{}").unwrap();

        let opened = open_static(&policy(dir.path()), "/static", "/static/app.css").unwrap();

        assert_eq!(opened.len, 6);
        assert_eq!(opened.content_type, "text/css; charset=utf-8");
    }

    #[test]
    fn serves_directory_index_without_autoindex() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("docs/index.html"), b"hi").unwrap();

        let opened = open_static(&policy(dir.path()), "/static", "/static/docs/").unwrap();
        let missing = open_static(&policy(dir.path()), "/static", "/static/");

        assert_eq!(opened.len, 2);
        assert!(matches!(missing, Err(StaticErr::NotFound)));
    }

    #[test]
    fn rejects_traversal_dotfiles_php_and_percent_weirdness() {
        let dir = tempdir().unwrap();
        for path in [
            "/static/../secret.txt",
            "/static/%2e%2e/secret.txt",
            "/static/.env",
            "/static/index.PHP",
            "/static/%252e%252e/secret.txt",
        ] {
            let err = open_static(&policy(dir.path()), "/static", path).unwrap_err();
            assert!(
                matches!(err, StaticErr::Forbidden),
                "path={path} err={err:?}"
            );
        }
    }

    #[test]
    fn rejects_fifo_or_missing() {
        let dir = tempdir().unwrap();
        let missing = open_static(&policy(dir.path()), "/static", "/static/missing.txt");
        assert!(matches!(missing, Err(StaticErr::NotFound)));

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let fifo_path = dir.path().join("pipe");
            let c = CString::new(fifo_path.as_os_str().as_bytes()).unwrap();
            // SAFETY: path is a valid NUL-terminated test path; mode is a standard FIFO mode.
            let rc = unsafe { libc::mkfifo(c.as_ptr(), 0o600) };
            assert_eq!(rc, 0);
            let err = open_static(&policy(dir.path()), "/static", "/static/pipe").unwrap_err();
            assert!(matches!(err, StaticErr::Forbidden));
        }
    }

    #[test]
    fn rejects_symlink_escape() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), b"secret").unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
            let err =
                open_static(&policy(dir.path()), "/static", "/static/out/secret.txt").unwrap_err();
            assert!(matches!(err, StaticErr::Forbidden));
        }
    }

    #[cfg(target_os = "linux")]
    fn force_linux_fallback(root: &Path, rel: &Path) -> Result<Candidate, StaticErr> {
        validate_confined_relative_path(rel)?;
        open_confined_linux_with_openat2(root, rel, false)
    }

    #[cfg(target_os = "linux")]
    fn read_candidate(candidate: Candidate) -> Vec<u8> {
        use std::io::Read;

        let Candidate::File(mut opened) = candidate else {
            panic!("expected regular file");
        };
        let mut bytes = Vec::new();
        opened.file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn forced_fallback_opens_nested_regular_file() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("a/b")).unwrap();
        fs::write(root.path().join("a/b/file.txt"), b"nested").unwrap();

        let candidate = force_linux_fallback(root.path(), Path::new("a/b/file.txt")).unwrap();

        assert_eq!(read_candidate(candidate), b"nested");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn forced_fallback_rejects_intermediate_and_final_symlinks() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), b"outside").unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        fs::write(root.path().join("real/file.txt"), b"inside").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            root.path().join("final-link"),
        )
        .unwrap();

        for rel in ["escape/secret.txt", "final-link"] {
            let err = match force_linux_fallback(root.path(), Path::new(rel)) {
                Err(err) => err,
                Ok(_) => panic!("symlink unexpectedly opened: {rel}"),
            };
            assert!(matches!(err, StaticErr::Forbidden), "rel={rel} err={err:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn forced_fallback_rejects_invalid_paths_and_nonregular_targets() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::net::UnixListener;

        let root = tempdir().unwrap();
        fs::write(root.path().join("plain"), b"file").unwrap();
        let _socket = UnixListener::bind(root.path().join("socket")).unwrap();

        for rel in [
            Path::new(""),
            Path::new("../plain"),
            Path::new("./plain"),
            Path::new("/plain"),
            Path::new("plain/child"),
            Path::new("socket"),
        ] {
            let result = validate_confined_relative_path(rel)
                .and_then(|()| force_linux_fallback(root.path(), rel));
            assert!(
                matches!(result, Err(StaticErr::Forbidden)),
                "path unexpectedly accepted: {rel:?}"
            );
        }

        let nul = Path::new(std::ffi::OsStr::from_bytes(b"bad\0name"));
        assert!(matches!(
            validate_confined_relative_path(nul),
            Err(StaticErr::Forbidden)
        ));
        assert!(matches!(
            open_static(&policy(root.path()), "/static", "/other/plain"),
            Err(StaticErr::NotFound)
        ));
        assert!(matches!(
            open_static(&policy(root.path()), "/static", "/static"),
            Err(StaticErr::Forbidden)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn forced_fallback_rename_swap_loop_never_opens_outside_file() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("value.txt"), b"outside").unwrap();

        for iteration in 0..128 {
            let live = root.path().join("live");
            let parked = root.path().join("parked");
            fs::create_dir(&live).unwrap();
            fs::write(live.join("value.txt"), b"inside").unwrap();
            let root_file = open_canonical_root(root.path()).unwrap();
            let mut swapped = false;

            let file = openat_walk_confined(&root_file, Path::new("live/value.txt"), |depth| {
                if depth == 1 {
                    fs::rename(&live, &parked).unwrap();
                    std::os::unix::fs::symlink(outside.path(), &live).unwrap();
                    swapped = true;
                }
            })
            .unwrap();
            let candidate = classify_opened(file, Path::new("live/value.txt")).unwrap();

            assert!(swapped, "iteration={iteration}");
            assert_eq!(
                read_candidate(candidate),
                b"inside",
                "iteration={iteration}"
            );
            fs::remove_file(&live).unwrap();
            fs::remove_dir_all(&parked).unwrap();
        }
    }

    #[test]
    fn bounded_pread_sender_streams_bytes() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"hello world").unwrap();
        let opened = open_static(&policy(dir.path()), "/static", "/static/file.txt").unwrap();
        let mut off = 0u64;
        let mut remaining = opened.len;
        let mut out = Vec::new();

        let state = send_static_body(&mut out, &opened.file, &mut off, &mut remaining).unwrap();

        assert_eq!(state, StaticWrite::Complete);
        assert_eq!(out, b"hello world");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn truncated_file_during_send_is_not_complete() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("shrinks.txt");
        fs::write(&path, b"abcdef").unwrap();
        let opened = open_static(&policy(dir.path()), "/static", "/static/shrinks.txt").unwrap();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(2)
            .unwrap();
        let mut off = 0u64;
        let mut remaining = opened.len;
        let mut out = Vec::new();

        let err = send_static_body(&mut out, &opened.file, &mut off, &mut remaining).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(remaining > 0);
    }

    #[cfg(all(target_os = "linux", feature = "test-utils"))]
    #[test]
    fn sendfile_einval_uses_same_fd_pread_fallback() {
        use std::io::Read;
        use std::net::TcpListener;
        use std::thread;

        let dir = tempdir().unwrap();
        let payload = b"pread-fallback-oracle-payload-v1";
        fs::write(dir.path().join("probe.bin"), payload).unwrap();
        let opened = open_static(&policy(dir.path()), "/static", "/static/probe.bin").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let reader = thread::spawn(move || {
            let mut client = std::net::TcpStream::connect(addr).unwrap();
            let mut buf = Vec::new();
            client.read_to_end(&mut buf).unwrap();
            buf
        });
        let (mut server, _) = listener.accept().unwrap();
        sendfile_test_seam::set_next_sendfile_errno(libc::EINVAL);
        let mut off = 0u64;
        let mut remaining = opened.len;
        let state =
            send_static_body_sendfile(&mut server, &opened.file, &mut off, &mut remaining).unwrap();
        assert_eq!(state, StaticWrite::Complete);
        assert_eq!(remaining, 0);
        assert_eq!(off, opened.len);
        drop(server);
        assert_eq!(reader.join().unwrap(), payload);
    }

    #[cfg(all(target_os = "linux", feature = "test-utils"))]
    #[test]
    fn sendfile_ineligible_errno_does_not_fallback_to_pread() {
        use std::net::TcpListener;
        use std::thread;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join("probe.bin"), b"x").unwrap();
        let opened = open_static(&policy(dir.path()), "/static", "/static/probe.bin").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let _connector = thread::spawn(move || {
            let _ = std::net::TcpStream::connect(addr);
        });
        thread::sleep(std::time::Duration::from_millis(50));
        let (mut server, _) = listener.accept().unwrap();
        sendfile_test_seam::set_next_sendfile_errno(libc::EIO);
        let mut off = 0u64;
        let mut remaining = opened.len;
        let err = send_static_body_sendfile(&mut server, &opened.file, &mut off, &mut remaining)
            .unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EIO));
        assert_eq!(remaining, opened.len);
    }
}
