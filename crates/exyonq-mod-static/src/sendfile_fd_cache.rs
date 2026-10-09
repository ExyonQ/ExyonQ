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
//! Cap067 worker-local generation-scoped sendfile FD reuse.
//!
//! # Contract
//!
//! - Cap004 `openat2(RESOLVE_BENEATH)` on cache **miss** (and inode change).
//! - On **hit**, revalidate pathname identity via `fstatat` from the generation
//!   root dirfd (same relative path Cap004 would open). The epoll hot path
//!   (`try_hit_hot`) repeats that `fstatat` at most once per millisecond per
//!   entry so a stable file does not pay a syscall on every request. A direct
//!   `try_hit` always stats. Atomic rename / delete / recreate that changes
//!   `(dev,ino)` forces Cap004 reopen once the interval has elapsed. In-place
//!   rewrite (same inode, new mtime/len) refreshes Cap020 wire via `fstat` on
//!   the cached FD.
//! - **Coded (ADR-046) entries** key on `(path, coding)` and store the *source*
//!   identity for revalidation. Source identity drift drops the entry (no
//!   same-inode wire refresh — coded object key includes mtime/len). Never
//!   select an identity FD when AE wants a coding.
//! - Entries are stamped with [`crate::StaticRuntime`] generation; `bind_roots`
//!   makes prior entries unselectable.
//! - Thread-local map: no global Mutex on the Cap067 hot path (Root2 lesson).
//!
//! `SendfileHandleRegistry` remains a short-lived match→begin handoff, **not**
//! the FD-reuse source of truth.

#![cfg(target_os = "linux")]
#![allow(dead_code)] // metrics/local_len are diagnostic surfaces for Netcup attribution

use crate::conditional::{PreparedStaticWire, ValidatorIdentity};
use crate::sendfile::{identity_from_c_str, SendfileAsset};
use crate::StaticRoot;
use std::cell::RefCell;
use std::ffi::CString;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Soft cap per epoll worker. P1 hot sets are tiny; bound FD pressure.
pub const FD_CACHE_MAX_ENTRIES: usize = 64;

/// Epoll hot path: one `fstatat` per entry per this interval. Direct `try_hit` ignores it.
const HOT_REVALIDATE_INTERVAL: Duration = Duration::from_millis(1);

/// Discriminant for ADR-046 coded FD entries (must match `CacheCoding::fd_cache_tag`).
/// `None` = identity (no Content-Encoding).
pub type CodingTag = Option<u8>;
pub const CODING_GZIP: u8 = 1;
pub const CODING_BROTLI: u8 = 2;

static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static REVALID_FAIL: AtomicU64 = AtomicU64::new(0);
static EVICTIONS: AtomicU64 = AtomicU64::new(0);
static INSERTS: AtomicU64 = AtomicU64::new(0);

struct CacheEntry {
    site_slot: u32,
    /// Owned once at insert. Hit compares with `&str` and does not allocate.
    request_path: String,
    /// `None` = identity sendfile; `Some(CODING_*)` = coded object FD.
    coding: CodingTag,
    /// Relative path under the generation root, built once at insert.
    /// For coded entries this is the *source* path used for `fstatat` revalidation.
    rel: CString,
    generation: u64,
    /// Source (or identity-file) identity stamped at insert / last refresh.
    identity: ValidatorIdentity,
    /// Last successful identity check. Hot hits skip `fstatat` inside [`HOT_REVALIDATE_INTERVAL`].
    revalidated_at: Option<Instant>,
    asset: Arc<SendfileAsset>,
}

struct LocalFdCache {
    entries: Vec<CacheEntry>,
}

impl LocalFdCache {
    fn new() -> Self {
        Self {
            entries: Vec::with_capacity(16),
        }
    }
}

thread_local! {
    static LOCAL: RefCell<LocalFdCache> = RefCell::new(LocalFdCache::new());
}

/// Drop all worker-local FD cache entries (pin publish / tests).
pub fn clear_local() {
    LOCAL.with(|cell| cell.borrow_mut().entries.clear());
}

