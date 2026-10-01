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
//! Epoll sendfile session table (module-owned SendingState).
//!
//! # Ownership (Cap067 / V044 P1)
//!
//! Session maps are **thread-local**. Each epoll keepalive worker owns its fds and is the
//! only thread that may call [`begin_sendfile_session`], [`pump_sendfile_session`], and
//! [`clear_sendfile_session`] for those fds. A process-global `Mutex` is not required:
//! sessions never migrate across workers; Hyper handoff clears the session on the owning
//! thread before the fd leaves epoll.
//!
//! Cross-thread SessionTable access is a contract violation (would panic under `RefCell`
//! or observe an empty map).
//!
//! # StaticRuntime pin (V044 P1 CPU Root1)
//!
//! [`StaticRuntime`] is process-lifetime: reload mutates roots **in place** via
//! [`StaticRuntime::bind_roots`] / `bind_compiled_slots` on the same `Arc`. Workers keep a
//! thread-local `Arc` cache of that published pin so Cap067 hot paths borrow `&StaticRuntime`
//! without per-request `Mutex` + `Arc::clone`.
//!
//! [`pin_runtime`] publishes a new `Arc` (install / test re-pin) and bumps [`PIN_EPOCH`].
//! Each worker refreshes its TLS cache when the epoch advances — never serves a retired Arc
//! after a publish. Generation/root freshness remains inside `StaticRuntime` (`RwLock`).

use crate::byte_range::{decide_range, range_header_from_raw_head, RangeDecision};
use crate::sendfile_fsm::{PumpResult, SendingState};
use crate::wire;
use crate::{StaticRoot, StaticRuntime};
use exyonq_module_api::static_dispatch::SendfileHandle;
use exyonq_module_api::static_epoll::StaticEpollPumpResult;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

struct SessionTable {
    next_id: u64,
    by_id: HashMap<u64, SendingState>,
    fd_session: HashMap<RawFd, u64>,
    /// Cap067: request head retained until terminal access emit.
    access_head: HashMap<u64, Vec<u8>>,
    access_started: HashMap<u64, std::time::Instant>,
}

impl SessionTable {
    fn new() -> Self {
        Self {
            next_id: 1,
            by_id: HashMap::new(),
            fd_session: HashMap::new(),
            access_head: HashMap::new(),
            access_started: HashMap::new(),
        }
    }
}

thread_local! {
    static SESSIONS: RefCell<SessionTable> = RefCell::new(SessionTable::new());
}

/// Published pin (rare writes: install / test re-pin). Hot path uses TLS cache.
static RUNTIME_PUBLISH: OnceLock<Mutex<Option<Arc<StaticRuntime>>>> = OnceLock::new();
/// Bumped on every [`pin_runtime`] so worker TLS caches refresh after Arc publish.
static PIN_EPOCH: AtomicU64 = AtomicU64::new(0);

struct LocalRuntimePin {
    epoch: u64,
    runtime: Option<Arc<StaticRuntime>>,
}

impl LocalRuntimePin {
    const fn empty() -> Self {
        Self {
            epoch: u64::MAX,
            runtime: None,
        }
    }
}

thread_local! {
    static LOCAL_RUNTIME: RefCell<LocalRuntimePin> = const { RefCell::new(LocalRuntimePin::empty()) };
}

fn with_sessions_mut<R>(f: impl FnOnce(&mut SessionTable) -> R) -> R {
    SESSIONS.with(|cell| f(&mut cell.borrow_mut()))
}

fn publish_slot() -> &'static Mutex<Option<Arc<StaticRuntime>>> {
    RUNTIME_PUBLISH.get_or_init(|| Mutex::new(None))
}

/// Pin StaticRuntime for epoll FSM (called from kernel_hooks / CLI).
///
/// Reload does **not** require a new pin when roots are rebound on the same `Arc`.
/// Re-pin (new `Arc`) invalidates worker TLS caches via [`PIN_EPOCH`].
/// True when a published static slot owns this request. Unpinned runtimes
/// report true so planner unit tests keep the Static decision.
pub fn static_slot_owns_request(head: &[u8]) -> bool {
    let published = publish_slot()
        .lock()
        .expect("epoll runtime pin poisoned")
        .clone();
    let Some(rt) = published else {
        return true;
    };
    let Some(path) = crate::wire_eligibility::request_path_from_head(head) else {
        return false;
    };
    let host = request_host_from_head(head);
    rt.slot_for_request_path(path, host.as_deref()).is_some()
}

pub fn pin_runtime(rt: Arc<StaticRuntime>) {
    let mut guard = publish_slot().lock().expect("epoll runtime pin poisoned");
    *guard = Some(rt);
    PIN_EPOCH.fetch_add(1, Ordering::Release);
    // New published Arc — drop worker-local Cap004 FDs bound to prior pin generation.
    crate::sendfile_fd_cache::clear_local();
}

/// Ensure this thread's TLS pin matches the published epoch, then borrow `&StaticRuntime`.
///
/// Cap067 hot path: no `Mutex` and no `Arc::clone` on the steady-state hit.
fn with_runtime<R>(f: impl FnOnce(&StaticRuntime) -> R) -> R {
    LOCAL_RUNTIME.with(|cell| {
        let mut local = cell.borrow_mut();
        let epoch = PIN_EPOCH.load(Ordering::Acquire);
        if local.epoch != epoch || local.runtime.is_none() {
            let published = publish_slot()
                .lock()
                .expect("epoll runtime pin poisoned")
                .clone()
                .expect("StaticRuntime not pinned for epoll FSM");
            local.runtime = Some(published);
            local.epoch = epoch;
        }
        f(local.runtime.as_ref().expect("pin filled").as_ref())
    })
}

