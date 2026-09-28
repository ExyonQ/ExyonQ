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
//! S4: generation-prepared Cap020 wire reuse must key on opened file identity,
//! not path alone — no stale 304 after replacement.

use exyonq_mod_static::sendfile::SendfileAsset;
use exyonq_mod_static::{
    decide_conditional, ConditionalDecision, PreloadLimits, StaticRoot, StaticValidators,
};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

fn preload_root(root: &Path) -> StaticRoot {
    let mut service =
        StaticRoot::new_with_preload_limits(root, "/site", None, PreloadLimits::DEFAULT)
            .expect("root");
    service.preload_tree().expect("preload");
    service
}

fn open_prepared(service: &StaticRoot, request_path: &str) -> SendfileAsset {
    let file_path = service
        .resolved_file_path(request_path)
        .expect("resolved path");
    let prepared = service.prepared_wire(request_path);
    #[cfg(target_os = "linux")]
    {
        SendfileAsset::open_under_root_prepared(
            &file_path,
            Some(exyonq_mod_static::sendfile::OpenUnderRoot {
                canonical_root: service.canonical_root(),
                root_dir_fd: service.root_dir_fd(),
            }),
            0,
            prepared,
        )
        .expect("open prepared")
    }
    #[cfg(not(target_os = "linux"))]
    {
        SendfileAsset::open_under_root_prepared(&file_path, None, 0, prepared).expect("open")
    }
}

#[test]
fn unchanged_file_reuses_prepared_validators_and_headers() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    fs::create_dir_all(&root).expect("mkdir");
    fs::write(root.join("a.bin"), vec![0u8; 128]).expect("write");

    let service = preload_root(&root);
    let prepared = service.prepared_wire("/site/a.bin").expect("prepared");
    let asset = open_prepared(&service, "/site/a.bin");

    assert!(
        Arc::ptr_eq(&asset.validators, &prepared.validators),
        "validators Arc must be reused on identity match"
    );
    assert!(
        Arc::ptr_eq(&asset.header, &prepared.header_200),
        "200 header Arc must be reused on identity match"
    );
    assert!(
        Arc::ptr_eq(&asset.header_304, &prepared.header_304),
        "304 header Arc must be reused on identity match"
    );
    assert_eq!(
        decide_conditional(Some(&asset.validators.etag), None, &asset.validators),
        ConditionalDecision::NotModified
    );
}

#[test]
fn in_place_replace_invalidates_prepared_wire() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    fs::create_dir_all(&root).expect("mkdir");
    let path = root.join("a.bin");
    fs::write(&path, b"AAAA").expect("write");

    let service = preload_root(&root);
    let prepared = service.prepared_wire("/site/a.bin").expect("prepared");
    let old_etag = prepared.validators.etag.clone();

    // Ensure mtime advances (identity includes mtime nsecs).
    thread::sleep(Duration::from_millis(20));
    fs::write(&path, b"BBBB").expect("replace in place");

    let asset = open_prepared(&service, "/site/a.bin");
    assert!(
        !Arc::ptr_eq(&asset.validators, &prepared.validators),
        "must not reuse validators after in-place replace"
    );
    assert_ne!(asset.validators.etag, old_etag);
    assert_eq!(
        decide_conditional(Some(&old_etag), None, &asset.validators),
        ConditionalDecision::Continue,
        "stale INM must not 304 after replace"
    );
    assert_eq!(
        decide_conditional(Some(&asset.validators.etag), None, &asset.validators),
        ConditionalDecision::NotModified
    );
}

#[test]
fn atomic_rename_replacement_invalidates_prepared_wire() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    fs::create_dir_all(&root).expect("mkdir");
    let path = root.join("a.bin");
    fs::write(&path, b"old-content").expect("write");

    let service = preload_root(&root);
    let prepared = service.prepared_wire("/site/a.bin").expect("prepared");
    let old_etag = prepared.validators.etag.clone();
    let old_lm = prepared.validators.last_modified.clone();
    let old_mtime_secs = prepared.validators.mtime_secs;

    // Cross a whole-second boundary so IMS (second granularity) observes the change.
    thread::sleep(Duration::from_millis(1100));
    let tmp = root.join("a.bin.tmp");
    fs::write(&tmp, b"new-content!!").expect("write tmp");
    fs::rename(&tmp, &path).expect("atomic rename");

    let asset = open_prepared(&service, "/site/a.bin");
    assert!(!Arc::ptr_eq(&asset.validators, &prepared.validators));
    assert_ne!(asset.validators.etag, old_etag);
    assert_eq!(
        decide_conditional(Some(&old_etag), None, &asset.validators),
        ConditionalDecision::Continue,
        "stale INM must not 304 after atomic rename"
    );
    if let (Some(lm), Some(old_secs)) = (old_lm.as_deref(), old_mtime_secs) {
        if asset.validators.mtime_secs != Some(old_secs) {
            assert_eq!(
                decide_conditional(None, Some(lm), &asset.validators),
                ConditionalDecision::Continue,
                "stale IMS must not 304 when mtime second changed"
            );
        }
    }
}

#[test]
fn size_change_invalidates_etag() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    fs::create_dir_all(&root).expect("mkdir");
    let path = root.join("a.bin");
    fs::write(&path, b"short").expect("write");

    let service = preload_root(&root);
    let old_etag = service
        .prepared_wire("/site/a.bin")
        .expect("prepared")
        .validators
        .etag
        .clone();

    fs::write(&path, b"much-longer-payload").expect("grow");
    let asset = open_prepared(&service, "/site/a.bin");
    assert_ne!(asset.validators.etag, old_etag);
    assert_eq!(
        decide_conditional(Some(&old_etag), None, &asset.validators),
        ConditionalDecision::Continue
    );
}

#[test]
fn generation_reload_picks_up_new_prepared_identity() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    fs::create_dir_all(&root).expect("mkdir");
    let path = root.join("a.bin");
    fs::write(&path, b"v1").expect("write");

    let service = preload_root(&root);
    let etag_v1 = service
        .prepared_wire("/site/a.bin")
        .expect("prepared")
        .validators
        .etag
        .clone();

    thread::sleep(Duration::from_millis(20));
    fs::write(&path, b"v2").expect("replace");

    // New generation (new StaticRoot + preload) must publish new prepared wire.
    let service2 = preload_root(&root);
    let prepared2 = service2.prepared_wire("/site/a.bin").expect("prepared2");
    assert_ne!(prepared2.validators.etag, etag_v1);

    let asset = open_prepared(&service2, "/site/a.bin");
    assert!(Arc::ptr_eq(&asset.validators, &prepared2.validators));
    assert_eq!(
        decide_conditional(Some(&etag_v1), None, &asset.validators),
        ConditionalDecision::Continue
    );
}

/// Current ETag contract is identity-based (dev/ino/mtime/len), not content hash.
/// Same identity ⇒ same ETag even if bytes differ (operator must change mtime/inode).
#[test]
fn same_identity_same_etag_is_current_contract() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("a.bin");
    fs::write(&path, b"AAAA").expect("write");
    let meta = fs::metadata(&path).expect("meta");
    let v1 = StaticValidators::from_metadata(&meta);
    // Re-read same metadata identity without rewriting file.
    let meta2 = fs::metadata(&path).expect("meta2");
    let v2 = StaticValidators::from_metadata(&meta2);
    assert_eq!(v1.etag, v2.etag);
}
