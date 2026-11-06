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
use super::{content_type_for, relative_as_safe_path, sensitive, strip_route_prefix, StaticError};
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

struct PreloadCandidate {
    path: PathBuf,
    request_path: String,
}

pub struct StaticRoot {
    canonical_root: PathBuf,
    /// Held directory fd for Cap004 openat2(RESOLVE_BENEATH) opens (Linux).
    #[cfg(target_os = "linux")]
    /// `None` for file roots (NGINX `alias` to a file under `location =`):
    /// those are Hyper-only — the wire/sendfile layer never sees them.
    root_dir: Option<File>,
    /// NGINX `alias /path/file` in `location =`: canonical_root IS the file.
    file_root: bool,
    route_prefix: String,
    /// IR `match.host`. `None` matches any request Host.
    route_host: Option<String>,
    #[cfg(target_os = "linux")]
    listen_port: Option<u16>,
    /// EXY-03: `true` = NGINX `root` semantics — `root + full URI`.
    /// `false` = alias — `root + (URI − route_prefix)`.
    join_uri: bool,
    /// EXY-03: `true` = NGINX literal-prefix matching — `route_prefix` claims
    /// any URI starting with it verbatim (no segment boundary).
    literal: bool,
    /// EXY-03: `true` = `301` to `URI/` when a directory is requested without
    /// a trailing slash (NGINX-style).
    dir_redirect: bool,
    /// EXY-03: `true` = the prefix claims only `URI == route_prefix` verbatim
    /// (NGINX `location =`). Takes precedence over `literal`.
    exact: bool,
    /// EXY-03 redirect `Location` shaping (NGINX `absolute_redirect` /
    /// `port_in_redirect` / `server_name_in_redirect`).
    redirect_policy: DirRedirectPolicy,
    index: Option<String>,
    preload_limits: PreloadLimits,
    cache: Arc<HashMap<String, CachedEntry>>,
}

/// EXY-03: how a directory `301` builds `Location` (NGINX parity).
#[derive(Debug, Clone)]
pub struct DirRedirectPolicy {
    /// `true` (NGINX `absolute_redirect on`) = absolute `Location`
    /// `scheme://authority/path/`; `false` = relative `path/`.
    pub absolute: bool,
    /// `true` (NGINX `port_in_redirect on`) = emit the listener port unless it
    /// is the scheme default; `false` = never emit a port.
    pub include_port: bool,
    /// NGINX `server_name_in_redirect on` — fixed host replacing the request
    /// authority host part in `Location`.
    pub fixed_host: Option<String>,
}

impl Default for DirRedirectPolicy {
    fn default() -> Self {
        Self {
            absolute: true,
            include_port: true,
            fixed_host: None,
        }
    }
}

/// EXY-03: host part of an HTTP authority (`host[:port]`, userinfo ignored).
fn authority_host_part(authority: &str) -> &str {
    let after_user = authority.rsplit('@').next().unwrap_or(authority);
    match after_user.split_once(':') {
        Some((host, _)) => host,
        None => after_user,
    }
}

/// EXY-03: default port for a scheme (NGINX `port_in_redirect` omits it).
fn scheme_default_port(scheme: &str) -> u16 {
    if scheme == "https" {
        443
    } else {
        80
    }
}

/// EXY-03: per-route matching + redirect options (NGINX-imported semantics).
/// `Default` = native ExyonQ behaviour (segment-boundary prefix, no redirect).
#[derive(Debug, Clone, Default)]
pub struct StaticRootRouteOpts {
    pub join_uri: bool,
    pub literal: bool,
    pub exact: bool,
    pub dir_redirect: bool,
    pub redirect_policy: DirRedirectPolicy,
    /// EXY-03: NGINX listener scope — `Some` slots never serve on the wire path.
    pub listen_port: Option<u16>,
}

impl StaticRoot {
    pub fn new(root: &Path, route_prefix: &str, index: Option<&str>) -> Result<Self, StaticError> {
        Self::new_with_preload_limits(
            root,
            route_prefix,
            index,
            PreloadLimits::DEFAULT,
            StaticRootRouteOpts::default(),
        )
    }

