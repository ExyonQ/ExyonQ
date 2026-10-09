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
//! Cached static root: canonicalize once, resolve paths without repeated syscalls.

use super::body::BoxBody;
use super::precooked::PrecookedResponse;
use super::{content_type_for, relative_as_safe_path, strip_route_prefix, StaticError};
use crate::conditional::PreparedStaticWire;
use crate::identity::StaticResourceIdentity;
use bytes::Bytes;
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Files at or above this size are mmap'd at preload instead of heap-copied.
const PRELOAD_MMAP_THRESHOLD: u64 = 65536;

/// Tree-preload memory limits (STATIC_PRELOAD_MEMORY_POLICY_DECISION).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreloadLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_entries: u64,
}

impl PreloadLimits {
    pub const DEFAULT: Self = Self {
        max_file_bytes: 33_554_432,
        max_total_bytes: 268_435_456,
        max_entries: 4096,
    };

    pub fn is_disabled(self) -> bool {
        self.max_file_bytes == 0 || self.max_total_bytes == 0 || self.max_entries == 0
    }
}

impl Default for PreloadLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

struct CachedEntry {
    path: PathBuf,
    precooked: Arc<PrecookedResponse>,
    /// Cap067 Cap020 fragments keyed to preload-time file identity.
    prepared: PreparedStaticWire,
}

impl CachedEntry {
    /// The snapshot body is usable only while inode, length, and mtime still match.
    /// A miss here is `NotFound` so the caller reads the live file.
    fn body_still_current(&self) -> bool {
        match std::fs::metadata(&self.path) {
            Ok(meta) if meta.is_file() => {
                crate::conditional::ValidatorIdentity::from_metadata(&meta)
                    == self.prepared.identity
            }
            _ => false,
        }
    }
}

struct PreloadCandidate {
    path: PathBuf,
    request_path: String,
}

pub struct StaticRoot {
    canonical_root: PathBuf,
    /// Held directory fd for Cap004 openat2(RESOLVE_BENEATH) opens (Linux).
    #[cfg(target_os = "linux")]
    root_dir: File,
    route_prefix: String,
    /// IR `match.host`. `None` matches any request Host.
    route_host: Option<String>,
    index: Option<String>,
    preload_limits: PreloadLimits,
    /// PHP source and dotfiles. False denies them, including names already preloaded.
    allow_sensitive: bool,
    cache: Arc<HashMap<String, CachedEntry>>,
}

impl StaticRoot {
    pub fn new(root: &Path, route_prefix: &str, index: Option<&str>) -> Result<Self, StaticError> {
        Self::new_with_preload_limits(root, route_prefix, index, PreloadLimits::DEFAULT)
    }

    pub fn new_with_preload_limits(
        root: &Path,
        route_prefix: &str,
        index: Option<&str>,
        preload_limits: PreloadLimits,
    ) -> Result<Self, StaticError> {
        let canonical_root = root.canonicalize()?;
        #[cfg(target_os = "linux")]
        let root_dir = open_root_dirfd(&canonical_root)?;
        Ok(Self {
            canonical_root,
            #[cfg(target_os = "linux")]
            root_dir,
            route_prefix: route_prefix.to_string(),
            route_host: None,
            index: index.map(str::to_string),
            preload_limits,
            allow_sensitive: false,
            cache: Arc::new(HashMap::new()),
        })
    }

    pub fn set_allow_sensitive(&mut self, allow: bool) {
        self.allow_sensitive = allow;
    }

    fn reject_sensitive(&self, request_path: &str) -> Result<(), StaticError> {
        if self.allow_sensitive || !crate::path_is_sensitive_static(request_path) {
            Ok(())
        } else {
            Err(StaticError::Forbidden)
        }
    }

    pub fn preload_limits(&self) -> PreloadLimits {
        self.preload_limits
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Canonical filesystem root for this generation-scoped snapshot (Cap004 prefix).
    #[inline]
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    /// Directory fd for `canonical_root` (Linux Cap004 openat2 containment).
    #[cfg(target_os = "linux")]
    #[inline]
    pub fn root_dir_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.root_dir.as_fd()
    }

    pub fn resolve_path_sync(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        self.resolved_file_path(request_path)
    }

    /// Canonical filesystem path for a request (preload hit or on-demand resolve).
    pub fn resolved_file_path(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        self.reject_sensitive(request_path)?;
        if let Some(entry) = self.lookup_entry(request_path) {
            return Ok(entry.path.clone());
        }
        self.resolve_path_uncached(request_path)
    }

