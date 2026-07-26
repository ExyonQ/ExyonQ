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
//! KD2 static dispatch contract (core ↔ module seam).
//!
//! KD2.0: core may depend only on types in this module — never on static wire/sendfile internals.
//! KD2.1: body/path shapes revised so KD2.2/KD2.3 need not materialize every response as `Vec<u8>`.

//! KD2.2 — sendfile handle for large static bodies (registry-owned until take/release).

use crate::fcgi_dispatch::MaterializedBackendOutcome;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Opaque module-owned sendfile asset id.
///
/// **Owner:** `exyonq-mod-static` [`SendfileHandleRegistry`] holds `Arc<SendfileAsset>` until
/// [`StaticRuntime::take_sendfile_handle`] (exactly once) or [`StaticRuntime::release_sendfile_handle`].
/// **Fd lifetime:** `Arc<File>` inside the asset; kernel closes when last `Arc` drops — no manual fd close in core.
/// **Reload:** handles tagged with runtime generation; stale ids are invalidated on `bind_roots`.
/// **KD2.3:** epoll FSM calls `take_sendfile_handle` once; cancel/error paths call `release_sendfile_handle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SendfileHandle(pub u64);

/// HTTP method for static dispatch (subset used by kernel shell).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticMethod {
    Get,
    Head,
}

/// Opaque root slot compiled into the runtime plan (maps to configured static root).
pub type StaticRootSlot = u32;

/// KD2.5 — compiled static metadata in snapshot (no operational `StaticRoot` in core).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCompiledSlot {
    pub filesystem_root: PathBuf,
    pub route_prefix: String,
    pub index_file: Option<String>,
    /// IR route name (e.g. `"site"` for wire/epoll fast path).
    pub route_name: String,
    /// Tree-preload per-file logical cap (bytes). `0` disables preload.
    pub preload_max_file_bytes: u64,
    /// Tree-preload total logical budget (bytes). `0` disables preload.
    pub preload_max_total_bytes: u64,
    /// Tree-preload max accepted entries. `0` disables preload.
    pub preload_max_entries: u64,
}

impl StaticCompiledSlot {
    /// Policy defaults (32 MiB / 256 MiB / 4096).
    pub const DEFAULT_PRELOAD_MAX_FILE_BYTES: u64 = 33_554_432;
    pub const DEFAULT_PRELOAD_MAX_TOTAL_BYTES: u64 = 268_435_456;
    pub const DEFAULT_PRELOAD_MAX_ENTRIES: u64 = 4096;
}

/// KD2.5 — minimal site fast-path handle (slot index only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticBackendSlot {
    pub backend_id: StaticRootSlot,
}

/// Materialized HTTP body for a static dispatch outcome (protocol-agnostic).
///
/// Avoids forcing every response into heap `Vec<u8>` — HEAD and future sendfile paths stay viable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticDispatchBody {
    /// HEAD or zero-length 204-style responses.
    Empty,
    /// Small/medium bodies served inline (module may cap size before choosing sendfile).
    Inline(Vec<u8>),
    /// KD2.3+: module-owned sendfile asset handle (registry id).
    SendfileHandle(SendfileHandle),
}

impl StaticDispatchBody {
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn inline_len(&self) -> Option<usize> {
        match self {
            Self::Inline(b) => Some(b.len()),
            _ => None,
        }
    }
}

/// Materialized HTTP outcome from a static backend (protocol-agnostic body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticDispatchOutcome {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: StaticDispatchBody,
}

/// Optional file identity snapshot for response-cache revalidation (module-owned capture).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticResourceIdentitySnapshot {
    pub canonical_path: String,
    pub file_len: u64,
    pub modified_unix_secs: Option<u64>,
    /// Subsecond fraction of [`modified_unix_secs`] (nanoseconds within the second).
    pub modified_subsec_nanos: Option<u32>,
    #[cfg(unix)]
    pub dev: Option<u64>,
    #[cfg(unix)]
    pub ino: Option<u64>,
}

/// Aggregate static counters (implementation lives in the module).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StaticMetricsSnapshot {
    pub responses_200: u64,
    pub responses_404: u64,
    pub responses_403: u64,
    pub responses_500: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub sendfile_engagements: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticRegisterError {
    AlreadyRegistered,
    Poisoned,
    InvalidRootSlot,
}

/// Registration bundle — module-built runtime plus compiled root slot metadata.
#[derive(Clone)]
pub struct StaticRuntimeRegistration {
    pub service: std::sync::Arc<dyn StaticDispatchService>,
    /// Root slots this runtime serves (compile-time indices from snapshot).
    pub root_slots: Vec<StaticRootSlot>,
}

/// Module-owned static runtime — resolution, sendfile policy, cache load, metrics.
#[async_trait]
pub trait StaticDispatchService: Send + Sync {
    /// Hyper/Tokio path: serve a request against a compiled root slot.
    async fn dispatch(&self, request: StaticDispatchRequest) -> StaticDispatchOutcome;

    /// Optional synchronous load for shared response-cache insert (GET/HEAD body + identity).
    async fn load_for_cache(&self, request: StaticDispatchRequest) -> StaticCacheLoadOutcome;