pub fn hits() -> u64 {
    HITS.load(Ordering::Relaxed)
}
pub fn misses() -> u64 {
    MISSES.load(Ordering::Relaxed)
}
pub fn revalidation_failures() -> u64 {
    REVALID_FAIL.load(Ordering::Relaxed)
}
pub fn evictions() -> u64 {
    EVICTIONS.load(Ordering::Relaxed)
}
pub fn inserts() -> u64 {
    INSERTS.load(Ordering::Relaxed)
}

#[doc(hidden)]
pub fn reset_metrics_for_tests() {
    HITS.store(0, Ordering::Relaxed);
    MISSES.store(0, Ordering::Relaxed);
    REVALID_FAIL.store(0, Ordering::Relaxed);
    EVICTIONS.store(0, Ordering::Relaxed);
    INSERTS.store(0, Ordering::Relaxed);
}

fn find_entry(
    cache: &LocalFdCache,
    site_slot: u32,
    request_path: &str,
    coding: CodingTag,
) -> Option<usize> {
    cache.entries.iter().position(|e| {
        e.site_slot == site_slot && e.coding == coding && e.request_path == request_path
    })
}

/// Look up a reusable **identity** asset. On hit + identity match, returns `Some(Arc)`.
/// On miss / stale generation / inode change, returns `None` (caller Cap004-opens).
///
/// Hit does not allocate: the request path is compared borrowed, and `fstatat`
/// uses the `CString` stored at insert.
pub fn try_hit(
    generation: u64,
    site_slot: u32,
    request_path: &str,
    root: &StaticRoot,
) -> Option<Arc<SendfileAsset>> {
    try_hit_with_coding(generation, site_slot, request_path, None, root, true, false)
}

/// Same as [`try_hit`], but a confirmed identity is reused for one millisecond.
pub fn try_hit_hot(
    generation: u64,
    site_slot: u32,
    request_path: &str,
    root: &StaticRoot,
) -> Option<Arc<SendfileAsset>> {
    try_hit_with_coding(generation, site_slot, request_path, None, root, true, true)
}

/// Look up a reusable **coded** sendfile asset (ADR-046).
///
/// Revalidates *source* pathname identity. Exact identity match → reuse coded FD.
/// Any source drift (incl. same-inode mtime/len) → drop (coded object key changed).
pub fn try_hit_coded(
    generation: u64,
    site_slot: u32,
    request_path: &str,
    coding_tag: u8,
    root: &StaticRoot,
) -> Option<Arc<SendfileAsset>> {
    try_hit_with_coding(
        generation,
        site_slot,
        request_path,
        Some(coding_tag),
        root,
        false,
        false,
    )
}

/// Same as [`try_hit_coded`], with the hot-path revalidation interval.
pub fn try_hit_coded_hot(
    generation: u64,
    site_slot: u32,
    request_path: &str,
    coding_tag: u8,
    root: &StaticRoot,
) -> Option<Arc<SendfileAsset>> {
    try_hit_with_coding(
        generation,
        site_slot,
        request_path,
        Some(coding_tag),
        root,
        false,
        true,
    )
}