    /// Cap067: generation-scoped prepared validators/headers when request is preloaded.
    pub fn prepared_wire(&self, request_path: &str) -> Option<&PreparedStaticWire> {
        Some(&self.lookup_entry(request_path)?.prepared)
    }

    /// Capture file identity for cache storage (metadata only, no body read).
    pub fn capture_identity_for_request(
        &self,
        request_path: &str,
    ) -> Result<StaticResourceIdentity, StaticError> {
        let path = self.resolve_live_file_path(request_path)?;
        StaticResourceIdentity::capture(&path).map_err(StaticError::Io)
    }

    /// Live path for Hyper / on-demand readers that open with ordinary `File::open`.
    ///
    /// Always Cap004-uncached (`canonicalize` + root prefix). Do **not** return a
    /// preload PathBuf here: post-preload intermediate symlink swaps must not be
    /// followable by `File::open` (logic audit LA-CAUSE1-001).
    ///
    /// Cap067 sendfile amortizes via [`Self::resolved_file_path`] +
    /// [`crate::sendfile::SendfileAsset::open_under_root`] only.
    pub fn resolve_live_file_path(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        self.resolve_path_uncached(request_path)
    }

    #[inline]
    #[cfg(target_os = "linux")]
    pub fn is_head_wire_request(head: &[u8]) -> bool {
        head.starts_with(b"HEAD ")
    }

    /// HTTP/3 static fast path: shared body bytes for preloaded cache assets.
    pub fn lookup_h3_static(&self, request_path: &str) -> Option<(&'static str, Arc<Bytes>)> {
        self.reject_sensitive(request_path).ok()?;
        let entry = self.lookup_entry(request_path)?;
        if !entry.body_still_current() {
            return None;
        }
        Some((
            entry.precooked.content_type,
            Arc::clone(&entry.precooked.body),
        ))
    }

    /// Hot path for preloaded assets: ordinary map lookup.
    pub fn serve_request(
        &self,
        request_path: &str,
    ) -> Result<hyper::Response<BoxBody>, StaticError> {
        self.reject_sensitive(request_path)?;
        if let Some(relative) = strip_route_prefix(&self.route_prefix, request_path) {
            let _ = relative_as_safe_path(&relative)?;
        } else {
            return Err(StaticError::NotFound);
        }

        let entry = self
            .lookup_entry(request_path)
            .ok_or(StaticError::NotFound)?;
        if !entry.body_still_current() {
            return Err(StaticError::NotFound);
        }
        Ok(entry.precooked.to_response())
    }

    /// HEAD for static assets (headers + Content-Length, no body).
    pub fn serve_head_request(
        &self,
        request_path: &str,
    ) -> Result<hyper::Response<BoxBody>, StaticError> {
        self.reject_sensitive(request_path)?;
        if let Some(relative) = strip_route_prefix(&self.route_prefix, request_path) {
            let _ = relative_as_safe_path(&relative)?;
        } else {
            return Err(StaticError::NotFound);
        }

        let entry = self
            .lookup_entry(request_path)
            .ok_or(StaticError::NotFound)?;
        if !entry.body_still_current() {
            return Err(StaticError::NotFound);
        }
        Ok(entry.precooked.to_head_response())
    }

    fn lookup_entry(&self, request_path: &str) -> Option<&CachedEntry> {
        if let Some(entry) = self.cache.get(request_path) {
            return Some(entry);
        }
        let index_name = self.index.as_deref().unwrap_or("index.html");
        for candidate in
            index_lookup_candidates(self.route_prefix.as_str(), request_path, index_name)
        {
            if let Some(entry) = self.cache.get(&candidate) {
                return Some(entry);
            }
        }
        None
    }