    fn metrics(&self) -> StaticMetricsSnapshot {
        StaticMetricsSnapshot::default()
    }

    /// Bind compiled slots after snapshot compile/reload (module builds operational roots).
    fn bind_compiled_slots(&self, _generation: u64, _slots: &[StaticCompiledSlot]) {}

    /// Materialize dispatch outcome to hyper-visible bytes (sendfile handle take is module-owned).
    fn materialize_outcome(&self, outcome: StaticDispatchOutcome) -> MaterializedBackendOutcome {
        match outcome.body {
            StaticDispatchBody::Empty => MaterializedBackendOutcome {
                status: outcome.status,
                headers: outcome.headers,
                body: Vec::new(),
            },
            StaticDispatchBody::Inline(body) => MaterializedBackendOutcome {
                status: outcome.status,
                headers: outcome.headers,
                body,
            },
            StaticDispatchBody::SendfileHandle(_) => MaterializedBackendOutcome {
                status: 500,
                headers: Vec::new(),
                body: b"sendfile handle not materialized".to_vec(),
            },
        }
    }

    /// KD2.4+ cache policy — module implements; core store delegates here.
    fn snapshot_matches_current(&self, _snapshot: &StaticResourceIdentitySnapshot) -> bool {
        false
    }

    fn canonical_path_for_invalidation(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    fn canonical_path_for_snapshot(&self, snapshot: &StaticResourceIdentitySnapshot) -> PathBuf {
        PathBuf::from(&snapshot.canonical_path)
    }

    fn cache_storage_method(&self, client: StaticMethod) -> StaticMethod {
        client
    }

    fn note_cache_revalidation_success(&self) {}
    fn note_cache_revalidation_failure(&self) {}
    fn note_cache_invalidation(&self) {}

    fn cache_revalidation_success_total(&self) -> u64 {
        0
    }

    fn cache_revalidation_failure_total(&self) -> u64 {
        0
    }

    fn cache_invalidations_total(&self) -> u64 {
        0
    }

    /// Directory-index overlay: first existing child URI under a root slot.
    fn probe_static_index(
        &self,
        _root_slot: StaticRootSlot,
        _dir_uri: &str,
        _candidates: &[String],
    ) -> Option<String> {
        None
    }

    /// Serve a resolved filesystem path (htaccess static fallback).
    ///
    /// `materialization_budget_bytes` mirrors [`StaticDispatchRequest::materialization_budget_bytes`].
    async fn serve_resolved_path(
        &self,
        _method: StaticMethod,
        _path: &Path,
        _materialization_budget_bytes: Option<u64>,
    ) -> StaticDispatchOutcome {
        StaticDispatchOutcome {
            status: 501,
            headers: Vec::new(),
            body: StaticDispatchBody::Inline(b"not implemented".to_vec()),
        }
    }
}

/// Request context passed from core routing shell to the static module.
///
/// `request_path` uses `Arc<str>` so the kernel can share compiled route keys without cloning
/// per dispatch. Headers are optional — omit when the module does not need conditional logic.
///
/// `materialization_budget_bytes` is a transport-agnostic optional ceiling on how many body
/// bytes the static module may materialize for this request. `None` preserves historical
/// unbounded inline reads (H1/H2/Hyper default). Callers that need a ceiling (e.g. H3 entry)
/// pass `Some(limit)` — the module must not import protocol-specific types to interpret it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticDispatchRequest {
    pub root_slot: StaticRootSlot,
    pub method: StaticMethod,
    pub request_path: Arc<str>,
    pub headers: Vec<(String, String)>,
    /// When `Some`, static must not fully materialize a body larger than this many bytes.
    pub materialization_budget_bytes: Option<u64>,
}

/// Private header set on static outcomes when materialization was refused due to budget.
///
/// Protocol adapters (e.g. H3 entry) map this signal to their oversized-response policy.
/// Not a public product API; name is transport-agnostic.
pub const MATERIALIZATION_BUDGET_EXCEEDED_HEADER: &str =
    "x-exyonq-materialization-budget-exceeded";

/// Result of a cache-oriented static load (module maps errors to HTTP outcomes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCacheLoadOutcome {
    pub outcome: StaticDispatchOutcome,
    pub identity: Option<StaticResourceIdentitySnapshot>,
    pub cacheable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_outcome_supports_inline_and_empty() {
        let inline = StaticDispatchOutcome {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: StaticDispatchBody::Inline(b"ok".to_vec()),
        };
        assert_eq!(inline.status, 200);
        assert_eq!(inline.body.inline_len(), Some(2));

        let head = StaticDispatchOutcome {
            status: 200,
            headers: vec![("content-length".into(), "0".into())],
            body: StaticDispatchBody::Empty,
        };
        assert!(head.body.is_empty());
    }

    #[test]
    fn request_path_shared_via_arc_str() {
        let path: Arc<str> = Arc::from("/site/index.html");
        let req = StaticDispatchRequest {
            root_slot: 0,
            method: StaticMethod::Get,
            request_path: Arc::clone(&path),
            headers: Vec::new(),
            materialization_budget_bytes: None,
        };
        assert_eq!(req.request_path.as_ref(), "/site/index.html");
    }
}