    /// EXY-03: alias for NGINX-`root` mode (`root + full URI`).
    pub fn new_join_uri(
        root: &Path,
        route_prefix: &str,
        index: Option<&str>,
    ) -> Result<Self, StaticError> {
        Self::new_with_preload_limits(
            root,
            route_prefix,
            index,
            PreloadLimits::DEFAULT,
            StaticRootRouteOpts {
                join_uri: true,
                ..Default::default()
            },
        )
    }

    pub fn new_with_preload_limits(
        root: &Path,
        route_prefix: &str,
        index: Option<&str>,
        preload_limits: PreloadLimits,
        opts: StaticRootRouteOpts,
    ) -> Result<Self, StaticError> {
        let canonical_root = root.canonicalize()?;
        // NGINX `alias` to a regular file (`location = /x { alias file; }`):
        // the root is the file itself — no dirfd, no tree walk, wire-invisible.
        let file_root = canonical_root.is_file();
        if file_root {
            let file_name = canonical_root
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(StaticError::SensitivePath)?;
            sensitive::ensure_relative_allowed(file_name)?;
        }
        #[cfg(target_os = "linux")]
        let root_dir = if file_root {
            None
        } else {
            Some(open_root_dirfd(&canonical_root)?)
        };
        Ok(Self {
            canonical_root,
            #[cfg(target_os = "linux")]
            root_dir,
            route_prefix: route_prefix.to_string(),
            route_host: None,
            file_root,
            #[cfg(target_os = "linux")]
            listen_port: opts.listen_port,
            join_uri: opts.join_uri,
            literal: opts.literal,
            dir_redirect: opts.dir_redirect,
            exact: opts.exact,
            redirect_policy: opts.redirect_policy,
            index: index.map(str::to_string),
            preload_limits,
            cache: Arc::new(HashMap::new()),
        })
    }

    /// Request-path → root-relative path under the active mapping mode.
    fn relative_for(&self, request_path: &str) -> Option<String> {
        if self.join_uri {
            // NGINX `root`: file = root + full request URI.
            Some(request_path.trim_start_matches('/').to_string())
        } else if self.exact || self.literal {
            // NGINX `alias`: replace the matched prefix verbatim — under
            // `location =` the whole URI is consumed, under literal prefix the
            // verbatim remainder joins (`/assets-extra` → `-extra`).
            let prefix = self.route_prefix.trim_end_matches('/');
            let prefix = if prefix.is_empty() { "/" } else { prefix };
            request_path
                .strip_prefix(prefix)
                .map(|rest| rest.trim_start_matches('/').to_string())
        } else {
            strip_route_prefix(&self.route_prefix, request_path)
        }
    }

    /// H-01: classify the root-relative path before any cache or filesystem use.
    fn checked_relative(&self, request_path: &str) -> Result<String, StaticError> {
        let relative = self
            .relative_for(request_path)
            .ok_or(StaticError::NotFound)?;
        sensitive::ensure_relative_allowed(&relative)?;
        Ok(relative)
    }

    /// Wire/sendfile entry points need the same policy without exposing the
    /// relative path machinery across modules. Linux-only today: the Hyper
    /// path uses `resolved_file_path`/`serve_request` directly.
    #[cfg(target_os = "linux")]
    pub(crate) fn ensure_request_allowed(&self, request_path: &str) -> Result<(), StaticError> {
        self.checked_relative(request_path).map(|_| ())
    }