/// Composition-root accessor for wire hooks that need an owned `Arc` (spawn / async move).
///
/// Not the Cap067 per-request hot path: clones the TLS-cached `Arc` once per hook call.
pub(crate) fn runtime_for_hooks() -> Arc<StaticRuntime> {
    LOCAL_RUNTIME.with(|cell| {
        let mut local = cell.borrow_mut();
        let epoch = PIN_EPOCH.load(Ordering::Acquire);
        if local.epoch != epoch || local.runtime.is_none() {
            let published = publish_slot()
                .lock()
                .expect("epoll runtime pin poisoned")
                .clone()
                .expect("StaticRuntime not pinned for epoll FSM");
            local.runtime = Some(published);
            local.epoch = epoch;
        }
        Arc::clone(local.runtime.as_ref().expect("pin filled"))
    })
}

fn request_host_from_head(head: &[u8]) -> Option<String> {
    for line in head.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.len() >= 5 && line[..5].eq_ignore_ascii_case(b"host:") {
            let mut value = &line[5..];
            while value.first().copied() == Some(b' ') || value.first().copied() == Some(b'\t') {
                value = &value[1..];
            }
            let host = std::str::from_utf8(value).ok()?;
            return Some(strip_host_port(host).to_string());
        }
    }
    None
}

fn strip_host_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return &host[..=end];
        }
        return host;
    }
    match host.rsplit_once(':') {
        Some((name, port)) if !name.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    }
}

pub fn match_sendfile_asset(site_slot: u32, head: &[u8]) -> Option<SendfileHandle> {
    // The accept pin is not the route. `slot_for_request_path` chooses the root.
    let _ = site_slot;
    // Cap067: capability-based resolve — never filename/bench allowlists.
    if !crate::sendfile_fsm::epoll_sendfile_enabled() {
        return None;
    }
    if !crate::wire_eligibility::epoll_sendfile_eligible(head) {
        return None;
    }
    let path = crate::wire_eligibility::request_path_from_head(head)?;
    let enc_cfg = crate::encoding_cache::effective_config();
    let ae = crate::conditional::header_from_raw_head(head, b"accept-encoding");
    let wanted_coding = crate::encoding_cache::select_coding_from_accept_encoding(ae, &enc_cfg);

    with_runtime(|rt| {
        let generation = rt.generation();
        let site_slot = rt.slot_for_request_path(path, request_host_from_head(head).as_deref())?;
        let root = rt.root_for_slot_public(site_slot).ok()?;

        // A large-file sendfile hit must not skip the 406 the miss path emits.
        if wanted_coding.is_none()
            && crate::encoding_cache::identity_q_zero_without_other_coding(ae)
        {
            return None;
        }

        // ADR-046: when AE wants a static-cache coding and the feature is ON,
        // skip identity FD-cache hits — never sendfile identity without CE.
        if wanted_coding.is_none() {
            if let Some(asset) =
                crate::sendfile_fd_cache::try_hit_hot(generation, site_slot, path, root.as_ref())
            {
                // LA-CAP067-DEAD-001: enforce SENDFILE_MIN_BYTES (comment was lying).
                if asset.body_len < crate::sendfile::SENDFILE_MIN_BYTES {
                    return None;
                }
                return Some(rt.issue_sendfile_handle(asset));
            }
        }

        // Cap067-only amortization: generation-scoped preload PathBuf (no canonicalize).
        // Hyper / File::open surfaces keep resolve_live_file_path → uncached Cap004.
        let file_path = root.resolved_file_path(path).ok()?;
        let prepared = root.prepared_wire(path);
        let open_root = crate::sendfile::OpenUnderRoot {
            canonical_root: root.canonical_root(),
            root_dir_fd: root.root_dir_fd(),
        };

        // ADR-046 A1: sync compress-once → sendfile coded FD (no hot-path zlib).
        // Warm FD-hit: reuse coded File Arc after source fstatat identity match.
        // Disk hit: fstatat + open coded only. Miss: Cap004 + read + compress-once.
        if let Some(coding) = wanted_coding {
            #[cfg(target_os = "linux")]
            {
                let coding_tag = coding.fd_cache_tag();
                if let Some(asset) = crate::sendfile_fd_cache::try_hit_coded_hot(
                    generation,
                    site_slot,
                    path,
                    coding_tag,
                    root.as_ref(),
                ) {
                    if asset.body_len < crate::sendfile::SENDFILE_MIN_BYTES {
                        return None;
                    }
                    return Some(rt.issue_sendfile_handle(asset));
                }
                let identity = crate::sendfile::identity_under_root(&file_path, open_root).ok()?;
                if identity.len < enc_cfg.min_bytes || identity.len > enc_cfg.max_bytes {
                    return None;
                }
                let content_type = crate::content_type_for(&file_path);
                let source_validators = match prepared {
                    Some(prep) if prep.identity == identity => prep.validators.as_ref().clone(),
                    _ => crate::conditional::StaticValidators::from_validator_identity(&identity),
                };
                if let Some(coded) = crate::encoding_cache::try_open_cached_coded(
                    &enc_cfg,
                    coding,
                    &identity,
                    &source_validators,
                    content_type,
                ) {
                    let asset = std::sync::Arc::new(coded);
                    crate::sendfile_fd_cache::insert_coded(
                        generation,
                        site_slot,
                        path,
                        coding_tag,
                        &file_path,
                        root.canonical_root(),
                        std::sync::Arc::clone(&asset),
                        identity,
                    );
                    return Some(rt.issue_sendfile_handle(asset));
                }
                let asset = crate::sendfile::SendfileAsset::open_under_root_prepared(
                    &file_path,
                    Some(open_root),
                    0,
                    prepared,
                )
                .ok()?;
                let fd_identity = match asset.file.metadata() {
                    Ok(meta) => crate::conditional::ValidatorIdentity::from_metadata(&meta),
                    Err(_) => return None,
                };
                return crate::encoding_cache::try_coded_sendfile_asset(
                    &enc_cfg,
                    coding,
                    &file_path,
                    &fd_identity,
                    asset.validators.as_ref(),
                    asset.content_type,
                )
                .map(|coded| {
                    let asset = std::sync::Arc::new(coded);
                    crate::sendfile_fd_cache::insert_coded(
                        generation,
                        site_slot,
                        path,
                        coding_tag,
                        &file_path,
                        root.canonical_root(),
                        std::sync::Arc::clone(&asset),
                        fd_identity,
                    );
                    rt.issue_sendfile_handle(asset)
                });
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = coding;
                return None;
            }
        }

        let asset = crate::sendfile::SendfileAsset::open_under_root_prepared(
            &file_path,
            Some(open_root),
            0,
            prepared,
        )
        .ok()?;

        if asset.body_len < crate::sendfile::SENDFILE_MIN_BYTES {
            return None;
        }
        // LA-CAP067-FD-001: stamp identity from Cap004 FD, and refuse to cache when
        // pathname fstatat already diverges (open → rename race) so we never pair
        // inode-A's File with pathname identity B.
        let fd_identity = match asset.file.metadata() {
            Ok(meta) => crate::conditional::ValidatorIdentity::from_metadata(&meta),
            Err(_) => {
                return Some(rt.issue_sendfile_handle(std::sync::Arc::new(asset)));
            }
        };
        let path_identity = crate::sendfile::identity_under_root(&file_path, open_root).ok();
        let asset = std::sync::Arc::new(asset);
        let cacheable = match path_identity {
            Some(pid) if pid.dev == fd_identity.dev && pid.ino == fd_identity.ino => Some(pid),
            _ => None,
        };
        if let Some(identity) = cacheable {
            crate::sendfile_fd_cache::insert(
                generation,
                site_slot,
                path,
                &file_path,
                root.canonical_root(),
                std::sync::Arc::clone(&asset),
                identity,
            );
        }
        Some(rt.issue_sendfile_handle(asset))
    })
}

