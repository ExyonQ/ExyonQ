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
//! KD3.0 — Proxy dispatch contract (core ↔ module seam).
//!
//! Core may depend only on types in this module — never on Hyper client, wire loop, or pool internals.
//! Proxy differs from Static/FastCGI: outcomes may be **materialized**, **streaming**, or **upgraded** (WebSocket).
//!
//! ## Handle ownership (KD3.1 — documented; FSM deferred to KD3.4+)
//!
//! ### `ProxyStreamHandle`
//! - **Owner:** `exyonq-mod-proxy` runtime (registry of live streaming sessions).
//! - **Send/Sync:** `Copy` id only; inner session is `Send` but **not** `Sync` — registry mutex serializes lookup.
//! - **Cancel safety:** dropping handle in core after attaching to response **does not** cancel; module task owns cancel.
//! - **Drop:** module decrements registry on pump completion or client disconnect.
//! - **Terminal errors:** surfaced as HTTP 502/504 **before** handle handoff; after commit, errors end the stream only.
//! - **Backpressure:** module pump applies Hyper/wire native flow control — core does not rechunk.
//! - **After headers committed:** core must not mutate status/headers; module continues body/tunnel pump.
//! - **Reload/drain:** module cancels upstream on generation change; core rejects new handles when draining.
//! - **Metrics:** module counts upstream errors; core counts cache hit/miss on materialized path only (no double count).
//!
//! ### `ProxyWebSocketHandle`
//! - **Owner:** `exyonq-mod-proxy` (post-101 bidirectional tunnel).
//! - **Half-close:** module preserves Hyper/tokio `copy` semantics (KD3.4 implementation).
//! - **Reload/drain:** module aborts tunnel tasks; core does not retain fd references.
//! - **Metrics:** upgrade failures before 101 → module `responses_502`; no cache metrics on upgraded paths.
//!
//! ## Outcome ownership (KD3.1 audit — behavior preserved, not redesigned)
//!
//! | Outcome | Status/headers producer | Downstream writer | Upstream cancel on client drop |
//! |---------|---------------------------|-------------------|--------------------------------|
//! | `Materialized` | module (or core legacy until KD3.2) | core generic HTTP response | N/A (body complete) |
//! | `Streaming` | module selects status/headers before handle handoff | core attaches handle; module pumps | module (KD3.4) |
//! | `Upgraded` | module after 101 | module tunnel (core relinquishes) | module (KD3.4) |

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

/// Maximum request body bytes accepted for proxy forward (aligned with core security limits).
pub const PROXY_MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Compiled endpoint row carried on [`ProxyCompiledSlot`] for P2B selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyCompiledEndpoint {
    pub endpoint_id: String,
    pub http_uri: String,
    pub weight: u32,
    pub priority: u32,
    /// Desired admin eligibility (`Enabled` only). Drain/Disabled ⇒ false.
    pub admin_enabled: bool,
}

/// Compiled proxy cluster metadata in snapshot (slot-only in core after KD3.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyCompiledSlot {
    pub cluster_id: u32,
    /// Config upstream name (compile-time identity).
    pub upstream_name: String,
    /// Base target URI string from config (module resolves at runtime).
    /// Populated for single-endpoint executable clusters; multi uses `endpoints`.
    pub target: String,
    pub timeout: Duration,
    /// True when exactly one eligible endpoint is productively executable.
    pub single_endpoint_executable: bool,
    /// True when N≥2 endpoints are retained and productive WRR selection is authorized (P2B).
    pub multi_endpoint_executable: bool,
    /// Full endpoint count retained in plan (no silent truncation).
    pub endpoint_count: u32,
    /// When true, use priority bands; when false, single WRR band over all eligible.
    pub failover_priority_bands: bool,
    /// Full desired endpoint set (may include ineligible rows for generation identity).
    pub endpoints: Box<[ProxyCompiledEndpoint]>,
}

impl ProxyCompiledSlot {
    /// Test/helper constructor for legacy single-target slots.
    pub fn legacy_single(
        cluster_id: u32,
        upstream_name: impl Into<String>,
        target: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        let target = target.into();
        let upstream_name = upstream_name.into();
        Self {
            cluster_id,
            upstream_name,
            target: target.clone(),
            timeout,
            single_endpoint_executable: true,
            multi_endpoint_executable: false,
            endpoint_count: 1,
            failover_priority_bands: true,
            endpoints: Box::new([ProxyCompiledEndpoint {
                endpoint_id: "legacy".into(),
                http_uri: target,
                weight: 1,
                priority: 0,
                admin_enabled: true,
            }]),
        }
    }
}