    /// Warm path: load files under the document root into an immutable cache (startup/reload).
    ///
    /// Bounded by [`PreloadLimits`]: per-file, total logical bytes, and entry count.
    /// Selection order is deterministic UTF-8 path order. Oversized / budget-skipped files
    /// remain available via on-demand resolve (not 404).
    pub fn preload_tree(&mut self) -> Result<(), StaticError> {
        let limits = self.preload_limits;
        if limits.is_disabled() {
            tracing::info!(
                root = %self.canonical_root.display(),
                max_file_bytes = limits.max_file_bytes,
                max_total_bytes = limits.max_total_bytes,
                max_entries = limits.max_entries,
                "static preload disabled (zero limit); serving on demand only"
            );
            self.cache = Arc::new(HashMap::new());
            return Ok(());
        }

        tracing::info!(
            root = %self.canonical_root.display(),
            max_file_bytes = limits.max_file_bytes,
            max_total_bytes = limits.max_total_bytes,
            max_entries = limits.max_entries,
            "static preload build start"
        );

        let mut candidates = Vec::new();
        self.collect_preload_candidates(
            &self.canonical_root.clone(),
            Path::new(""),
            &mut candidates,
        )?;
        candidates.sort_by(|a, b| a.request_path.as_bytes().cmp(b.request_path.as_bytes()));
        let considered = candidates.len();

        let mut map = HashMap::new();
        let mut accepted_bytes: u64 = 0;
        let mut accepted_entries: u64 = 0;
        let mut skipped: u64 = 0;
        let mut stop_accepting = false;

        for cand in &candidates {
            if stop_accepting {
                break;
            }
            if accepted_entries >= limits.max_entries {
                tracing::warn!(
                    accepted_entries,
                    max_entries = limits.max_entries,
                    "static preload max_entries reached; stopping acceptance"
                );
                stop_accepting = true;
                break;
            }

            let file = match File::open(&cand.path) {
                Ok(f) => f,
                Err(err) => {
                    tracing::warn!(
                        path = %cand.path.display(),
                        error = %err,
                        "static preload per-file I/O skip"
                    );
                    skipped += 1;
                    continue;
                }
            };
            let meta = match file.metadata() {
                Ok(m) => m,
                Err(err) => {
                    tracing::warn!(
                        path = %cand.path.display(),
                        error = %err,
                        "static preload metadata skip"
                    );
                    skipped += 1;
                    continue;
                }
            };
            if !meta.is_file() {
                continue;
            }
            let len = meta.len();
            if len > limits.max_file_bytes {
                tracing::warn!(
                    path = %cand.request_path,
                    file_bytes = len,
                    max_file_bytes = limits.max_file_bytes,
                    "static preload oversized file skipped; serve on demand"
                );
                skipped += 1;
                continue;
            }

            let candidate_total = match accepted_bytes.checked_add(len) {
                Some(total) => total,
                None => {
                    tracing::error!(
                        path = %cand.request_path,
                        accepted_bytes,
                        file_bytes = len,
                        "static preload checked_add overflow; failing build"
                    );
                    return Err(StaticError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "preload byte accumulation overflow",
                    )));
                }
            };
            if candidate_total > limits.max_total_bytes {
                tracing::warn!(
                    path = %cand.request_path,
                    current_bytes = accepted_bytes,
                    candidate_bytes = len,
                    total_budget = limits.max_total_bytes,
                    accepted_entries,
                    "static preload total budget reached; stopping acceptance"
                );
                stop_accepting = true;
                break;
            }

            let body = match load_preload_body_bounded(&cand.path, file, len, limits.max_file_bytes)
            {
                Ok(body) => body,
                Err(StaticError::BudgetExceeded) => {
                    tracing::warn!(
                        path = %cand.request_path,
                        max_file_bytes = limits.max_file_bytes,
                        "static preload bounded read exceeded; skipped"
                    );
                    skipped += 1;
                    continue;
                }
                Err(err) => {
                    tracing::warn!(
                        path = %cand.path.display(),
                        error = %err,
                        "static preload load skip"
                    );
                    skipped += 1;
                    continue;
                }
            };