/// Cap067 CAP067_MISS_NEEDHYPER_HANG: classify `match_sendfile_asset(None)` into a
/// terminal HTTP wire response owned by the epoll path.
///
/// Returns `Some(wire)` for known static negative outcomes (Cap004 / missing / open
/// failure). Returns `None` only when Cap067 is not eligible to own the miss (mechanism
/// off / head not sendfile-eligible) — caller may Hyper-fallback without re-registering
/// sendfile.
///
/// Security: PathTraversal → 403; containment/open failures → 404. Never discloses
/// out-of-root secret bytes. Does not reopen a less-restrictive Hyper static path.
pub fn sendfile_miss_http_wire(site_slot: u32, head: &[u8]) -> Option<bytes::Bytes> {
    let _ = site_slot;
    if !crate::sendfile_fsm::epoll_sendfile_enabled() {
        return None;
    }
    if !crate::wire_eligibility::epoll_sendfile_eligible(head) {
        return None;
    }
    let client_close = head_requests_connection_close(head);
    let head_only = StaticRoot::is_head_wire_request(head);
    let not_found = |close: bool| {
        if head_only {
            crate::wire::not_found_status_head_wire(close)
        } else {
            crate::wire::not_found_status_wire(close)
        }
    };
    let forbidden = |close: bool| {
        if head_only {
            crate::wire::forbidden_status_head_wire(close)
        } else {
            crate::wire::forbidden_status_wire(close)
        }
    };
    let Some(path) = crate::wire_eligibility::request_path_from_head(head) else {
        // Eligible GET/HEAD with unparseable path — reject, do not hang.
        return Some(not_found(client_close));
    };
    with_runtime(|rt| {
        let Some(site_slot) = rt.slot_for_request_path(path, request_host_from_head(head).as_deref()) else {
            // No static root owns this path. Hyper must route it (proxy, metrics
            // health). A terminal 404 would hide those routes. The planner must
            // not choose Static again for this head.
            return None;
        };
        let Ok(root) = rt.root_for_slot_public(site_slot) else {
            return Some(not_found(client_close));
        };
        match root.resolved_file_path(path) {
            Err(crate::StaticError::PathTraversal) => Some(forbidden(client_close)),
            Err(crate::StaticError::NotFound) | Err(crate::StaticError::BudgetExceeded) => {
                Some(not_found(client_close))
            }
            Err(crate::StaticError::Io(_)) => {
                // Do not leak IO detail on the static surface.
                Some(not_found(client_close))
            }
            Ok(file_path) => {
                let ae = crate::conditional::header_from_raw_head(head, b"accept-encoding");
                let enc_cfg = crate::encoding_cache::effective_config();
                let wanted =
                    crate::encoding_cache::select_coding_from_accept_encoding(ae, &enc_cfg);
                if wanted.is_none()
                    && crate::encoding_cache::identity_q_zero_without_other_coding(ae)
                {
                    return Some(if head_only {
                        crate::wire::not_acceptable_status_head_wire(client_close)
                    } else {
                        crate::wire::not_acceptable_status_wire(client_close)
                    });
                }
                let open_root = crate::sendfile::OpenUnderRoot {
                    canonical_root: root.canonical_root(),
                    root_dir_fd: root.root_dir_fd(),
                };
                // OLS keeps files ≤ maxCachedFileSize (4096) in RAM and writes them.
                // P1 is 1 KiB, below sendfile, so this path used to open+read every request.
                if !client_close {
                    if let Some(wire) = cached_small_wire(
                        rt.generation(),
                        site_slot,
                        path,
                        &file_path,
                        open_root,
                        head_only,
                    ) {
                        return Some(wire);
                    }
                }
                match crate::sendfile::SendfileAsset::open_under_root_prepared(
                    &file_path,
                    Some(open_root),
                    0,
                    None,
                ) {
                    Ok(asset) => {
                        // LA-CAP067-DEAD-001: small files must not NeedHyper (Cap067 re-hang).
                        // Serve inline HTTP/1 wire; large TOCTOU still declines to Hyper.
                        if asset.body_len < crate::sendfile::SENDFILE_MIN_BYTES {
                            if !client_close && asset.body_len <= SMALL_FILE_MEM_CACHE {
                                match small_file_inline_wire(&asset, false) {
                                    Ok(full) => {
                                        let header_len = asset.header.len();
                                        remember_small_wire(
                                            rt.generation(),
                                            site_slot,
                                            path,
                                            &asset,
                                            header_len,
                                            &full,
                                        );
                                        let wire = if head_only {
                                            full.slice(..header_len.min(full.len()))
                                        } else {
                                            full
                                        };
                                        Some(wire)
                                    }
                                    Err(_) => Some(not_found(client_close)),
                                }
                            } else {
                                match small_file_inline_wire(&asset, head_only) {
                                    Ok(wire) => Some(wire),
                                    Err(_) => Some(not_found(client_close)),
                                }
                            }
                        } else {
                            None
                        }
                    }
                    Err(_) => Some(not_found(client_close)),
                }
            }
        }
    })
}