/// Opaque module-owned streaming bridge id (Hyper `Incoming`, wire pump, or tunnel).
///
/// Core holds the handle only long enough to attach it to the generic response path —
/// no per-chunk trait objects, no mandatory rechunking layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProxyStreamHandle(pub u64);

/// Opaque WebSocket tunnel id after successful upgrade (bidirectional pump owned by module).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProxyWebSocketHandle(pub u64);

/// HTTP method subset used by kernel shell for proxy dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
    Other,
}

/// Request context passed from core shell into registered proxy runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyDispatchRequest {
    pub cluster_id: u32,
    pub method: ProxyMethod,
    /// Path and query (e.g. `/api/health?v=1`).
    pub path_and_query: String,
    pub host: Option<String>,
    pub headers: Vec<(String, String)>,
    /// Present for methods with body; bounded by [`PROXY_MAX_REQUEST_BODY_BYTES`].
    pub body: Option<Vec<u8>>,
    pub remote_addr: String,
    pub scheme: String,
}

/// Materialized upstream success (cache-eligible GET/HEAD, small responses, error pages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyMaterializedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Closed proxy outcome — core maps to live HTTP without seeing transport types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyDispatchOutcome {
    /// No module registered for proxy contract routes.
    NotRegistered,
    /// Connect refused, protocol error, incomplete response, invalid headers.
    BadGateway,
    /// Connect/read/write/request timeout (including delegate timeout budget).
    GatewayTimeout,
    /// Immediate reject when pool capacity exhausted (future tranche; preserve 503 semantics).
    ServiceUnavailable,
    /// Fully materialized response (Plan 12 cache path, small bodies).
    Materialized(ProxyMaterializedResponse),
    /// Streaming body (SSE, large download) — module pumps via [`ProxyStreamHandle`].
    Streaming {
        status: u16,
        headers: Vec<(String, String)>,
        stream: ProxyStreamHandle,
    },
    /// HTTP 101 Switching Protocols — module owns bidirectional tunnel via [`ProxyWebSocketHandle`].
    Upgraded(ProxyWebSocketHandle),
}

/// Aggregate proxy counters (implementation lives in module after KD3.2+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProxyMetricsSnapshot {
    pub responses_502: u64,
    pub responses_503: u64,
    pub responses_504: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyRegisterError {
    AlreadyRegistered,
    Poisoned,
}

/// Registration bundle — module-built runtime + compiled cluster table.
#[derive(Clone)]
pub struct ProxyRuntimeRegistration {
    pub service: Arc<dyn ProxyDispatchService>,
    pub clusters: Arc<[ProxyCompiledSlot]>,
}

/// Module-owned proxy runtime — upstream pools, timeouts, streaming, WebSocket, cache load policy.
#[async_trait]
pub trait ProxyDispatchService: Send + Sync {
    async fn dispatch(&self, request: ProxyDispatchRequest) -> ProxyDispatchOutcome;

    fn metrics(&self) -> ProxyMetricsSnapshot {
        ProxyMetricsSnapshot::default()
    }

    /// Bind compiled cluster table on reload (KD3.2+).
    fn bind_compiled_slots(&self, _generation: u64, _slots: &[ProxyCompiledSlot]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialized_outcome_carries_status_headers_body() {
        let outcome = ProxyDispatchOutcome::Materialized(ProxyMaterializedResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"ok".to_vec(),
        });
        match outcome {
            ProxyDispatchOutcome::Materialized(r) => {
                assert_eq!(r.status, 200);
                assert_eq!(r.body, b"ok");
            }
            _ => panic!("expected materialized"),
        }
    }

    #[test]
    fn streaming_outcome_carries_opaque_handle() {
        let outcome = ProxyDispatchOutcome::Streaming {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            stream: ProxyStreamHandle(42),
        };
        match outcome {
            ProxyDispatchOutcome::Streaming { stream, .. } => assert_eq!(stream.0, 42),
            _ => panic!("expected streaming"),
        }
    }
}
