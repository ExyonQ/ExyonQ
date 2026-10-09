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
//! KD2.4 — static response-cache policy (module-owned).
//!
//! Generic store/singleflight remain in core; this module owns static-specific
//! identity revalidation, path canonicalization for invalidation, and metrics.

#[cfg(unix)]
use crate::identity::FileIdentity;
use crate::identity::StaticResourceIdentity;
use exyonq_module_api::static_dispatch::{StaticMethod, StaticResourceIdentitySnapshot};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

static REVALIDATION_SUCCESS: AtomicU64 = AtomicU64::new(0);
static REVALIDATION_FAILURE: AtomicU64 = AtomicU64::new(0);
static INVALIDATIONS: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

/// GET representation storage uses GET body even when the client issued HEAD.
pub fn cache_storage_method(client: StaticMethod) -> StaticMethod {
    match client {
        StaticMethod::Head => StaticMethod::Get,
        other => other,
    }
}

/// Canonical filesystem path for static cache path-index / targeted invalidation.
pub fn canonical_path_for_invalidation(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn canonical_path_for_snapshot(snapshot: &StaticResourceIdentitySnapshot) -> PathBuf {
    PathBuf::from(&snapshot.canonical_path)
}

/// Fail closed: metadata errors or identity drift → false (modify/rename/delete).
pub fn snapshot_matches_current(snapshot: &StaticResourceIdentitySnapshot) -> bool {
    snapshot_to_identity(snapshot).matches_current()
}

pub fn note_revalidation_success() {
    REVALIDATION_SUCCESS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_revalidation_failure() {
    REVALIDATION_FAILURE.fetch_add(1, Ordering::Relaxed);
}

pub fn note_invalidation() {
    INVALIDATIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_cache_hit() {
    CACHE_HITS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_cache_miss() {
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
}

pub fn revalidation_success_total() -> u64 {
    REVALIDATION_SUCCESS.load(Ordering::Relaxed)
}

pub fn revalidation_failure_total() -> u64 {
    REVALIDATION_FAILURE.load(Ordering::Relaxed)
}

pub fn invalidations_total() -> u64 {
    INVALIDATIONS.load(Ordering::Relaxed)
}

pub fn cache_hits_total() -> u64 {
    CACHE_HITS.load(Ordering::Relaxed)
}

pub fn cache_misses_total() -> u64 {
    CACHE_MISSES.load(Ordering::Relaxed)
}

#[doc(hidden)]
pub fn reset_metrics_for_tests() {
    REVALIDATION_SUCCESS.store(0, Ordering::Relaxed);
    REVALIDATION_FAILURE.store(0, Ordering::Relaxed);
    INVALIDATIONS.store(0, Ordering::Relaxed);
    CACHE_HITS.store(0, Ordering::Relaxed);
    CACHE_MISSES.store(0, Ordering::Relaxed);
}

pub fn snapshot_from_identity(identity: StaticResourceIdentity) -> StaticResourceIdentitySnapshot {
    crate::outcome::identity_to_snapshot(identity)
}

fn snapshot_to_identity(snapshot: &StaticResourceIdentitySnapshot) -> StaticResourceIdentity {
    StaticResourceIdentity {
        canonical_path: Arc::from(Path::new(&snapshot.canonical_path)),
        file_len: snapshot.file_len,
        modified: snapshot
            .modified_unix_secs
            .map(|s| UNIX_EPOCH + Duration::new(s, snapshot.modified_subsec_nanos.unwrap_or(0))),
        #[cfg(unix)]
        platform_file_id: match (snapshot.dev, snapshot.ino) {
            (Some(dev), Some(ino)) => Some(FileIdentity { dev, ino }),
            _ => None,
        },
        #[cfg(not(unix))]
        platform_file_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn snapshot_roundtrip_revalidates_like_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.txt");
        fs::write(&path, b"v1").expect("write");
        let identity = StaticResourceIdentity::capture(&path).expect("capture");
        let snap = snapshot_from_identity(identity.clone());
        assert!(snapshot_matches_current(&snap));
        fs::write(&path, b"v2-longer").expect("rewrite");
        assert!(!snapshot_matches_current(&snap));
    }

    #[test]
    fn head_uses_get_for_storage_method() {
        assert_eq!(cache_storage_method(StaticMethod::Head), StaticMethod::Get);
        assert_eq!(cache_storage_method(StaticMethod::Get), StaticMethod::Get);
    }
}