/// Cap067 small-file path: write prepared 200 header + body (or HEAD header only).
fn small_file_inline_wire(
    asset: &crate::sendfile::SendfileAsset,
    head_only: bool,
) -> std::io::Result<bytes::Bytes> {
    use std::io::{Read, Seek, SeekFrom};
    if head_only {
        return Ok(asset.header.as_ref().clone());
    }
    let mut file = asset.file.try_clone()?;
    file.seek(SeekFrom::Start(0))?;
    let mut body = vec![0u8; asset.body_len];
    file.read_exact(&mut body)?;
    let mut out = Vec::with_capacity(asset.header.len() + body.len());
    out.extend_from_slice(asset.header.as_ref());
    out.extend_from_slice(&body);
    Ok(bytes::Bytes::from(out))
}

/// OLS `maxCachedFileSize`: files this small are served from one cached wire buffer.
const SMALL_FILE_MEM_CACHE: usize = 4096;
const SMALL_WIRE_REVALIDATE: Duration = Duration::from_millis(1);
const SMALL_WIRE_MAX_ENTRIES: usize = 64;

struct SmallWireEntry {
    generation: u64,
    site_slot: u32,
    path: String,
    identity: crate::conditional::ValidatorIdentity,
    revalidated_at: Instant,
    header_len: usize,
    wire: bytes::Bytes,
}

thread_local! {
    static SMALL_WIRES: RefCell<Vec<SmallWireEntry>> = const { RefCell::new(Vec::new()) };
}

fn cached_small_wire(
    generation: u64,
    site_slot: u32,
    path: &str,
    file_path: &std::path::Path,
    open_root: crate::sendfile::OpenUnderRoot<'_>,
    head_only: bool,
) -> Option<bytes::Bytes> {
    SMALL_WIRES.with(|cell| {
        let mut cache = cell.borrow_mut();
        let idx = cache.iter().position(|entry| {
            entry.generation == generation && entry.site_slot == site_slot && entry.path == path
        })?;
        if cache[idx].revalidated_at.elapsed() >= SMALL_WIRE_REVALIDATE {
            let live = crate::sendfile::identity_under_root(file_path, open_root).ok();
            match live {
                Some(id) if id == cache[idx].identity => {
                    cache[idx].revalidated_at = Instant::now();
                }
                _ => {
                    cache.swap_remove(idx);
                    return None;
                }
            }
        }
        let entry = &cache[idx];
        let wire = if head_only {
            entry.wire.slice(..entry.header_len.min(entry.wire.len()))
        } else {
            entry.wire.clone()
        };
        Some(wire)
    })
}