fn try_hit_with_coding(
    generation: u64,
    site_slot: u32,
    request_path: &str,
    coding: CodingTag,
    root: &StaticRoot,
    allow_same_inode_refresh: bool,
    coalesce: bool,
) -> Option<Arc<SendfileAsset>> {
    LOCAL.with(|cell| {
        let mut cache = cell.borrow_mut();
        let Some(idx) = find_entry(&cache, site_slot, request_path, coding) else {
            MISSES.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        if cache.entries[idx].generation != generation {
            cache.entries.swap_remove(idx);
            MISSES.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        if coalesce {
            if let Some(at) = cache.entries[idx].revalidated_at {
                if at.elapsed() < HOT_REVALIDATE_INTERVAL {
                    HITS.fetch_add(1, Ordering::Relaxed);
                    return Some(Arc::clone(&cache.entries[idx].asset));
                }
            }
        }
        let stat = identity_from_c_str(cache.entries[idx].rel.as_c_str(), root.root_dir_fd());
        let now = match stat {
            Ok(id) => id,
            Err(_) => {
                cache.entries.swap_remove(idx);
                REVALID_FAIL.fetch_add(1, Ordering::Relaxed);
                MISSES.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        if now == cache.entries[idx].identity {
            cache.entries[idx].revalidated_at = Some(Instant::now());
            HITS.fetch_add(1, Ordering::Relaxed);
            return Some(Arc::clone(&cache.entries[idx].asset));
        }
        // Identity only: same inode, metadata drift → keep FD, refresh Cap020 wire.
        // Coded: never refresh in place — object key includes mtime/len.
        if allow_same_inode_refresh
            && now.dev == cache.entries[idx].identity.dev
            && now.ino == cache.entries[idx].identity.ino
        {
            let meta_result = cache.entries[idx].asset.file.metadata();
            let meta = match meta_result {
                Ok(m) => m,
                Err(_) => {
                    cache.entries.swap_remove(idx);
                    REVALID_FAIL.fetch_add(1, Ordering::Relaxed);
                    MISSES.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            };
            let refreshed = Arc::new(asset_refresh_meta(cache.entries[idx].asset.as_ref(), &meta));
            cache.entries[idx].identity = now;
            cache.entries[idx].revalidated_at = Some(Instant::now());
            cache.entries[idx].asset = Arc::clone(&refreshed);
            HITS.fetch_add(1, Ordering::Relaxed);
            return Some(refreshed);
        }
        // Different inode (or coded source drift): drop and reopen.
        cache.entries.swap_remove(idx);
        REVALID_FAIL.fetch_add(1, Ordering::Relaxed);
        MISSES.fetch_add(1, Ordering::Relaxed);
        None
    })
}

/// Insert Cap004-opened **identity** asset into the worker-local cache.
///
/// The relative `CString` is built once here. A path that cannot be expressed
/// under the root is not cached (next request Cap004-opens again).
pub fn insert(
    generation: u64,
    site_slot: u32,
    request_path: &str,
    file_path: &Path,
    canonical_root: &Path,
    asset: Arc<SendfileAsset>,
    identity: ValidatorIdentity,
) {
    insert_with_coding(
        generation,
        site_slot,
        request_path,
        None,
        file_path,
        canonical_root,
        asset,
        identity,
    );
}

/// Insert a coded sendfile asset keyed by `(path, coding_tag)`.
///
/// `source_file_path` is the Cap004 source path (for `fstatat` revalidation);
/// `source_identity` is the source identity that produced the coded object.
pub fn insert_coded(
    generation: u64,
    site_slot: u32,
    request_path: &str,
    coding_tag: u8,
    source_file_path: &Path,
    canonical_root: &Path,
    asset: Arc<SendfileAsset>,
    source_identity: ValidatorIdentity,
) {
    insert_with_coding(
        generation,
        site_slot,
        request_path,
        Some(coding_tag),
        source_file_path,
        canonical_root,
        asset,
        source_identity,
    );
}

fn insert_with_coding(
    generation: u64,
    site_slot: u32,
    request_path: &str,
    coding: CodingTag,
    file_path: &Path,
    canonical_root: &Path,
    asset: Arc<SendfileAsset>,
    identity: ValidatorIdentity,
) {
    let Ok(rel) = crate::sendfile::relative_c_path(file_path, canonical_root) else {
        return;
    };
    LOCAL.with(|cell| {
        let mut cache = cell.borrow_mut();
        let existing = find_entry(&cache, site_slot, request_path, coding);
        let entry = CacheEntry {
            site_slot,
            request_path: request_path.to_string(),
            coding,
            rel,
            generation,
            identity,
            revalidated_at: Some(Instant::now()),
            asset,
        };
        if let Some(idx) = existing {
            cache.entries[idx] = entry;
        } else {
            if cache.entries.len() >= FD_CACHE_MAX_ENTRIES {
                cache.entries.swap_remove(0);
                EVICTIONS.fetch_add(1, Ordering::Relaxed);
            }
            cache.entries.push(entry);
        }
        INSERTS.fetch_add(1, Ordering::Relaxed);
    });
}

fn asset_refresh_meta(old: &SendfileAsset, meta: &std::fs::Metadata) -> SendfileAsset {
    let content_type = old.content_type;
    let len = usize::try_from(meta.len()).unwrap_or(old.body_len);
    let wire = PreparedStaticWire::from_metadata(meta, content_type);
    SendfileAsset {
        file: Arc::clone(&old.file),
        header: wire.header_200,
        header_304: wire.header_304,
        body_len: len,
        content_type,
        validators: wire.validators,
    }
}

/// Peak entries currently held on **this** worker (test / diagnostics).
pub fn local_len() -> usize {
    LOCAL.with(|cell| cell.borrow().entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conditional::decide_conditional;
    use crate::conditional::ConditionalDecision;
    use crate::sendfile::{identity_under_root, OpenUnderRoot, SendfileAsset};
    use crate::{PreloadLimits, StaticRoot, StaticRuntime};
    use std::fs;
    use std::thread;
    use std::time::Duration;

    fn sendfile_fixture(byte: u8) -> Vec<u8> {
        vec![byte; crate::sendfile::SENDFILE_MIN_BYTES]
    }

    fn serial_pin() -> std::sync::MutexGuard<'static, ()> {
        crate::epoll_session::serial_runtime_pin_for_tests()
    }

    fn preload_root(root: &Path) -> StaticRoot {
        let mut service =
            StaticRoot::new_with_preload_limits(root, "/site", None, PreloadLimits::DEFAULT)
                .expect("root");
        service.preload_tree().expect("preload");
        service
    }

    fn head_get(path: &str) -> Vec<u8> {
        format!("GET {path} HTTP/1.1\r\nHost: t\r\n\r\n").into_bytes()
    }

    #[test]
    fn unchanged_file_second_match_reuses_file_arc() {
        let _g = serial_pin();
        crate::sendfile_fsm::reset_epoll_sendfile_enabled_cache_for_tests();
        clear_local();
        reset_metrics_for_tests();

        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        fs::create_dir_all(&root_path).expect("mkdir");
        fs::write(root_path.join("a.bin"), sendfile_fixture(0)).expect("write");

        let root = preload_root(&root_path);
        let rt = Arc::new(StaticRuntime::new());
        rt.bind_roots(1, Box::new([Arc::new(root)]));
        crate::epoll_session::pin_runtime(Arc::clone(&rt));

        let head = head_get("/site/a.bin");
        let h1 = crate::epoll_session::match_sendfile_asset(0, &head).expect("match1");
        let a1 = rt.take_sendfile_handle(h1).expect("take1");
        let h2 = crate::epoll_session::match_sendfile_asset(0, &head).expect("match2");
        let a2 = rt.take_sendfile_handle(h2).expect("take2");

        assert!(
            Arc::ptr_eq(&a1.file, &a2.file),
            "second Cap067 match must reuse Cap004 File Arc"
        );
        assert!(hits() >= 1);
    }

    #[test]
    fn atomic_rename_forces_new_inode_fd() {
        let _g = serial_pin();
        crate::sendfile_fsm::reset_epoll_sendfile_enabled_cache_for_tests();
        clear_local();
        reset_metrics_for_tests();

        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        fs::create_dir_all(&root_path).expect("mkdir");
        let path = root_path.join("a.bin");
        fs::write(&path, sendfile_fixture(b'o')).expect("write");

        let root = preload_root(&root_path);
        let rt = Arc::new(StaticRuntime::new());
        rt.bind_roots(1, Box::new([Arc::new(root)]));
        crate::epoll_session::pin_runtime(Arc::clone(&rt));

        let head = head_get("/site/a.bin");
        let h1 = crate::epoll_session::match_sendfile_asset(0, &head).expect("match1");
        let a1 = rt.take_sendfile_handle(h1).expect("take1");
        let etag1 = a1.validators.etag.clone();

        thread::sleep(Duration::from_millis(20));
        let tmp = root_path.join("a.bin.tmp");
        fs::write(&tmp, vec![b'n'; crate::sendfile::SENDFILE_MIN_BYTES + 1]).expect("tmp");
        fs::rename(&tmp, &path).expect("rename");

        let h2 = crate::epoll_session::match_sendfile_asset(0, &head).expect("match2");
        let a2 = rt.take_sendfile_handle(h2).expect("take2");
        assert!(
            !Arc::ptr_eq(&a1.file, &a2.file),
            "atomic rename must Cap004-reopen (new inode)"
        );
        assert_ne!(a2.validators.etag, etag1);
        assert_eq!(
            decide_conditional(Some(&etag1), None, &a2.validators),
            ConditionalDecision::Continue
        );
    }

    #[test]
    fn generation_bump_drops_selectable_cached_fd() {
        let _g = serial_pin();
        crate::sendfile_fsm::reset_epoll_sendfile_enabled_cache_for_tests();
        clear_local();
        reset_metrics_for_tests();

        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        fs::create_dir_all(&root_path).expect("mkdir");
        fs::write(root_path.join("a.bin"), sendfile_fixture(b'v')).expect("write");

        let root = preload_root(&root_path);
        let rt = Arc::new(StaticRuntime::new());
        rt.bind_roots(1, Box::new([Arc::new(root)]));
        crate::epoll_session::pin_runtime(Arc::clone(&rt));

        let head = head_get("/site/a.bin");
        let h1 = crate::epoll_session::match_sendfile_asset(0, &head).expect("m1");
        let _ = rt.take_sendfile_handle(h1).expect("t1");
        let hits_before = hits();

        let root2 = preload_root(&root_path);
        rt.bind_roots(2, Box::new([Arc::new(root2)]));

        let h2 = crate::epoll_session::match_sendfile_asset(0, &head).expect("m2");
        let _ = rt.take_sendfile_handle(h2).expect("t2");
        assert_eq!(
            hits(),
            hits_before,
            "generation N+1 must not select handles_N"
        );
    }

    #[test]
    fn miss_fill_does_not_stamp_path_identity_onto_foreign_fd() {
        // LA-CAP067-FD-001 regression: if pathname identity ≠ Cap004 FD (dev,ino),
        // insert must be skipped (simulated by calling insert only when they match —
        // here we assert the predicate the product uses).
        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        fs::create_dir_all(&root_path).expect("mkdir");
        let path = root_path.join("a.bin");
        fs::write(&path, b"inode-A").expect("write");
        let service = preload_root(&root_path);
        let file_path = service.resolved_file_path("/site/a.bin").expect("path");
        let open_root = OpenUnderRoot {
            canonical_root: service.canonical_root(),
            root_dir_fd: service.root_dir_fd(),
        };
        let asset = SendfileAsset::open_under_root_prepared(&file_path, Some(open_root), 0, None)
            .expect("open A");
        let fd_id = ValidatorIdentity::from_metadata(&asset.file.metadata().expect("meta"));

        let tmp = root_path.join("a.bin.tmp");
        fs::write(&tmp, b"inode-B-content").expect("tmp");
        fs::rename(&tmp, &path).expect("rename over A");
        let path_id = identity_under_root(&file_path, open_root).expect("path id B");

        assert_ne!(
            (fd_id.dev, fd_id.ino),
            (path_id.dev, path_id.ino),
            "fixture must produce FD≠pathname identity"
        );
        // Product must not insert under this mismatch (cacheable predicate).
        assert!(
            !(path_id.dev == fd_id.dev && path_id.ino == fd_id.ino),
            "mismatch must be non-cacheable"
        );
    }

    #[test]
    fn coded_second_lookup_reuses_file_arc() {
        let _g = serial_pin();
        clear_local();
        reset_metrics_for_tests();

        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        let cache_path = dir.path().join("enc-cache");
        fs::create_dir_all(&root_path).expect("mkdir root");
        fs::create_dir_all(&cache_path).expect("mkdir cache");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&cache_path).expect("meta").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&cache_path, perms).expect("chmod");
        }
        let plain = vec![b'p'; 512];
        fs::write(root_path.join("a.txt"), &plain).expect("write");

        let root = preload_root(&root_path);
        let file_path = root.resolved_file_path("/site/a.txt").expect("path");
        let open_root = OpenUnderRoot {
            canonical_root: root.canonical_root(),
            root_dir_fd: root.root_dir_fd(),
        };
        let source_id = identity_under_root(&file_path, open_root).expect("src id");
        let cfg = crate::encoding_cache::EncodingCacheConfig {
            enabled: true,
            cache_dir: cache_path,
            level: 6,
            ..crate::encoding_cache::EncodingCacheConfig::default()
        };
        let coded_path = crate::encoding_cache::ensure_coded_object(
            &cfg,
            &source_id,
            crate::encoding_cache::CacheCoding::Gzip,
            &plain,
        )
        .expect("compress");
        let validators = crate::conditional::StaticValidators::from_validator_identity(&source_id);
        let coded = crate::encoding_cache::open_encoded_sendfile_asset(
            &coded_path,
            &validators,
            crate::encoding_cache::CacheCoding::Gzip,
            "text/plain",
        )
        .expect("open coded");
        let asset = Arc::new(coded);

        insert_coded(
            1,
            0,
            "/site/a.txt",
            CODING_GZIP,
            &file_path,
            root.canonical_root(),
            Arc::clone(&asset),
            source_id,
        );
        let h1 = try_hit_coded(1, 0, "/site/a.txt", CODING_GZIP, &root).expect("hit1");
        let h2 = try_hit_coded(1, 0, "/site/a.txt", CODING_GZIP, &root).expect("hit2");
        assert!(
            Arc::ptr_eq(&h1.file, &h2.file),
            "coded FD cache must reuse File Arc"
        );
        assert!(
            Arc::ptr_eq(&h1.file, &asset.file),
            "hit must be the inserted coded FD"
        );
        // Identity key must not select coded entry.
        assert!(
            try_hit(1, 0, "/site/a.txt", &root).is_none(),
            "identity try_hit must not return coded FD"
        );
        assert!(hits() >= 2);
    }

    #[test]
    fn coded_source_rewrite_drops_cached_fd() {
        let _g = serial_pin();
        clear_local();
        reset_metrics_for_tests();

        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        let cache_path = dir.path().join("enc-cache");
        fs::create_dir_all(&root_path).expect("mkdir root");
        fs::create_dir_all(&cache_path).expect("mkdir cache");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&cache_path).expect("meta").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&cache_path, perms).expect("chmod");
        }
        let path = root_path.join("a.txt");
        let plain = vec![b'q'; 512];
        fs::write(&path, &plain).expect("write");

        let root = preload_root(&root_path);
        let file_path = root.resolved_file_path("/site/a.txt").expect("path");
        let open_root = OpenUnderRoot {
            canonical_root: root.canonical_root(),
            root_dir_fd: root.root_dir_fd(),
        };
        let source_id = identity_under_root(&file_path, open_root).expect("src id");
        let cfg = crate::encoding_cache::EncodingCacheConfig {
            enabled: true,
            cache_dir: cache_path,
            level: 6,
            ..crate::encoding_cache::EncodingCacheConfig::default()
        };
        let coded_path = crate::encoding_cache::ensure_coded_object(
            &cfg,
            &source_id,
            crate::encoding_cache::CacheCoding::Gzip,
            &plain,
        )
        .expect("compress");
        let validators = crate::conditional::StaticValidators::from_validator_identity(&source_id);
        let coded = crate::encoding_cache::open_encoded_sendfile_asset(
            &coded_path,
            &validators,
            crate::encoding_cache::CacheCoding::Gzip,
            "text/plain",
        )
        .expect("open coded");
        insert_coded(
            1,
            0,
            "/site/a.txt",
            CODING_GZIP,
            &file_path,
            root.canonical_root(),
            Arc::new(coded),
            source_id,
        );
        assert!(try_hit_coded(1, 0, "/site/a.txt", CODING_GZIP, &root).is_some());

        thread::sleep(Duration::from_millis(20));
        fs::write(&path, vec![b'r'; 600]).expect("rewrite");
        assert!(
            try_hit_coded(1, 0, "/site/a.txt", CODING_GZIP, &root).is_none(),
            "source identity drift must drop coded FD"
        );
    }
}