            let content_type = content_type_for(&cand.path);
            let prepared = PreparedStaticWire::from_metadata(&meta, content_type);
            map.insert(
                cand.request_path.clone(),
                CachedEntry {
                    path: cand.path.clone(),
                    precooked: PrecookedResponse::new(body, content_type),
                    prepared,
                },
            );
            accepted_bytes = candidate_total;
            accepted_entries = accepted_entries.saturating_add(1);
        }

        tracing::info!(
            considered,
            accepted = accepted_entries,
            skipped,
            logical_bytes = accepted_bytes,
            max_file_bytes = limits.max_file_bytes,
            total_budget = limits.max_total_bytes,
            max_entries = limits.max_entries,
            partial = stop_accepting || skipped > 0,
            "static preload build complete"
        );

        self.cache = Arc::new(map);
        Ok(())
    }

    fn collect_preload_candidates(
        &self,
        dir: &Path,
        relative: &Path,
        out: &mut Vec<PreloadCandidate>,
    ) -> Result<(), StaticError> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            // Follow symlinks only after canonicalize + root containment.
            // Without this, `is_file()`/`is_dir()` follow escapes and preload can
            // cache outside-root bytes under an in-root request path (Cap004).
            let Ok(canonical) = path.canonicalize() else {
                continue;
            };
            if !canonical.starts_with(&self.canonical_root) {
                tracing::warn!(
                    path = %path.display(),
                    canonical = %canonical.display(),
                    root = %self.canonical_root.display(),
                    "static preload path escapes root; skipped"
                );
                continue;
            }
            if canonical.is_dir() {
                let next = relative.join(entry.file_name());
                self.collect_preload_candidates(&canonical, &next, out)?;
                continue;
            }
            if !canonical.is_file() {
                continue;
            }
            let rel = relative.join(entry.file_name());
            let request_path = if rel.as_os_str().is_empty() {
                self.route_prefix.clone()
            } else {
                format!(
                    "{}/{}",
                    self.route_prefix.trim_end_matches('/'),
                    rel.to_string_lossy()
                )
            };
            if !self.allow_sensitive && crate::path_is_sensitive_static(&request_path) {
                continue;
            }
            out.push(PreloadCandidate {
                path: canonical,
                request_path,
            });
        }
        Ok(())
    }

    fn resolve_path_uncached(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        self.reject_sensitive(request_path)?;
        let relative = match strip_route_prefix(&self.route_prefix, request_path) {
            Some(value) => value,
            None => return Err(StaticError::NotFound),
        };
        let relative_path = relative_as_safe_path(&relative)?;

        let mut candidate = self.canonical_root.join(relative_path);
        // Index directory: follow in-root dir symlinks (historical semantics), then
        // canonicalize + root prefix. Regular-file confirmation is deferred to open.
        match std::fs::metadata(&candidate) {
            Ok(meta) if meta.is_dir() => {
                let index_name = self.index.as_deref().unwrap_or("index.html");
                candidate = candidate.join(index_name);
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(StaticError::NotFound);
            }
            Err(_) => {
                // Fall through to canonicalize for races / exotic types.
            }
        }

        let canonical = candidate.canonicalize().map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                StaticError::NotFound
            } else {
                StaticError::Io(err)
            }
        })?;

        if !canonical.starts_with(&self.canonical_root) {
            return Err(StaticError::PathTraversal);
        }
        if let Ok(rel) = canonical.strip_prefix(&self.canonical_root) {
            self.reject_sensitive(&rel.to_string_lossy())?;
        }
        // Regular-file confirmation deferred to open(O_NOFOLLOW) at the I/O boundary.
        Ok(canonical)
    }

    pub(crate) fn set_route_host(&mut self, host: Option<String>) {
        self.route_host = host;
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn route_prefix(&self) -> &str {
        &self.route_prefix
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn route_host(&self) -> Option<&str> {
        self.route_host.as_deref()
    }
}

#[cfg(target_os = "linux")]
fn open_root_dirfd(canonical_root: &Path) -> Result<File, StaticError> {
    use std::os::unix::fs::OpenOptionsExt;
    // O_NOFOLLOW: final-component symlink must not redirect the Cap004 dirfd trust anchor.
    File::options()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(canonical_root)
        .map_err(StaticError::Io)
}

fn load_preload_body_bounded(
    path: &Path,
    mut file: File,
    logical_len: u64,
    max_file_bytes: u64,
) -> Result<Arc<Bytes>, StaticError> {
    // mmap and heap paths both charge full logical_len against budgets (caller).
    // Never map/read before the per-file check (caller + re-check here).
    if logical_len > max_file_bytes {
        return Err(StaticError::BudgetExceeded);
    }
    if logical_len >= PRELOAD_MMAP_THRESHOLD {
        let mmap = unsafe { Mmap::map(&file).map_err(StaticError::Io)? };
        let mapped = mmap.len() as u64;
        if mapped > max_file_bytes {
            drop(mmap);
            return Err(StaticError::BudgetExceeded);
        }
        return Ok(Arc::new(Bytes::from_owner(mmap)));
    }
    let buf = crate::bounded_read::read_bounded_from_reader(&mut file, max_file_bytes)?;
    let _ = path;
    Ok(Arc::new(Bytes::from(buf)))
}

fn index_lookup_candidates(
    route_prefix: &str,
    request_path: &str,
    index_name: &str,
) -> Vec<String> {
    let index_name = if index_name.is_empty() {
        "index.html"
    } else {
        index_name
    };
    let prefix = route_prefix.trim_end_matches('/');
    let mut out = Vec::new();
    if request_path == prefix || request_path == format!("{prefix}/") {
        out.push(format!("{prefix}/{index_name}"));
        return out;
    }
    if request_path.ends_with('/') {
        out.push(format!("{request_path}{index_name}"));
        let dir = request_path.trim_end_matches('/');
        out.push(format!("{dir}/{index_name}"));
    }
    out
}