fn remember_small_wire(
    generation: u64,
    site_slot: u32,
    path: &str,
    asset: &crate::sendfile::SendfileAsset,
    header_len: usize,
    wire: &bytes::Bytes,
) {
    let Ok(meta) = asset.file.metadata() else {
        return;
    };
    let identity = crate::conditional::ValidatorIdentity::from_metadata(&meta);
    let entry = SmallWireEntry {
        generation,
        site_slot,
        path: path.to_string(),
        identity,
        revalidated_at: Instant::now(),
        header_len,
        wire: wire.clone(),
    };
    SMALL_WIRES.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some(idx) = cache.iter().position(|old| {
            old.generation == generation && old.site_slot == site_slot && old.path == path
        }) {
            cache[idx] = entry;
            return;
        }
        if cache.len() >= SMALL_WIRE_MAX_ENTRIES {
            cache.swap_remove(0);
        }
        cache.push(entry);
    });
}

fn head_requests_connection_close(head: &[u8]) -> bool {
    let lower = |b: u8| b.to_ascii_lowercase();
    let mut i = 0usize;
    while i + 12 < head.len() {
        if lower(head[i]) == b'c'
            && lower(head[i + 1]) == b'o'
            && lower(head[i + 2]) == b'n'
            && lower(head[i + 3]) == b'n'
            && lower(head[i + 4]) == b'e'
            && lower(head[i + 5]) == b'c'
            && lower(head[i + 6]) == b't'
            && lower(head[i + 7]) == b'i'
            && lower(head[i + 8]) == b'o'
            && lower(head[i + 9]) == b'n'
            && head[i + 10] == b':'
        {
            let rest = &head[i + 11..];
            let end = rest
                .iter()
                .position(|&b| b == b'\r' || b == b'\n')
                .unwrap_or(rest.len());
            let val = &rest[..end];
            if val.windows(5).any(|w| {
                w.iter()
                    .map(|b| b.to_ascii_lowercase())
                    .eq(b"close".iter().copied())
            }) {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Cap067: drop an issued handle that will not enter a session (abort / Hyper fallback).
pub fn release_sendfile_handle(asset: SendfileHandle) {
    with_runtime(|rt| rt.release_sendfile_handle(asset));
}

pub fn begin_sendfile_session(fd: RawFd, head: &[u8], asset: SendfileHandle) -> Result<u64, ()> {
    let asset_arc = with_runtime(|rt| rt.take_sendfile_handle(asset)).ok_or(())?;
    let head_only = StaticRoot::is_head_wire_request(head);
    let full_len = asset_arc.body_len as u64;
    // Validators + Cap020 200/304 headers prepared at open (or reused from generation).
    let validators = asset_arc.validators.as_ref();
    let inm = crate::conditional::header_from_raw_head(head, b"if-none-match");
    let ims = crate::conditional::header_from_raw_head(head, b"if-modified-since");
    if crate::conditional::decide_conditional(inm, ims, validators)
        == crate::conditional::ConditionalDecision::NotModified
    {
        let header_304 = std::sync::Arc::clone(&asset_arc.header_304);
        let state =
            SendingState::new_head_range(asset_arc, header_304.as_ref(), 304).map_err(|_| ())?;
        let id = insert_session(fd, head, state)?;
        with_runtime(|rt| rt.note_sendfile_engagement());
        return Ok(id);
    }
    let range_raw = range_header_from_raw_head(head);
    let decision = decide_range(range_raw, full_len);

    let state = match decision {
        RangeDecision::Ignore => {
            // S4: reuse prepared Cap020 200 header (no per-request String format).
            let header_200 = std::sync::Arc::clone(&asset_arc.header);
            if head_only {
                SendingState::new_head_range(asset_arc, header_200.as_ref(), 200).map_err(|_| ())?
            } else {
                SendingState::new_get_full(asset_arc, header_200.as_ref()).map_err(|_| ())?
            }
        }
        RangeDecision::Unsatisfiable { full_length } => {
            let header = wire::range_not_satisfiable_header(full_length);
            SendingState::new_head_range(asset_arc, header.as_ref(), 416).map_err(|_| ())?
        }
        RangeDecision::Satisfied(sel) => {
            // Range Content-Range varies per request — still format here.
            let header = wire::partial_content_header_with_validators(
                sel.start,
                sel.end,
                sel.full_length,
                asset_arc.content_type,
                Some(&validators.etag),
                validators.last_modified.as_deref(),
            );
            let length = usize::try_from(sel.content_length()).map_err(|_| ())?;
            if head_only {
                SendingState::new_head_range(asset_arc, header.as_ref(), 206).map_err(|_| ())?
            } else {
                SendingState::new_get_range(asset_arc, header.as_ref(), sel.start, length)
                    .map_err(|_| ())?
            }
        }
    };

    let id = insert_session(fd, head, state)?;
    // LA-CAP067-ENG-001: Cap067 engagements were invisible (Hyper divert always None).
    with_runtime(|rt| rt.note_sendfile_engagement());
    Ok(id)
}

fn insert_session(fd: RawFd, head: &[u8], state: SendingState) -> Result<u64, ()> {
    with_sessions_mut(|table| {
        let id = table.next_id;
        table.next_id = table.next_id.wrapping_add(1).max(1);
        // Access-off (P1 bench and default static) must not allocate the request head.
        if exyonq_module_api::static_wire::access_notices_enabled() {
            table.access_head.insert(id, head.to_vec());
            table.access_started.insert(id, std::time::Instant::now());
        }
        table.by_id.insert(id, state);
        table.fd_session.insert(fd, id);
        Ok(id)
    })
}

fn access_record(table: &SessionTable, session: u64) -> (Vec<u8>, std::time::Instant) {
    if !exyonq_module_api::static_wire::access_notices_enabled() {
        return (Vec::new(), std::time::Instant::now());
    }
    let head = table
        .access_head
        .get(&session)
        .cloned()
        .unwrap_or_default();
    let started = table
        .access_started
        .get(&session)
        .copied()
        .unwrap_or_else(std::time::Instant::now);
    (head, started)
}

pub fn pump_sendfile_session(fd: RawFd, session: u64) -> StaticEpollPumpResult {
    // Collect terminal access fields under the TLS borrow, then notify after release so
    // access hooks cannot re-enter `SESSIONS` under `RefCell`.
    enum Terminal {
        Complete {
            status: u16,
            outcome: &'static str,
            head: Vec<u8>,
            started: std::time::Instant,
        },
        Aborted {
            status: u16,
            head: Vec<u8>,
            started: std::time::Instant,
        },
        None,
    }

    let (result, terminal) = with_sessions_mut(|table| {
        let drain = {
            let Some(state) = table.by_id.get_mut(&session) else {
                return (StaticEpollPumpResult::Error, Terminal::None);
            };
            crate::sendfile_fsm::epoll_drain_pump_once(fd, state)
        };
        match drain.result {
            PumpResult::Progress => (StaticEpollPumpResult::Progress, Terminal::None),
            PumpResult::Parked => (StaticEpollPumpResult::Parked, Terminal::None),
            PumpResult::Complete => {
                let status = table
                    .by_id
                    .get(&session)
                    .map(|s| s.access_status)
                    .unwrap_or(500);
                let outcome = match status {
                    200 => "static_sendfile_ok",
                    206 => "static_sendfile_partial",
                    304 => "static_sendfile_not_modified",
                    416 => "static_sendfile_range_unsat",
                    _ => "static_sendfile_terminal",
                };
                let (head, started) = access_record(table, session);
                (
                    StaticEpollPumpResult::Complete,
                    Terminal::Complete {
                        status,
                        outcome,
                        head,
                        started,
                    },
                )
            }
            PumpResult::Error(_) => {
                // Cap067 / Cap061: if any response bytes may have been committed, emit one
                // terminal access event (never invent Complete/200).
                let aborted = if table
                    .by_id
                    .get(&session)
                    .is_some_and(|state| state.header_done || state.header.offset > 0)
                {
                    let status = table
                        .by_id
                        .get(&session)
                        .map(|state| state.access_status)
                        .unwrap_or(500);
                    let (head, started) = access_record(table, session);
                    Some(Terminal::Aborted {
                        status,
                        head,
                        started,
                    })
                } else {
                    None
                };
                (
                    StaticEpollPumpResult::Error,
                    aborted.unwrap_or(Terminal::None),
                )
            }
        }
    });

    match terminal {
        Terminal::Complete {
            status,
            outcome,
            head,
            started,
        } => {
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            let method = if head.starts_with(b"HEAD ") { "HEAD" } else { "GET" };
            let path = crate::wire_eligibility::request_path_from_head(&head).unwrap_or("/");
            exyonq_module_api::wire_record_exchange(method, path, status, elapsed_ms);
            crate::wire_conn::notify_wire_access_for_hooks(&head, status, outcome, started);
        }
        Terminal::Aborted {
            status,
            head,
            started,
        } => {
            crate::wire_conn::notify_wire_access_for_hooks(
                &head,
                status,
                "static_sendfile_aborted",
                started,
            );
        }
        Terminal::None => {}
    }

    result
}

pub fn clear_sendfile_session(fd: RawFd, session: u64) {
    with_sessions_mut(|table| {
        table.by_id.remove(&session);
        if table.fd_session.get(&fd).copied() == Some(session) {
            table.fd_session.remove(&fd);
        }
        table.access_head.remove(&session);
        table.access_started.remove(&session);
    });
}

/// Clears sessions on the **calling** thread only (TLS). Tests that drive Cap067 from a
/// dedicated worker must clear on that same thread.
#[cfg(any(test, feature = "test-utils"))]
#[allow(dead_code)]
pub fn clear_all_sessions_for_test() {
    with_sessions_mut(|table| {
        table.by_id.clear();
        table.fd_session.clear();
        table.access_head.clear();
        table.access_started.clear();
        table.next_id = 1;
    });
}

#[cfg(test)]
pub(crate) fn serial_runtime_pin_for_tests() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    /// Process-global pin is shared — serialize pin/re-pin tests under parallel runners.
    fn serial_runtime_pin() -> std::sync::MutexGuard<'static, ()> {
        super::serial_runtime_pin_for_tests()
    }

    #[test]
    fn session_table_is_thread_local() {
        clear_all_sessions_for_test();
        with_sessions_mut(|t| {
            t.fd_session.insert(7, 42);
        });
        let seen = std::thread::spawn(|| with_sessions_mut(|t| t.fd_session.contains_key(&7)))
            .join()
            .expect("join");
        assert!(
            !seen,
            "child thread must not observe parent TLS SessionTable entries"
        );
        with_sessions_mut(|t| {
            assert!(t.fd_session.contains_key(&7));
            t.fd_session.clear();
        });
    }

    #[test]
    fn runtime_pin_is_thread_local_cache_with_shared_publish() {
        let _guard = serial_runtime_pin();
        let rt_a = Arc::new(StaticRuntime::new());
        pin_runtime(Arc::clone(&rt_a));
        with_runtime(|rt| {
            assert!(
                std::ptr::eq(
                    rt as *const StaticRuntime,
                    rt_a.as_ref() as *const StaticRuntime
                ),
                "parent must observe published runtime"
            );
        });

        let parent_epoch = PIN_EPOCH.load(Ordering::Acquire);
        let child_saw_a = std::thread::spawn({
            let rt_a = Arc::clone(&rt_a);
            move || {
                with_runtime(|rt| {
                    std::ptr::eq(
                        rt as *const StaticRuntime,
                        rt_a.as_ref() as *const StaticRuntime,
                    )
                })
            }
        })
        .join()
        .expect("join");
        assert!(
            child_saw_a,
            "child cold path must load same published Arc without inheriting parent TLS cell"
        );
        assert_eq!(
            PIN_EPOCH.load(Ordering::Acquire),
            parent_epoch,
            "read path must not bump pin epoch"
        );
    }

    #[test]
    fn re_pin_refreshes_worker_local_cache() {
        let _guard = serial_runtime_pin();
        let rt_a = Arc::new(StaticRuntime::new());
        let rt_b = Arc::new(StaticRuntime::new());
        pin_runtime(Arc::clone(&rt_a));
        with_runtime(|rt| {
            assert!(std::ptr::eq(
                rt as *const StaticRuntime,
                rt_a.as_ref() as *const StaticRuntime
            ));
        });
        pin_runtime(Arc::clone(&rt_b));
        with_runtime(|rt| {
            assert!(
                std::ptr::eq(
                    rt as *const StaticRuntime,
                    rt_b.as_ref() as *const StaticRuntime
                ),
                "after re-pin, TLS cache must serve the new Arc"
            );
            assert!(!std::ptr::eq(
                rt as *const StaticRuntime,
                rt_a.as_ref() as *const StaticRuntime
            ));
        });
    }

    #[test]
    fn bind_roots_on_pinned_arc_visible_without_re_pin() {
        let _guard = serial_runtime_pin();
        let rt = Arc::new(StaticRuntime::new());
        pin_runtime(Arc::clone(&rt));
        assert_eq!(with_runtime(|r| r.generation()), 0);

        let dir = tempfile::tempdir().expect("tempdir");
        let root = StaticRoot::new(dir.path(), "/site", None).expect("root");
        rt.bind_roots(7, Box::new([Arc::new(root)]));

        // Same Arc pin — generation publish must be visible without pin_runtime.
        assert_eq!(
            with_runtime(|r| r.generation()),
            7,
            "in-place bind_roots must be visible through worker-local pin of same Arc"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn engagement_counts_once_only_after_session_insert() {
        let _guard = serial_runtime_pin();
        clear_all_sessions_for_test();
        crate::sendfile_fsm::reset_epoll_sendfile_enabled_cache_for_tests();
        crate::sendfile_fd_cache::clear_local();

        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        std::fs::create_dir_all(&root_path).expect("mkdir");
        std::fs::write(
            root_path.join("large.bin"),
            vec![0x5a; crate::sendfile::SENDFILE_MIN_BYTES],
        )
        .expect("write");

        let root = StaticRoot::new_with_preload_limits(
            &root_path,
            "/",
            None,
            crate::PreloadLimits::DEFAULT,
        )
        .expect("root");
        let rt = Arc::new(StaticRuntime::new());
        rt.bind_roots(1, Box::new([Arc::new(root)]));
        pin_runtime(Arc::clone(&rt));

        let head = b"GET /large.bin HTTP/1.1\r\nHost: t\r\n\r\n";
        assert!(
            match_sendfile_asset(0, b"GET /missing.bin HTTP/1.1\r\nHost: t\r\n\r\n").is_none(),
            "fallback must not create a session"
        );
        let released = match_sendfile_asset(0, head).expect("match for release");
        assert_eq!(
            exyonq_module_api::static_dispatch::StaticDispatchService::metrics(rt.as_ref())
                .sendfile_engagements,
            0,
            "matching alone must not count an engagement"
        );
        release_sendfile_handle(released);
        assert!(
            begin_sendfile_session(7, head, released).is_err(),
            "released handle must fail before session insertion"
        );
        assert_eq!(
            exyonq_module_api::static_dispatch::StaticDispatchService::metrics(rt.as_ref())
                .sendfile_engagements,
            0,
            "failed begin must not count an engagement"
        );

        let handle = match_sendfile_asset(0, head).expect("match for session");
        let session = begin_sendfile_session(7, head, handle).expect("session insert");
        assert_eq!(
            exyonq_module_api::static_dispatch::StaticDispatchService::metrics(rt.as_ref())
                .sendfile_engagements,
            1,
            "successful insert must count exactly once"
        );
        clear_sendfile_session(7, session);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sendfile_miss_wire_classifies_not_found_and_traversal() {
        let _guard = serial_runtime_pin();
        crate::sendfile_fsm::reset_epoll_sendfile_enabled_cache_for_tests();
        crate::sendfile_fd_cache::clear_local();

        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        std::fs::create_dir_all(&root_path).expect("mkdir");
        std::fs::write(root_path.join("ok.bin"), b"ok").expect("write");

        let root = StaticRoot::new_with_preload_limits(
            &root_path,
            "/",
            None,
            crate::PreloadLimits::DEFAULT,
        )
        .expect("root");
        let rt = Arc::new(StaticRuntime::new());
        rt.bind_roots(1, Box::new([Arc::new(root)]));
        pin_runtime(Arc::clone(&rt));

        let missing = b"GET /missing.bin HTTP/1.1\r\nHost: t\r\n\r\n";
        let miss_wire = sendfile_miss_http_wire(0, missing).expect("missing terminal");
        assert!(
            miss_wire.starts_with(b"HTTP/1.1 404 "),
            "missing path must be HTTP 404, got {:?}",
            std::str::from_utf8(&miss_wire[..32.min(miss_wire.len())])
        );

        let trav = b"GET /../outside HTTP/1.1\r\nHost: t\r\n\r\n";
        let trav_wire = sendfile_miss_http_wire(0, trav).expect("traversal terminal");
        assert!(
            trav_wire.starts_with(b"HTTP/1.1 403 "),
            "path traversal must be HTTP 403, got {:?}",
            std::str::from_utf8(&trav_wire[..32.min(trav_wire.len())])
        );

        assert!(
            match_sendfile_asset(0, missing).is_none(),
            "missing must not issue sendfile handle"
        );

        let head_miss = b"HEAD /missing.bin HTTP/1.1\r\nHost: t\r\n\r\n";
        let head_wire = sendfile_miss_http_wire(0, head_miss).expect("HEAD missing terminal");
        assert!(
            head_wire.starts_with(b"HTTP/1.1 404 "),
            "HEAD missing must be HTTP 404"
        );
        let after_hdr = head_wire
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| &head_wire[i + 4..])
            .unwrap_or(&head_wire[..]);
        assert!(
            after_hdr.is_empty(),
            "HEAD miss must not write a response body (keepalive framing)"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sendfile_miss_wire_rejects_out_of_root_symlink_without_disclosure() {
        let _guard = serial_runtime_pin();
        crate::sendfile_fsm::reset_epoll_sendfile_enabled_cache_for_tests();
        crate::sendfile_fd_cache::clear_local();

        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        let outside = dir.path().join("outside-secret.txt");
        std::fs::create_dir_all(&root_path).expect("mkdir");
        std::fs::write(&outside, b"TOP_SECRET_PAYLOAD").expect("secret");
        std::os::unix::fs::symlink(&outside, root_path.join("escape.link")).expect("symlink");

        let root = StaticRoot::new_with_preload_limits(
            &root_path,
            "/",
            None,
            crate::PreloadLimits::DEFAULT,
        )
        .expect("root");
        let rt = Arc::new(StaticRuntime::new());
        rt.bind_roots(1, Box::new([Arc::new(root)]));
        pin_runtime(Arc::clone(&rt));

        let head = b"GET /escape.link HTTP/1.1\r\nHost: t\r\n\r\n";
        let wire = sendfile_miss_http_wire(0, head).expect("symlink miss terminal");
        let text = std::str::from_utf8(&wire).unwrap_or("");
        assert!(
            wire.starts_with(b"HTTP/1.1 403 ") || wire.starts_with(b"HTTP/1.1 404 "),
            "symlink escape must be 403 or 404, got {text:?}"
        );
        assert!(
            !text.contains("TOP_SECRET_PAYLOAD"),
            "must not disclose out-of-root secret"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn small_file_wire_is_reused_and_sees_a_later_rewrite() {
        let _guard = serial_runtime_pin();
        crate::sendfile_fsm::reset_epoll_sendfile_enabled_cache_for_tests();
        SMALL_WIRES.with(|cell| cell.borrow_mut().clear());

        let dir = tempfile::tempdir().expect("tempdir");
        let root_path = dir.path().join("root");
        std::fs::create_dir_all(&root_path).expect("mkdir");
        let file = root_path.join("1k.bin");
        std::fs::write(&file, vec![b'A'; 1024]).expect("write");

        let root = StaticRoot::new_with_preload_limits(
            &root_path,
            "/",
            None,
            crate::PreloadLimits::DEFAULT,
        )
        .expect("root");
        let rt = Arc::new(StaticRuntime::new());
        rt.bind_roots(1, Box::new([Arc::new(root)]));
        pin_runtime(Arc::clone(&rt));

        let head = b"GET /1k.bin HTTP/1.1\r\nHost: t\r\n\r\n";
        let first = sendfile_miss_http_wire(0, head).expect("first");
        let second = sendfile_miss_http_wire(0, head).expect("second");
        assert_eq!(first, second);
        assert!(first.ends_with(&[b'A'; 16]));

        std::thread::sleep(SMALL_WIRE_REVALIDATE + Duration::from_millis(2));
        std::fs::write(&file, vec![b'B'; 1024]).expect("rewrite");
        let third = sendfile_miss_http_wire(0, head).expect("rewritten");
        assert!(
            third.ends_with(&[b'B'; 16]),
            "a replaced small file must show up after the memory-cache interval"
        );
        assert_ne!(first, third);
    }
}
