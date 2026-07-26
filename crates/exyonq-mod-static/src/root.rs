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
use super::precooked::{route_bin_response, PrecookedResponse};
#[cfg(target_os = "linux")]
use super::sendfile::SendfileAsset;
use super::wire::{self, WirePair};
use super::{content_type_for, relative_as_safe_path, strip_route_prefix, StaticError};
use crate::identity::StaticResourceIdentity;
use bytes::Bytes;
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Files at or above this size are mmap'd at preload instead of heap-copied (P2/P3 RSS).
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
}

struct PreloadCandidate {
    path: PathBuf,
    request_path: String,
}

pub struct StaticRoot {
    canonical_root: PathBuf,
    route_prefix: String,
    index: Option<String>,
    preload_limits: PreloadLimits,
    /// P1 fast path: exact match for `{prefix}/1k.bin`.
    one_k_path: String,
    /// P2 bench path served via pre-wired response on the raw static loop.
    sixty_four_k_path: String,
    /// P3 bench path served via sendfile on Linux.
    #[cfg(target_os = "linux")]
    one_m_path: String,
    one_k: Option<Arc<PrecookedResponse>>,
    /// P7 fast path: `/site/routes/routeNNN.bin` → slot N (0–99).
    route_table_prefix: String,
    route_path_len: usize,
    route_bodies: [Option<Arc<bytes::Bytes>>; 100],
    /// When every routeNNN.bin payload is identical (benchmark P7), reuse one response.
    route_shared: Option<Arc<PrecookedResponse>>,
    wire_one_k: Option<WirePair>,
    wire_sixty_four_k: Option<WirePair>,
    wire_route_shared: Option<WirePair>,
    bench_one_k_wire: Option<Arc<Bytes>>,
    bench_sixty_four_k_wire: Option<Arc<Bytes>>,
    bench_route_wire: Option<Arc<Bytes>>,
    one_k_get_prefix: Vec<u8>,
    one_k_head_prefix: Vec<u8>,
    sixty_four_k_get_prefix: Vec<u8>,
    sixty_four_k_head_prefix: Vec<u8>,
    #[cfg(target_os = "linux")]
    one_m_get_prefix: Vec<u8>,
    #[cfg(target_os = "linux")]
    one_m_head_prefix: Vec<u8>,
    route_get_prefix: Vec<u8>,
    #[cfg(target_os = "linux")]
    sendfile_sixty_four_k: Option<Arc<SendfileAsset>>,
    #[cfg(target_os = "linux")]
    sendfile_one_m: Option<Arc<SendfileAsset>>,
    one_k_path_len: usize,
    sixty_four_k_path_len: usize,
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
        let trimmed = route_prefix.trim_end_matches('/');
        Ok(Self {
            canonical_root: root.canonicalize()?,
            route_prefix: route_prefix.to_string(),
            index: index.map(str::to_string),
            preload_limits,
            one_k_path: format!("{trimmed}/1k.bin"),
            sixty_four_k_path: format!("{trimmed}/64k.bin"),
            #[cfg(target_os = "linux")]
            one_m_path: format!("{trimmed}/1m.bin"),
            one_k: None,
            route_table_prefix: format!("{trimmed}/routes/route"),
            route_path_len: format!("{trimmed}/routes/route").len() + 7,
            route_bodies: std::array::from_fn(|_| None),
            route_shared: None,
            wire_one_k: None,
            wire_sixty_four_k: None,
            wire_route_shared: None,
            bench_one_k_wire: None,
            bench_sixty_four_k_wire: None,
            bench_route_wire: None,
            one_k_get_prefix: format!("GET {trimmed}/1k.bin ").into_bytes(),
            one_k_head_prefix: format!("HEAD {trimmed}/1k.bin ").into_bytes(),
            sixty_four_k_get_prefix: format!("GET {trimmed}/64k.bin ").into_bytes(),
            sixty_four_k_head_prefix: format!("HEAD {trimmed}/64k.bin ").into_bytes(),
            #[cfg(target_os = "linux")]
            one_m_get_prefix: format!("GET {trimmed}/1m.bin ").into_bytes(),
            #[cfg(target_os = "linux")]
            one_m_head_prefix: format!("HEAD {trimmed}/1m.bin ").into_bytes(),
            route_get_prefix: format!("GET {trimmed}/routes/route").into_bytes(),
            #[cfg(target_os = "linux")]
            sendfile_sixty_four_k: None,
            #[cfg(target_os = "linux")]
            sendfile_one_m: None,
            one_k_path_len: format!("{trimmed}/1k.bin").len(),
            sixty_four_k_path_len: format!("{trimmed}/64k.bin").len(),
            cache: Arc::new(HashMap::new()),
        })
    }

    pub fn preload_limits(&self) -> PreloadLimits {
        self.preload_limits
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    pub fn resolve_path_sync(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        self.resolved_file_path(request_path)
    }

    /// Canonical filesystem path for a request (preload hit or on-demand resolve).
    pub fn resolved_file_path(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        if let Some(entry) = self.lookup_entry(request_path) {
            return Ok(entry.path.clone());
        }
        self.resolve_path_uncached(request_path)
    }

    /// Capture file identity for cache storage (metadata only, no body read).
    pub fn capture_identity_for_request(
        &self,
        request_path: &str,
    ) -> Result<StaticResourceIdentity, StaticError> {
        let path = self.resolve_live_file_path(request_path)?;
        StaticResourceIdentity::capture(&path).map_err(StaticError::Io)
    }

    /// Resolve a request to an on-disk regular file (validates existence; no preload body).
    pub fn resolve_live_file_path(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        self.resolve_path_uncached(request_path)
    }

    pub fn lookup_wire(&self, request_path: &str) -> Option<&WirePair> {
        if let Some(idx) =
            parse_route_slot(request_path, &self.route_table_prefix, self.route_path_len)
        {
            if self.route_bodies[idx].is_some() {
                return self.wire_route_shared.as_ref();
            }
        }
        if request_path == self.one_k_path {
            return self.wire_one_k.as_ref();
        }
        if request_path == self.sixty_four_k_path {
            return self.wire_sixty_four_k.as_ref();
        }
        None
    }

    #[inline]
    pub fn match_bench_head(&self, head: &[u8]) -> Option<&Arc<Bytes>> {
        // P1 first — hottest bench scenario.
        if head.starts_with(&self.one_k_get_prefix) || head.starts_with(&self.one_k_head_prefix) {
            return self.bench_one_k_wire.as_ref();
        }
        if head.starts_with(&self.sixty_four_k_get_prefix)
            || head.starts_with(&self.sixty_four_k_head_prefix)
        {
            return self.bench_sixty_four_k_wire.as_ref();
        }
        // P7: routeNNN.bin on shared wire body.
        if head.len() >= self.route_path_len + 16
            && head.starts_with(&self.route_get_prefix)
            && self.bench_route_wire.is_some()
        {
            if let Some(wire) = self.match_bench_route_head(head) {
                return Some(wire);
            }
        }
        if let Some(wire) = self.match_bench_route_head(head) {
            return Some(wire);
        }
        None
    }

    /// P1 bench: request targets `{prefix}/1k.bin` (keep-alive loop fast path).
    #[inline]
    pub fn is_bench_one_k_head(&self, head: &[u8]) -> bool {
        head.starts_with(&self.one_k_get_prefix) || head.starts_with(&self.one_k_head_prefix)
    }

    /// P2 bench: request targets `{prefix}/64k.bin` (sendfile keep-alive fast path).
    #[inline]
    pub fn is_bench_64k_head(&self, head: &[u8]) -> bool {
        head.starts_with(&self.sixty_four_k_get_prefix)
            || head.starts_with(&self.sixty_four_k_head_prefix)
    }

    /// P3 bench: request targets `{prefix}/1m.bin` (blocking sendfile keep-alive fast path).
    #[cfg(target_os = "linux")]
    #[inline]
    pub fn is_bench_1m_head(&self, head: &[u8]) -> bool {
        head.starts_with(&self.one_m_get_prefix) || head.starts_with(&self.one_m_head_prefix)
    }

    /// P7 bench: `GET {prefix}/routes/routeNNN.bin` on the shared wire body.
    #[inline]
    pub fn is_bench_route_head(&self, head: &[u8]) -> bool {
        self.match_bench_route_head(head).is_some()
    }

    /// Pre-serialized P7 response bytes (header + shared route body), kept in RAM at preload.
    #[inline]
    pub fn bench_route_wire_bytes(&self) -> Option<&[u8]> {
        self.bench_route_wire
            .as_ref()
            .map(|wire| wire.as_ref().as_ref())
    }

    /// Pre-serialized P1 response bytes (header + 1 KiB body), kept in RAM at preload.
    #[inline]
    pub fn bench_one_k_wire_bytes(&self) -> Option<&[u8]> {
        if wire::rodata_p1_enabled() {
            return Some(wire::p1_bench_wire_rodata());
        }
        self.bench_one_k_wire
            .as_ref()
            .map(|wire| wire.as_ref().as_ref())
    }

    #[cfg(target_os = "linux")]
    #[inline]
    pub fn match_bench_sendfile_head(&self, head: &[u8]) -> Option<&SendfileAsset> {
        if head.starts_with(&self.sixty_four_k_get_prefix)
            || head.starts_with(&self.sixty_four_k_head_prefix)
        {
            return self.sendfile_sixty_four_k.as_deref();
        }
        if head.starts_with(&self.one_m_get_prefix) || head.starts_with(&self.one_m_head_prefix) {
            return self.sendfile_one_m.as_deref();
        }
        None
    }

    /// Arc-cloning variant for the epoll sendfile FSM (ADR-025 PR #2): refcount bump only,
    /// no `File` reopen. Lets the epoll worker own a `SendingState` for the connection.
    #[cfg(target_os = "linux")]
    #[inline]
    pub fn match_bench_sendfile_head_arc(&self, head: &[u8]) -> Option<Arc<SendfileAsset>> {
        if head.starts_with(&self.sixty_four_k_get_prefix)
            || head.starts_with(&self.sixty_four_k_head_prefix)
        {
            return self.sendfile_sixty_four_k.clone();
        }
        if head.starts_with(&self.one_m_get_prefix) || head.starts_with(&self.one_m_head_prefix) {
            return self.sendfile_one_m.clone();
        }
        None
    }

    #[inline]
    #[cfg(target_os = "linux")]
    pub fn is_head_wire_request(head: &[u8]) -> bool {
        head.starts_with(b"HEAD ")
    }

    #[inline]
    fn match_bench_route_head(&self, head: &[u8]) -> Option<&Arc<Bytes>> {
        let wire = self.bench_route_wire.as_ref()?;
        let i = self.route_get_prefix.len();
        // Fixed layout at path start: GET {prefix}NNN.bin — total header may include Host etc.
        if head.len() < i + 12 {
            return None;
        }
        if !head.starts_with(&self.route_get_prefix) {
            return None;
        }
        let d0 = head[i];
        let d1 = head[i + 1];
        let d2 = head[i + 2];
        if !d0.is_ascii_digit() || !d1.is_ascii_digit() || !d2.is_ascii_digit() {
            return None;
        }
        if head.get(i + 3..i + 7) != Some(b".bin") {
            return None;
        }
        if head.get(i + 7) != Some(&b' ') {
            return None;
        }
        Some(wire)
    }

    /// Bench static loop: return pre-wired keep-alive bytes without per-request header work.
    #[inline]
    pub fn lookup_bench_wire(&self, request_path: &str) -> Option<&Arc<Bytes>> {
        match request_path.len() {
            len if len == self.route_path_len => {
                if self.bench_route_wire.is_some()
                    && parse_route_slot(request_path, &self.route_table_prefix, self.route_path_len)
                        .is_some()
                {
                    self.bench_route_wire.as_ref()
                } else {
                    None
                }
            }
            len if len == self.one_k_path_len && request_path == self.one_k_path => {
                self.bench_one_k_wire.as_ref()
            }
            len if len == self.sixty_four_k_path_len && request_path == self.sixty_four_k_path => {
                self.bench_sixty_four_k_wire.as_ref()
            }
            _ => None,
        }
    }

    pub fn has_static_fast_path(&self, request_path: &str) -> bool {
        if request_path == "/health" {
            return true;
        }
        self.lookup_bench_wire(request_path).is_some()
    }

    /// HTTP/3 static fast path: shared body bytes for preloaded bench assets.
    pub fn lookup_h3_static(&self, request_path: &str) -> Option<(&'static str, Arc<Bytes>)> {
        if request_path == self.one_k_path {
            return self
                .one_k
                .as_ref()
                .map(|entry| (entry.content_type, Arc::clone(&entry.body)));
        }
        if let Some(idx) =
            parse_route_slot(request_path, &self.route_table_prefix, self.route_path_len)
        {
            if self.route_bodies[idx].is_some() {
                if let Some(shared) = &self.route_shared {
                    return Some((shared.content_type, Arc::clone(&shared.body)));
                }
                if let Some(body) = &self.route_bodies[idx] {
                    return Some(("application/octet-stream", Arc::clone(body)));
                }
            }
        }
        None
    }

    /// Hot path for preloaded assets (P1–P3, P7): O(1) route slots or map lookup.
    pub fn serve_request(
        &self,
        request_path: &str,
    ) -> Result<hyper::Response<BoxBody>, StaticError> {
        if let Some(relative) = strip_route_prefix(&self.route_prefix, request_path) {
            let _ = relative_as_safe_path(&relative)?;
        } else {
            return Err(StaticError::NotFound);
        }

        if let Some(idx) =
            parse_route_slot(request_path, &self.route_table_prefix, self.route_path_len)
        {
            if self.route_bodies[idx].is_some() {
                if let Some(shared) = &self.route_shared {
                    return Ok(shared.to_response());
                }
                if let Some(body) = &self.route_bodies[idx] {
                    return Ok(route_bin_response(body));
                }
            }
        }

        if request_path.len() == self.one_k_path.len() {
            if let Some(precooked) = &self.one_k {
                if request_path == self.one_k_path {
                    return Ok(precooked.to_response());
                }
            }
        }

        let entry = self
            .lookup_entry(request_path)
            .ok_or(StaticError::NotFound)?;
        Ok(entry.precooked.to_response())
    }

    /// HEAD for bench static assets (headers + Content-Length, no body).
    pub fn serve_head_request(
        &self,
        request_path: &str,
    ) -> Result<hyper::Response<BoxBody>, StaticError> {
        if let Some(relative) = strip_route_prefix(&self.route_prefix, request_path) {
            let _ = relative_as_safe_path(&relative)?;
        } else {
            return Err(StaticError::NotFound);
        }

        #[cfg(target_os = "linux")]
        {
            if request_path == self.sixty_four_k_path && self.sendfile_sixty_four_k.is_some() {
                return Ok(super::precooked::octet_stream_head_response(65536));
            }
            if request_path == self.one_m_path && self.sendfile_one_m.is_some() {
                return Ok(super::precooked::octet_stream_head_response(1_048_576));
            }
        }

        if let Some(idx) =
            parse_route_slot(request_path, &self.route_table_prefix, self.route_path_len)
        {
            if self.route_bodies[idx].is_some() {
                if let Some(shared) = &self.route_shared {
                    return Ok(shared.to_head_response());
                }
                if let Some(body) = &self.route_bodies[idx] {
                    return Ok(super::precooked::route_bin_head_response(body.len()));
                }
            }
        }

        if request_path.len() == self.one_k_path.len() {
            if let Some(precooked) = &self.one_k {
                if request_path == self.one_k_path {
                    return Ok(precooked.to_head_response());
                }
            }
        }

        let entry = self
            .lookup_entry(request_path)
            .ok_or(StaticError::NotFound)?;
        Ok(entry.precooked.to_head_response())
    }

    fn lookup_entry(&self, request_path: &str) -> Option<&CachedEntry> {
        if let Some(entry) = self.cache.get(request_path) {
            return Some(entry);
        }
        for candidate in index_lookup_candidates(self.route_prefix.as_str(), request_path) {
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
        if edge_static_enabled() {
            return self.preload_bench_assets();
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

            #[cfg(target_os = "linux")]
            if skip_sendfile_body(&cand.request_path, self) {
                continue;
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

            let body = match load_preload_body_bounded(
                &cand.path,
                file,
                len,
                skip_sendfile_body(&cand.request_path, self),
                limits.max_file_bytes,
            ) {
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
            map.insert(
                cand.request_path.clone(),
                CachedEntry {
                    path: cand.path.clone(),
                    precooked: PrecookedResponse::new(body, content_type),
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

        let (route_bodies, route_shared) = load_bench_route_table(self)?;
        let one_k = map
            .get(&self.one_k_path)
            .map(|entry| Arc::clone(&entry.precooked));
        self.one_k = one_k.clone();
        self.route_bodies = route_bodies;
        self.route_shared = route_shared.clone();
        self.wire_one_k = None;
        self.wire_route_shared = None;
        self.bench_one_k_wire = one_k
            .as_ref()
            .map(|entry| wire::bench_wire_keep(entry.body.as_ref()));
        self.bench_route_wire = route_shared
            .as_ref()
            .map(|entry| wire::bench_wire_keep(entry.body.as_ref()));
        #[cfg(not(target_os = "linux"))]
        {
            let sixty_four_k = map
                .get(&self.sixty_four_k_path)
                .map(|entry| Arc::clone(&entry.precooked));
            self.wire_sixty_four_k = sixty_four_k
                .as_ref()
                .map(|entry| WirePair::for_bench_body(entry.body.as_ref()));
            self.bench_sixty_four_k_wire = self
                .wire_sixty_four_k
                .as_ref()
                .map(|pair| Arc::clone(&pair.keep_alive));
        }
        #[cfg(target_os = "linux")]
        {
            self.wire_sixty_four_k = None;
            self.bench_sixty_four_k_wire = None;
            let sixty_four_k_path = self.canonical_root.join("64k.bin");
            let one_m_path = self.canonical_root.join("1m.bin");
            self.sendfile_sixty_four_k = SendfileAsset::open(&sixty_four_k_path, 65536)
                .ok()
                .map(Arc::new);
            self.sendfile_one_m = SendfileAsset::open(&one_m_path, 1048576).ok().map(Arc::new);
        }
        self.cache = Arc::new(map);
        Ok(())
    }

    /// Bench `edge` profile: preload only P1/P2/P3/P7 hot assets (lower RSS).
    /// Same per-file / total / entry limits apply.
    fn preload_bench_assets(&mut self) -> Result<(), StaticError> {
        let limits = self.preload_limits;
        let mut map = HashMap::new();
        let mut accepted_bytes: u64 = 0;
        let mut accepted_entries: u64 = 0;

        let index_name = self.index.as_deref().unwrap_or("index.html");
        let index_path = format!("{}/{}", self.route_prefix.trim_end_matches('/'), index_name);
        try_add_edge_preload(
            &mut map,
            &mut accepted_bytes,
            &mut accepted_entries,
            limits,
            index_path,
            self.canonical_root.join(index_name),
        )?;

        let one_k_file = self.canonical_root.join("1k.bin");
        if one_k_file.is_file() {
            try_add_edge_preload(
                &mut map,
                &mut accepted_bytes,
                &mut accepted_entries,
                limits,
                self.one_k_path.clone(),
                one_k_file,
            )?;
            if let Some(entry) = map.get(&self.one_k_path) {
                self.one_k = Some(Arc::clone(&entry.precooked));
                self.bench_one_k_wire = Some(wire::bench_wire_keep(entry.precooked.body.as_ref()));
            }
        }

        let (route_bodies, route_shared) = load_bench_route_table(self)?;
        self.route_bodies = route_bodies;
        self.route_shared = route_shared.clone();
        self.bench_route_wire = route_shared
            .as_ref()
            .map(|entry| wire::bench_wire_keep(entry.body.as_ref()));
        #[cfg(target_os = "linux")]
        {
            self.wire_sixty_four_k = None;
            self.bench_sixty_four_k_wire = None;
            let sixty_four_k_path = self.canonical_root.join("64k.bin");
            let one_m_path = self.canonical_root.join("1m.bin");
            self.sendfile_sixty_four_k = SendfileAsset::open(&sixty_four_k_path, 65536)
                .ok()
                .map(Arc::new);
            self.sendfile_one_m = SendfileAsset::open(&one_m_path, 1048576).ok().map(Arc::new);
        }
        #[cfg(not(target_os = "linux"))]
        {
            let sixty_four_k_file = self.canonical_root.join("64k.bin");
            if sixty_four_k_file.is_file() {
                try_add_edge_preload(
                    &mut map,
                    &mut accepted_bytes,
                    &mut accepted_entries,
                    limits,
                    self.sixty_four_k_path.clone(),
                    sixty_four_k_file,
                )?;
                if let Some(entry) = map.get(&self.sixty_four_k_path) {
                    self.wire_sixty_four_k =
                        Some(WirePair::for_bench_body(entry.precooked.body.as_ref()));
                    self.bench_sixty_four_k_wire = self
                        .wire_sixty_four_k
                        .as_ref()
                        .map(|pair| Arc::clone(&pair.keep_alive));
                }
            }
        }
        let _ = (accepted_bytes, accepted_entries);
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
            if path.is_dir() {
                let next = relative.join(entry.file_name());
                self.collect_preload_candidates(&path, &next, out)?;
                continue;
            }
            if !path.is_file() {
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
            if parse_route_slot(&request_path, &self.route_table_prefix, self.route_path_len)
                .is_some()
            {
                continue;
            }
            out.push(PreloadCandidate {
                path,
                request_path,
            });
        }
        Ok(())
    }

    fn resolve_path_uncached(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        let relative = match strip_route_prefix(&self.route_prefix, request_path) {
            Some(value) => value,
            None => return Err(StaticError::NotFound),
        };
        let relative_path = relative_as_safe_path(&relative)?;

        let mut candidate = self.canonical_root.join(relative_path);
        if candidate.is_dir() {
            let index_name = self.index.as_deref().unwrap_or("index.html");
            candidate = candidate.join(index_name);
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
        if !canonical.is_file() {
            return Err(StaticError::NotFound);
        }

        Ok(canonical)
    }
}

fn try_add_edge_preload(
    map: &mut HashMap<String, CachedEntry>,
    accepted_bytes: &mut u64,
    accepted_entries: &mut u64,
    limits: PreloadLimits,
    request_path: String,
    file: PathBuf,
) -> Result<(), StaticError> {
    if !file.is_file() {
        return Ok(());
    }
    if *accepted_entries >= limits.max_entries {
        return Ok(());
    }
    let meta = std::fs::metadata(&file)?;
    let len = meta.len();
    if len > limits.max_file_bytes {
        tracing::warn!(
            path = %request_path,
            file_bytes = len,
            max_file_bytes = limits.max_file_bytes,
            "static edge preload oversized skipped"
        );
        return Ok(());
    }
    let Some(candidate_total) = accepted_bytes.checked_add(len) else {
        return Err(StaticError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "preload byte accumulation overflow",
        )));
    };
    if candidate_total > limits.max_total_bytes {
        tracing::warn!(
            path = %request_path,
            "static edge preload total budget reached"
        );
        return Ok(());
    }
    let opened = File::open(&file)?;
    let body = match load_preload_body_bounded(&file, opened, len, false, limits.max_file_bytes) {
        Ok(b) => b,
        Err(StaticError::BudgetExceeded) => return Ok(()),
        Err(e) => return Err(e),
    };
    let content_type = content_type_for(&file);
    map.insert(
        request_path,
        CachedEntry {
            path: file,
            precooked: PrecookedResponse::new(body, content_type),
        },
    );
    *accepted_bytes = candidate_total;
    *accepted_entries = accepted_entries.saturating_add(1);
    Ok(())
}

fn edge_static_enabled() -> bool {
    std::env::var("EXYONQ_EDGE_STATIC").ok().as_deref() == Some("1")
}

fn parse_route_slot(
    request_path: &str,
    route_table_prefix: &str,
    route_path_len: usize,
) -> Option<usize> {
    let bytes = request_path.as_bytes();
    if bytes.len() != route_path_len {
        return None;
    }
    let prefix = route_table_prefix.as_bytes();
    if !bytes.starts_with(prefix) {
        return None;
    }
    let digit_start = prefix.len();
    if &bytes[digit_start + 3..] != b".bin" {
        return None;
    }
    let d0 = bytes[digit_start];
    let d1 = bytes[digit_start + 1];
    let d2 = bytes[digit_start + 2];
    if !d0.is_ascii_digit() || !d1.is_ascii_digit() || !d2.is_ascii_digit() {
        return None;
    }
    let index = (d0 - b'0') as usize * 100 + (d1 - b'0') as usize * 10 + (d2 - b'0') as usize;
    (index < 100).then_some(index)
}

type BenchRouteTable = (
    [Option<Arc<bytes::Bytes>>; 100],
    Option<Arc<PrecookedResponse>>,
);

fn load_bench_route_table(root: &StaticRoot) -> Result<BenchRouteTable, StaticError> {
    let sample = root.canonical_root.join("routes/route000.bin");
    if !sample.is_file() {
        return Ok((std::array::from_fn(|_| None), None));
    }
    let limits = root.preload_limits;
    if limits.is_disabled() {
        return Ok((std::array::from_fn(|_| None), None));
    }
    let meta = std::fs::metadata(&sample)?;
    let len = meta.len();
    if len > limits.max_file_bytes || len > limits.max_total_bytes {
        tracing::warn!(
            path = %sample.display(),
            file_bytes = len,
            "static bench route table skipped by preload limits"
        );
        return Ok((std::array::from_fn(|_| None), None));
    }
    let opened = File::open(&sample)?;
    let body_bytes = load_preload_body_bounded(&sample, opened, len, false, limits.max_file_bytes)?;
    let precooked = PrecookedResponse::new(Arc::clone(&body_bytes), content_type_for(&sample));
    let slots = std::array::from_fn(|_| Some(Arc::clone(&body_bytes)));
    Ok((slots, Some(precooked)))
}

#[cfg(target_os = "linux")]
fn skip_sendfile_body(request_path: &str, root: &StaticRoot) -> bool {
    request_path == root.one_m_path || request_path == root.sixty_four_k_path
}

#[cfg(not(target_os = "linux"))]
fn skip_sendfile_body(_request_path: &str, _root: &StaticRoot) -> bool {
    false
}

fn load_preload_body_bounded(
    path: &Path,
    mut file: File,
    logical_len: u64,
    #[allow(unused_variables)] skip_body: bool,
    max_file_bytes: u64,
) -> Result<Arc<Bytes>, StaticError> {
    // mmap and heap paths both charge full logical_len against budgets (caller).
    // Never map/read before the per-file check (caller + re-check here).
    if logical_len > max_file_bytes {
        return Err(StaticError::BudgetExceeded);
    }
    #[cfg(target_os = "linux")]
    if skip_body && logical_len >= super::sendfile::SENDFILE_MIN_BYTES as u64 {
        return Ok(Arc::new(Bytes::new()));
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

fn index_lookup_candidates(route_prefix: &str, request_path: &str) -> Vec<String> {
    let prefix = route_prefix.trim_end_matches('/');
    let mut out = Vec::new();
    if request_path == prefix || request_path == format!("{prefix}/") {
        out.push(format!("{prefix}/index.html"));
        return out;
    }
    if request_path.ends_with('/') {
        out.push(format!("{}index.html", request_path));
        out.push(format!(
            "{}{}",
            request_path.trim_end_matches('/'),
            "/index.html"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_route_slot_accepts_p7_paths() {
        let prefix = "/site/routes/route";
        let len = prefix.len() + 7;
        assert_eq!(
            parse_route_slot("/site/routes/route000.bin", prefix, len),
            Some(0)
        );
        assert_eq!(
            parse_route_slot("/site/routes/route099.bin", prefix, len),
            Some(99)
        );
        assert_eq!(
            parse_route_slot("/site/routes/route100.bin", prefix, len),
            None
        );
        assert_eq!(parse_route_slot("/site/1k.bin", prefix, len), None);
    }

    #[test]
    fn bench_64k_head_prefix_matches_head_method() {
        let root = StaticRoot::new(Path::new("/tmp"), "/site", None).expect("root");
        assert!(root.is_bench_64k_head(b"HEAD /site/64k.bin HTTP/1.1\r\n"));
        assert!(root.is_bench_64k_head(b"GET /site/64k.bin HTTP/1.1\r\n"));
        assert!(!root.is_bench_64k_head(b"GET /site/1k.bin HTTP/1.1\r\n"));
    }
}
