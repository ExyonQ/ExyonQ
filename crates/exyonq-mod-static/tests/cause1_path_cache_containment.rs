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
//! CAUSE_1: generation-scoped preload path reuse must not weaken Cap004 containment.

#[cfg(target_os = "linux")]
use exyonq_mod_static::sendfile::SendfileAsset;
use exyonq_mod_static::{PreloadLimits, StaticRoot};
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::fs::symlink;
use tempfile::tempdir;

#[test]
fn preload_hit_returns_cached_path_without_requiring_uncached_walk() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    fs::create_dir_all(root.join("nested")).expect("mkdir");
    fs::write(root.join("nested/a.bin"), b"hello").expect("write");

    let mut service =
        StaticRoot::new_with_preload_limits(&root, "/site", None, PreloadLimits::DEFAULT)
            .expect("root");
    service.preload_tree().expect("preload");
    assert!(service.cache_len() >= 1);

    let path = service
        .resolved_file_path("/site/nested/a.bin")
        .expect("cached path");
    assert!(path.ends_with("a.bin"));
    assert!(path.starts_with(service.canonical_root()));
    // Live Hyper path stays Cap004-uncached (may still succeed while file exists).
    let live = service
        .resolve_live_file_path("/site/nested/a.bin")
        .expect("live path");
    assert_eq!(live, path);
}

#[cfg(target_os = "linux")]
#[test]
fn open_under_root_rejects_intermediate_symlink_escape_after_preload() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let outside = dir.path().join("outside");
    fs::create_dir_all(root.join("nested")).expect("mkdir");
    fs::create_dir_all(&outside).expect("mkdir outside");
    fs::write(root.join("nested/a.bin"), b"inside").expect("write inside");
    fs::write(outside.join("a.bin"), b"SECRET").expect("write outside");

    let mut service =
        StaticRoot::new_with_preload_limits(&root, "/site", None, PreloadLimits::DEFAULT)
            .expect("root");
    service.preload_tree().expect("preload");
    let cached = service
        .resolved_file_path("/site/nested/a.bin")
        .expect("preload hit path");

    // Swap parent directory for an escaping symlink (classic TOCTOU vs path cache).
    fs::remove_dir_all(root.join("nested")).expect("rm nested");
    symlink(&outside, root.join("nested")).expect("escape symlink");

    let err = SendfileAsset::open_under_root(
        &cached,
        Some(exyonq_mod_static::sendfile::OpenUnderRoot {
            canonical_root: service.canonical_root(),
            root_dir_fd: service.root_dir_fd(),
        }),
        0,
    )
    .expect_err("must reject escaped open");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);

    // Hyper / live surface must not use the stale PathBuf — uncached Cap004 reject.
    let live_err = service
        .resolve_live_file_path("/site/nested/a.bin")
        .expect_err("live resolve must reject escape");
    assert!(matches!(
        live_err,
        exyonq_mod_static::StaticError::PathTraversal
            | exyonq_mod_static::StaticError::NotFound
            | exyonq_mod_static::StaticError::Io(_)
    ));
}

#[cfg(unix)]
#[test]
fn missing_and_traversal_still_fail() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    fs::create_dir_all(&root).expect("mkdir");
    fs::write(root.join("ok.txt"), b"ok").expect("ok");

    let mut service =
        StaticRoot::new_with_preload_limits(&root, "/site", None, PreloadLimits::DEFAULT)
            .expect("root");
    service.preload_tree().expect("preload");

    assert!(service.resolve_live_file_path("/site/missing.txt").is_err());
    assert!(service
        .resolve_live_file_path("/site/../outside.txt")
        .is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn o_nofollow_rejects_directory_symlink_after_canonicalize_path() {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&root).expect("mkdir root");
    fs::create_dir_all(&outside).expect("mkdir outside");
    let canon = root.canonicalize().expect("canon");
    // Race window: replace the canonicalized path with a symlink to an attacker tree.
    fs::remove_dir(&root).expect("rm root");
    symlink(&outside, &root).expect("symlink swap");
    let err = std::fs::File::options()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&canon)
        .expect_err("O_NOFOLLOW must reject final symlink at canonical path");
    // O_NOFOLLOW must reject the swapped final symlink. Observed errno varies by
    // kernel/path race (ELOOP vs ENOTDIR). Do not use ErrorKind::FilesystemLoop —
    // it remains feature-gated (`io_error_more`) on Rust 1.97.1 stable.
    assert!(
        err.raw_os_error() == Some(libc::ELOOP)
            || err.raw_os_error() == Some(libc::ENOTDIR)
            || err.kind() == std::io::ErrorKind::NotADirectory
            || err.kind() == std::io::ErrorKind::Other,
        "unexpected err {err:?}"
    );
}
