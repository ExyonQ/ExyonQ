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
//! Cap004: outside-root symlink must not enter preload cache or resolve live.

use exyonq_mod_static::{resolve_path, PreloadLimits, StaticError, StaticRoot};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use tempfile::tempdir;

#[cfg(unix)]
#[test]
fn preload_skips_symlink_escape_and_live_resolve_rejects() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let outside = dir.path().join("secret.txt");
    fs::create_dir_all(root.join("safe")).expect("mkdir");
    fs::write(root.join("ok.txt"), b"ok").expect("ok");
    fs::write(&outside, b"SECRET-OUTSIDE").expect("outside");
    symlink(&outside, root.join("safe/escape.link")).expect("symlink");

    let mut service = StaticRoot::new_with_preload_limits(
        &root,
        "/assets",
        Some("index.html"),
        PreloadLimits::DEFAULT,
    )
    .expect("root");
    service.preload_tree().expect("preload");
    // Cache must not serve outside bytes via serve_request.
    assert!(matches!(
        service.serve_request("/assets/safe/escape.link"),
        Err(StaticError::NotFound)
    ));
    let err = service
        .resolve_live_file_path("/assets/safe/escape.link")
        .expect_err("live resolve must reject");
    assert!(matches!(err, StaticError::PathTraversal));

    // In-root file still works.
    let path = resolve_path(&root, "/assets", "/assets/ok.txt", None).expect("ok path");
    assert!(path.ends_with("ok.txt"));
    assert!(service.serve_request("/assets/ok.txt").is_ok());
}

#[cfg(unix)]
#[test]
fn resolve_path_rejects_symlink_escape() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let outside = dir.path().join("secret.txt");
    fs::create_dir_all(root.join("safe")).expect("mkdir");
    fs::write(&outside, b"SECRET").expect("outside");
    symlink(&outside, root.join("safe/escape.link")).expect("symlink");
    let err = resolve_path(
        &root,
        "/assets",
        "/assets/safe/escape.link",
        Some("index.html"),
    )
    .expect_err("must reject");
    assert!(matches!(err, StaticError::PathTraversal));
}

#[cfg(unix)]
#[test]
fn route_table_symlink_escape_not_preloaded() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let outside = dir.path().join("secret.txt");
    fs::create_dir_all(root.join("routes")).expect("mkdir");
    fs::write(&outside, b"SECRET-ROUTE-TABLE").expect("outside");
    symlink(&outside, root.join("routes/route000.bin")).expect("symlink");
    fs::write(root.join("ok.txt"), b"ok").expect("ok");

    let mut service = StaticRoot::new_with_preload_limits(
        &root,
        "/assets",
        Some("index.html"),
        PreloadLimits::DEFAULT,
    )
    .expect("root");
    service.preload_tree().expect("preload");
    assert!(matches!(
        service.serve_request("/assets/routes/route000.bin"),
        Err(StaticError::NotFound)
    ));
    assert!(matches!(
        service.serve_request("/assets/routes/route042.bin"),
        Err(StaticError::NotFound)
    ));
    let err = service
        .resolve_live_file_path("/assets/routes/route000.bin")
        .expect_err("live resolve must reject");
    assert!(matches!(err, StaticError::PathTraversal));
}

#[cfg(unix)]
#[test]
fn sixty_four_k_symlink_escape_not_served_from_cache() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let outside = dir.path().join("secret64.txt");
    fs::create_dir_all(&root).expect("mkdir");
    fs::write(&outside, vec![0x41u8; 65536]).expect("outside");
    symlink(&outside, root.join("64k.bin")).expect("symlink");

    let mut service = StaticRoot::new_with_preload_limits(
        &root,
        "/assets",
        Some("index.html"),
        PreloadLimits::DEFAULT,
    )
    .expect("root");
    service.preload_tree().expect("preload");
    match service.serve_request("/assets/64k.bin") {
        Err(StaticError::NotFound) => {}
        Ok(resp) => assert_ne!(resp.status(), hyper::StatusCode::OK),
        Err(other) => panic!("unexpected err {other:?}"),
    }
    let err = service
        .resolve_live_file_path("/assets/64k.bin")
        .expect_err("live resolve must reject");
    assert!(matches!(err, StaticError::PathTraversal));
}