    /// EXY-03: NGINX-style directory redirect. When `dir_redirect` is on and
    /// the request resolves to a directory while lacking a trailing `/`,
    /// returns the `Location` target. NGINX `absolute_redirect` default emits
    /// an absolute URI `scheme://host:port/path/`; when no authority source
    /// exists we fall back to the relative `path/` form.
    pub fn dir_redirect_location(
        &self,
        request_path: &str,
        host: Option<&str>,
        scheme: &str,
        request_query: Option<&str>,
        local_port: Option<u16>,
    ) -> Option<String> {
        if !self.dir_redirect || request_path.ends_with('/') {
            return None;
        }
        let relative = self.checked_relative(request_path).ok()?;
        let candidate = self
            .canonical_root
            .join(relative_as_safe_path(&relative).ok()?);
        if !candidate.is_dir() {
            return None;
        }
        let suffix = request_query.map_or_else(String::new, |q| format!("?{q}"));
        if !self.redirect_policy.absolute {
            // NGINX `absolute_redirect off`: path-only Location.
            return Some(format!("{request_path}/{suffix}"));
        }
        let Some(host_part) = self
            .redirect_policy
            .fixed_host
            .as_deref()
            .or_else(|| host.map(authority_host_part))
        else {
            // Absolute requested but no authority source: relative fallback.
            return Some(format!("{request_path}/{suffix}"));
        };
        let port = match (self.redirect_policy.include_port, local_port) {
            (true, Some(p)) if p != scheme_default_port(scheme) => format!(":{p}"),
            _ => String::new(),
        };
        Some(format!(
            "{scheme}://{host_part}{port}{request_path}/{suffix}"
        ))
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
    /// `None` for file roots — they never reach the wire/sendfile layer.
    #[cfg(target_os = "linux")]
    #[inline]
    pub fn root_dir_fd(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        use std::os::fd::AsFd;
        self.root_dir.as_ref().map(|f| f.as_fd())
    }

    /// `true` when `canonical_root` is a regular file (NGINX `alias` file
    /// under `location =`) — Hyper-only; wire/sendfile must not see it.
    #[cfg(target_os = "linux")]
    pub(crate) fn file_root(&self) -> bool {
        self.file_root
    }

    pub fn resolve_path_sync(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        self.resolved_file_path(request_path)
    }

    /// Canonical filesystem path for a request (preload hit or on-demand resolve).
    pub fn resolved_file_path(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        let relative = self.checked_relative(request_path)?;
        if let Some(entry) = self.lookup_entry(&relative) {
            return Ok(entry.path.clone());
        }
        self.resolve_relative_uncached(&relative)
    }

    /// Cap067: generation-scoped prepared validators/headers when request is preloaded.
    pub fn prepared_wire(&self, request_path: &str) -> Option<&PreparedStaticWire> {
        let relative = self.checked_relative(request_path).ok()?;
        Some(&self.lookup_entry(&relative)?.prepared)
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
        let relative = self.checked_relative(request_path).ok()?;
        let entry = self.lookup_entry(&relative)?;
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
        let relative = self.checked_relative(request_path)?;
        let _ = relative_as_safe_path(&relative)?;
        let entry = self.lookup_entry(&relative).ok_or(StaticError::NotFound)?;
        Ok(entry.precooked.to_response())
    }

    /// HEAD for static assets (headers + Content-Length, no body).
    pub fn serve_head_request(
        &self,
        request_path: &str,
    ) -> Result<hyper::Response<BoxBody>, StaticError> {
        let relative = self.checked_relative(request_path)?;
        let _ = relative_as_safe_path(&relative)?;
        let entry = self.lookup_entry(&relative).ok_or(StaticError::NotFound)?;
        Ok(entry.precooked.to_head_response())
    }

    fn lookup_entry(&self, relative: &str) -> Option<&CachedEntry> {
        if let Some(entry) = self.cache.get(relative) {
            return Some(entry);
        }
        let index_name = self.index.as_deref().unwrap_or("index.html");
        // Cache keys are always root-relative, so a preload can be shared by
        // routes with different prefixes/hosts over the same filesystem root.
        if relative.is_empty() || relative.ends_with('/') {
            let dir = relative.trim_end_matches('/');
            let candidate = if dir.is_empty() {
                index_name.to_string()
            } else {
                format!("{dir}/{index_name}")
            };
            if sensitive::ensure_relative_allowed(&candidate).is_err() {
                return None;
            }
            return self.cache.get(&candidate);
        }
        None
    }

    /// Reuse an already-built tree when another route resolves the same root.
    pub(crate) fn share_preload_cache_from(&mut self, other: &StaticRoot) {
        self.cache = Arc::clone(&other.cache);
    }

    /// Warm path: load files under the document root into an immutable cache (startup/reload).
    ///
    /// Bounded by [`PreloadLimits`]: per-file, total logical bytes, and entry count.
    /// Selection order is deterministic UTF-8 path order. Oversized / budget-skipped files
    /// remain available via on-demand resolve (not 404).
    pub fn preload_tree(&mut self) -> Result<(), StaticError> {
        // File roots (`alias` to a file): nothing to walk — the single file is
        // served through the live resolve path.
        if self.file_root {
            return Ok(());
        }
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
            let rel = relative.join(entry.file_name());
            let Some(rel_str) = rel.to_str() else {
                continue;
            };
            if sensitive::ensure_relative_allowed(rel_str).is_err() {
                continue;
            }
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
            // Classify the canonical target too: a symlink with a harmless name
            // must not smuggle `.env` or `config.php` into the preload cache.
            let Ok(canonical_rel) = canonical.strip_prefix(&self.canonical_root) else {
                continue;
            };
            let Some(canonical_rel_str) = canonical_rel.to_str() else {
                continue;
            };
            if sensitive::ensure_relative_allowed(canonical_rel_str).is_err() {
                continue;
            }
            if canonical.is_dir() {
                self.collect_preload_candidates(&canonical, &rel, out)?;
                continue;
            }
            if !canonical.is_file() {
                continue;
            }
            out.push(PreloadCandidate {
                path: canonical,
                request_path: rel_str.to_string(),
            });
        }
        Ok(())
    }

