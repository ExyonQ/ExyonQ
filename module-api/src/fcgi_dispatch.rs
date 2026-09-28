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
//! Plan 08 FastCGI dispatch contract (core ↔ module seam).
//!
//! KD0: core may depend only on types in this module — never on module wire/transport internals.

use async_trait::async_trait;

/// Maximum request body bytes accepted for FastCGI STDIN (aligned with mod-fastcgi cap).
pub const FCGI_MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Default in-flight capacity when registration omits explicit pool limits.
pub const DEFAULT_FCGI_MAX_CONCURRENCY: usize = 16;

/// Materialized HTTP outcome from a contract backend (core-visible, protocol-agnostic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedBackendOutcome {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Aggregate FastCGI counters exposed for observability (implementation lives in the module).
///
/// **Module runtime semantics (`responses_501`):** counts HTTP **501 outcomes produced inside
/// a registered FastCGI runtime** (e.g. executor `NotRegistered`, authorization deny paths).
/// Does **not** include HTTP 501 emitted by the core dispatch shell when no module is registered.
///
/// **Public aggregate (`exyonq_core::fcgi_responses_501_total`):** core shell HTTP 501 on
/// FastCGI contract routes plus module runtime `responses_501` — no double counting per request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FcgiMetricsSnapshot {
    /// Runtime-only 501 outcomes (registered module path).
    pub responses_501: u64,
    pub responses_501_success_not_authorized: u64,
    pub responses_200: u64,
    pub responses_502: u64,
    pub responses_503: u64,
    pub responses_504: u64,
    pub saturation_rejections: u64,
    pub inflight_current: u64,
}

/// Registration bundle — wire executor plus per-pool concurrency limits.
#[derive(Clone)]
pub struct FcgiRuntimeRegistration {
    pub executor: std::sync::Arc<dyn FcgiBackendExecutor>,
    pub pool_capacities: Vec<(u32, usize)>,
}

/// Compiled FastCGI pool slot (plan metadata → module bind; no sockets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FcgiCompiledSlot {
    pub pool_id: u32,
    pub name: String,
    pub address: String,
    pub transport: String,
    pub document_root: Option<std::path::PathBuf>,
    pub max_concurrency: u32,
    pub max_connections: u32,
    pub idle_timeout_ms: u64,
    pub total_timeout_ms: u64,
    pub checkout_timeout_ms: u64,
}

/// Failure building or publishing a FastCGI generation (fail closed → keep previous).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FcgiBindError {
    InvalidEndpoint {
        pool: String,
        detail: String,
    },
    InvalidCapacity {
        pool: String,
    },
    /// Global drain in progress — reload must not reactivate admission.
    Draining,
    Poisoned,
}

impl std::fmt::Display for FcgiBindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEndpoint { pool, detail } => {
                write!(f, "fcgi pool `{pool}` invalid endpoint: {detail}")
            }
            Self::InvalidCapacity { pool } => {
                write!(f, "fcgi pool `{pool}` invalid capacity")
            }
            Self::Draining => {
                write!(f, "fcgi bind rejected: global drain in progress")
            }
            Self::Poisoned => write!(f, "fcgi bind state poisoned"),
        }
    }
}

impl std::error::Error for FcgiBindError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FcgiRegisterError {
    AlreadyRegistered,
    Poisoned,
    InvalidCapacity,
}

/// Module-owned FastCGI runtime — pool policy, timeouts, metrics, status mapping.
#[async_trait]
pub trait FcgiDispatchService: Send + Sync {
    async fn dispatch(&self, request: FcgiDispatchRequest) -> MaterializedBackendOutcome;

    fn metrics(&self) -> FcgiMetricsSnapshot {
        FcgiMetricsSnapshot::default()
    }

    /// Begin FastCGI connection-pool drain (idle drop + reject new checkout).
    /// Default: empty; production runtime overrides.
    fn begin_drain(&self) {}

    /// Publish a new FastCGI pool generation from compiled plan slots.
    ///
    /// PREPARE builds N+1; on success COMMIT publishes and RETIRE drains N.
    /// Failure must leave the previous generation active (fail closed).
    fn bind_compiled_pools(
        &self,
        _generation: u64,
        _slots: &[FcgiCompiledSlot],
    ) -> Result<(), FcgiBindError> {
        Ok(())
    }

    /// Active FastCGI pool-registry generation (0 if never bound).
    fn active_pool_generation(&self) -> u64 {
        0
    }
}

/// Blocking executor owned by the FastCGI module (no async in this trait).
pub trait FcgiBackendExecutor: Send + Sync {
    fn dispatch(&self, request: &FcgiDispatchRequest) -> FcgiDispatchOutcome;

    /// Drain pooled sockets for this executor (default empty).
    fn begin_drain(&self) {}
}

/// HTTP + filesystem context passed into the injected executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FcgiDispatchRequest {
    pub pool_id: u32,
    pub method: String,
    pub request_uri: String,
    pub query_string: String,
    pub script_name: String,
    pub script_filename: String,
    pub path_info: Option<String>,
    pub document_root: String,
    pub server_name: String,
    pub server_port: u16,
    pub remote_addr: String,
    pub server_protocol: String,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

/// Authorized FastCGI success payload — owned by module-api, not module internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FcgiSuccessResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Closed outcome enum — core maps to live HTTP without seeing wire types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FcgiDispatchOutcome {
    /// No executor registered.
    NotRegistered,
    /// Connect refused, protocol error, incomplete response, invalid CGI headers, task failure.
    BadGateway,
    /// Connect/read/write/request timeout (including delegate timeout budget).
    GatewayTimeout,
    /// Pool capacity exhausted — immediate reject (PR5-B2).
    ServiceUnavailable,
    /// Authorized upstream success (PR5-B1+).
    Success(FcgiSuccessResponse),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_response_carries_status_headers_body() {
        let outcome = FcgiDispatchOutcome::Success(FcgiSuccessResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"ok".to_vec(),
        });
        match outcome {
            FcgiDispatchOutcome::Success(r) => {
                assert_eq!(r.status, 200);
                assert_eq!(r.body, b"ok");
            }
            _ => panic!("expected success"),
        }
    }
}