    fn resolve_path_uncached(&self, request_path: &str) -> Result<PathBuf, StaticError> {
        let relative = self.checked_relative(request_path)?;
        self.resolve_relative_uncached(&relative)
    }

    fn resolve_relative_uncached(&self, relative: &str) -> Result<PathBuf, StaticError> {
        let relative_path = relative_as_safe_path(relative)?;

        // Empty remainder (exact `location =` file alias): `join("")` would
        // append a trailing separator and ENOTDIR on regular-file roots.
        let mut candidate = if relative_path.as_os_str().is_empty() {
            self.canonical_root.clone()
        } else {
            self.canonical_root.join(relative_path)
        };
        // Index directory: follow in-root dir symlinks (historical semantics), then
        // canonicalize + root prefix. Regular-file confirmation is deferred to open.
        match std::fs::metadata(&candidate) {
            Ok(meta) if meta.is_dir() => {
                let index_name = self.index.as_deref().unwrap_or("index.html");
                sensitive::ensure_relative_allowed(index_name)?;
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
        if self.file_root {
            sensitive::ensure_file_name_allowed(&canonical)?;
        }
        if let Ok(canonical_rel) = canonical.strip_prefix(&self.canonical_root) {
            if let Some(rel) = canonical_rel.to_str() {
                sensitive::ensure_relative_allowed(rel)?;
            }
        }
        // Regular-file confirmation deferred to open(O_NOFOLLOW) at the I/O boundary.
        Ok(canonical)
    }

    pub(crate) fn set_route_host(&mut self, host: Option<String>) {
        self.route_host = host;
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn literal_prefix(&self) -> bool {
        self.literal
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn exact_match(&self) -> bool {
        self.exact
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn route_prefix(&self) -> &str {
        &self.route_prefix
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn route_host(&self) -> Option<&str> {
        self.route_host.as_deref()
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn listen_port(&self) -> Option<u16> {
        self.listen_port
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_root(
        dir_redirect: bool,
        literal: bool,
        join_uri: bool,
    ) -> (tempfile::TempDir, StaticRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("assets")).expect("mkdir");
        let root = StaticRoot::new_with_preload_limits(
            dir.path(),
            "/assets",
            Some("index.html"),
            PreloadLimits::DEFAULT,
            StaticRootRouteOpts {
                join_uri,
                literal,
                exact: false,
                dir_redirect,
                redirect_policy: DirRedirectPolicy::default(),
                listen_port: None,
            },
        )
        .expect("root");
        (dir, root)
    }

    #[test]
    fn dir_redirect_absolute_location_scheme_authority_query() {
        let (_t, root) = dir_root(true, false, false);
        assert_eq!(
            root.dir_redirect_location(
                "/assets",
                Some("ex.example:8443"),
                "https",
                Some("a=1&b=2"),
                Some(8443),
            )
            .as_deref(),
            Some("https://ex.example:8443/assets/?a=1&b=2")
        );
        assert_eq!(
            root.dir_redirect_location("/assets", Some("ex.example"), "http", None, None)
                .as_deref(),
            Some("http://ex.example/assets/")
        );
    }

    #[test]
    fn dir_redirect_no_host_falls_back_to_relative() {
        let (_t, root) = dir_root(true, false, false);
        assert_eq!(
            root.dir_redirect_location("/assets", None, "http", None, None)
                .as_deref(),
            Some("/assets/")
        );
    }

    #[test]
    fn dir_redirect_off_or_slash_or_file_is_none() {
        let (t_off, off) = dir_root(false, false, false);
        assert_eq!(
            off.dir_redirect_location("/assets", Some("h"), "http", None, None),
            None
        );
        std::fs::create_dir_all(t_off.path().join("f")).expect("mkdir");
        std::fs::write(t_off.path().join("f/x"), b"x").expect("write");
        let (_t, on) = dir_root(true, false, false);
        assert_eq!(
            on.dir_redirect_location("/assets/", Some("h"), "http", None, None),
            None
        );
        assert_eq!(
            on.dir_redirect_location("/assets/miss", Some("h"), "http", None, None),
            None
        );
    }

    fn dir_root_policy(policy: DirRedirectPolicy) -> (tempfile::TempDir, StaticRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("assets")).expect("mkdir");
        let root = StaticRoot::new_with_preload_limits(
            dir.path(),
            "/assets",
            Some("index.html"),
            PreloadLimits::DEFAULT,
            StaticRootRouteOpts {
                dir_redirect: true,
                redirect_policy: policy,
                listen_port: None,
                ..Default::default()
            },
        )
        .expect("root");
        (dir, root)
    }

    #[test]
    fn dir_redirect_absolute_off_emits_relative() {
        // NGINX `absolute_redirect off`: Location is path-only, query kept.
        let (_t, root) = dir_root_policy(DirRedirectPolicy {
            absolute: false,
            ..Default::default()
        });
        assert_eq!(
            root.dir_redirect_location(
                "/assets",
                Some("h.example:8080"),
                "http",
                Some("q=1"),
                Some(8080),
            )
            .as_deref(),
            Some("/assets/?q=1")
        );
    }

    #[test]
    fn dir_redirect_port_in_redirect_off_omits_port() {
        // NGINX `port_in_redirect off`: authority without any port.
        let (_t, root) = dir_root_policy(DirRedirectPolicy {
            include_port: false,
            ..Default::default()
        });
        assert_eq!(
            root.dir_redirect_location(
                "/assets",
                Some("h.example:9999"),
                "http",
                None,
                Some(8081),
            )
            .as_deref(),
            Some("http://h.example/assets/")
        );
    }

    #[test]
    fn dir_redirect_server_name_in_redirect_overrides_host() {
        // NGINX `server_name_in_redirect on`: configured name replaces Host.
        let (_t, root) = dir_root_policy(DirRedirectPolicy {
            fixed_host: Some("nameinred.example".into()),
            ..Default::default()
        });
        assert_eq!(
            root.dir_redirect_location(
                "/assets",
                Some("client.example:8080"),
                "http",
                None,
                Some(8083),
            )
            .as_deref(),
            Some("http://nameinred.example:8083/assets/")
        );
    }

    #[test]
    fn dir_redirect_listener_port_used_and_default_omitted() {
        let (_t, root) = dir_root_policy(DirRedirectPolicy::default());
        // Non-default listener port is emitted (listener, not client port).
        assert_eq!(
            root.dir_redirect_location(
                "/assets",
                Some("h.example:5555"),
                "http",
                None,
                Some(8080),
            )
            .as_deref(),
            Some("http://h.example:8080/assets/")
        );
        // Scheme-default port is omitted.
        assert_eq!(
            root.dir_redirect_location("/assets", Some("h.example"), "http", None, Some(80))
                .as_deref(),
            Some("http://h.example/assets/")
        );
        assert_eq!(
            root.dir_redirect_location("/assets", Some("h.example"), "https", None, Some(443),)
                .as_deref(),
            Some("https://h.example/assets/")
        );
    }

    #[test]
    fn preload_cache_is_shared_across_route_prefixes() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.html"), b"shared").expect("write");
        let mut a = StaticRoot::new(dir.path(), "/a", Some("index.html")).expect("root a");
        let mut b = StaticRoot::new(dir.path(), "/b", Some("index.html")).expect("root b");
        a.preload_tree().expect("preload");
        assert_eq!(a.cache_len(), 1);
        b.share_preload_cache_from(&a);
        assert_eq!(b.cache_len(), 1);
        assert!(b.serve_request("/b/index.html").is_ok());
    }
}
