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
//! Generic backend dispatch shell — FastCGI-specific ownership lives in `exyonq-mod-fastcgi` (KD1).

use crate::contract_service_registry::{
    ContractServiceSlot, ContractServiceTestGuard, TestServiceOverride,
};
use crate::Backend;
use exyonq_module_api::fcgi_dispatch::{
    FcgiBindError, FcgiCompiledSlot, FcgiDispatchRequest, FcgiDispatchService,
    MaterializedBackendOutcome, FCGI_MAX_REQUEST_BODY_BYTES,
};
use exyonq_module_api::fcgi_script_resolver::{
    fastcgi_script_resolver, FastcgiScriptResolutionOutcome, FastcgiScriptResolutionPurpose,
    FastcgiScriptResolutionRequest,
};
use exyonq_module_api::kernel_observation::{
    note_fastcgi_http_501, note_proxy_http_501, note_static_http_501,
};
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest, ProxyDispatchService,
    ProxyMaterializedResponse, ProxyMethod, ProxyRegisterError,
};
use exyonq_module_api::static_dispatch::{
    StaticCompiledSlot, StaticDispatchRequest, StaticDispatchService, StaticMethod,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

pub use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiRegisterError, FcgiRuntimeRegistration,
    FcgiSuccessResponse, DEFAULT_FCGI_MAX_CONCURRENCY,
};
pub use exyonq_module_api::static_dispatch::{StaticRegisterError, StaticRuntimeRegistration};

/// Alias retained for handler/cache seams.
pub type ExecuteBackendOutcome = MaterializedBackendOutcome;

thread_local! {
    static STATIC_TEST_OVERRIDE: RefCell<TestServiceOverride<dyn StaticDispatchService>> =
        RefCell::new(TestServiceOverride::Inherit);
    static FCGI_TEST_OVERRIDE: RefCell<TestServiceOverride<dyn FcgiDispatchService>> =
        RefCell::new(TestServiceOverride::Inherit);
    static PROXY_TEST_OVERRIDE: RefCell<TestServiceOverride<dyn ProxyDispatchService>> =
        RefCell::new(TestServiceOverride::Inherit);
}

/// Per-thread static dispatch override — restored on drop (parallel-safe, KD2D).
#[must_use = "test override must live for the duration of the test"]
#[allow(dead_code)]
pub struct StaticDispatchTestGuard(ContractServiceTestGuard<dyn StaticDispatchService>);

impl StaticDispatchTestGuard {
    pub fn install(service: Arc<dyn StaticDispatchService>) -> Self {
        Self(ContractServiceTestGuard::install(
            &STATIC_TEST_OVERRIDE,
            service,
        ))
    }

    pub fn force_absent() -> Self {
        Self(ContractServiceTestGuard::force_absent(
            &STATIC_TEST_OVERRIDE,
        ))
    }
}

/// Per-thread FastCGI dispatch override — restored on drop (parallel-safe, KD2D).
#[must_use = "test override must live for the duration of the test"]
#[allow(dead_code)]
pub struct FcgiDispatchTestGuard(ContractServiceTestGuard<dyn FcgiDispatchService>);

impl FcgiDispatchTestGuard {
    pub fn install(service: Arc<dyn FcgiDispatchService>) -> Self {
        Self(ContractServiceTestGuard::install(
            &FCGI_TEST_OVERRIDE,
            service,
        ))
    }

    pub fn force_absent() -> Self {
        Self(ContractServiceTestGuard::force_absent(&FCGI_TEST_OVERRIDE))
    }
}

/// Per-thread proxy dispatch override — restored on drop (parallel-safe, KD3.2).
#[must_use = "test override must live for the duration of the test"]
#[allow(dead_code)]
pub struct ProxyDispatchTestGuard(ContractServiceTestGuard<dyn ProxyDispatchService>);

impl ProxyDispatchTestGuard {
    pub fn install(service: Arc<dyn ProxyDispatchService>) -> Self {
        Self(ContractServiceTestGuard::install(
            &PROXY_TEST_OVERRIDE,
            service,
        ))
    }

    pub fn force_absent() -> Self {
        Self(ContractServiceTestGuard::force_absent(&PROXY_TEST_OVERRIDE))
    }
}

static FCGI_SERVICE: ContractServiceSlot<dyn FcgiDispatchService> = ContractServiceSlot::new();
static STATIC_SERVICE: ContractServiceSlot<dyn StaticDispatchService> = ContractServiceSlot::new();
static PROXY_SERVICE: ContractServiceSlot<dyn ProxyDispatchService> = ContractServiceSlot::new();

/// Serializes integration tests that snapshot/increment global FastCGI response counters.
static FCGI_METRICS_ASSERT_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Hold for the full test body (including `.await`) when asserting `fcgi_responses_*` deltas.
#[doc(hidden)]
pub async fn fcgi_metrics_assert_guard() -> tokio::sync::MutexGuard<'static, ()> {
    FCGI_METRICS_ASSERT_GATE.lock().await
}

/// Serializes integration tests that snapshot/increment process-wide `proxy_http_501_total`.
static PROXY_METRICS_ASSERT_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Hold for the full test body (including `.await`) when asserting proxy HTTP 501 counter deltas.
#[doc(hidden)]
pub async fn proxy_metrics_assert_guard() -> tokio::sync::MutexGuard<'static, ()> {
    PROXY_METRICS_ASSERT_GATE.lock().await
}

/// Test-only global slot reset — **only** for register-once tests under
/// [`contract_service_registration_test_gate`]; production has no reset API.
#[doc(hidden)]
pub fn clear_global_static_dispatch_for_register_once_test() {
    STATIC_SERVICE.clear_for_tests();
}

/// Test-only global slot reset — register-once FastCGI tests under gate only.
#[doc(hidden)]
pub fn clear_global_fcgi_dispatch_for_register_once_test() {
    FCGI_SERVICE.clear_for_tests();
}

/// Test-only global slot reset — register-once proxy tests under gate only.
#[doc(hidden)]
pub fn clear_global_proxy_dispatch_for_register_once_test() {
    PROXY_SERVICE.clear_for_tests();
}

/// Composition-root registration — pass a module-built [`StaticDispatchService`].
pub fn register_static_dispatch_service(
    service: Arc<dyn StaticDispatchService>,
) -> Result<(), StaticRegisterError> {
    STATIC_SERVICE.register_static(service)
}

fn registered_static_service() -> Option<Arc<dyn StaticDispatchService>> {
    let override_state = STATIC_TEST_OVERRIDE.with(|cell| cell.borrow().clone());
    STATIC_SERVICE.resolve(&override_state)
}

/// Bind compiled static slots into the registered module runtime (reload-safe, KD2.5).
pub fn bind_static_compiled_slots(generation: u64, slots: &[StaticCompiledSlot]) {
    if let Some(service) = registered_static_service() {
        service.bind_compiled_slots(generation, slots);
    }
}

/// Composition-root registration — pass a module-built [`ProxyDispatchService`].
pub fn register_proxy_dispatch_service(
    service: Arc<dyn ProxyDispatchService>,
) -> Result<(), ProxyRegisterError> {
    PROXY_SERVICE.register_proxy(service)
}

fn registered_proxy_service() -> Option<Arc<dyn ProxyDispatchService>> {
    let override_state = PROXY_TEST_OVERRIDE.with(|cell| cell.borrow().clone());
    PROXY_SERVICE.resolve(&override_state)
}

/// Bind compiled proxy clusters into the registered module runtime (reload-safe, KD3.2).
pub fn bind_proxy_compiled_slots(generation: u64, slots: &[ProxyCompiledSlot]) {
    if let Some(service) = registered_proxy_service() {
        service.bind_compiled_slots(generation, slots);
    }
}

/// Cap048: build `FcgiCompiledSlot` list from IR (sorted pool names → dense pool_id).
pub fn fcgi_compiled_slots_from_config(config: &crate::config::AppConfig) -> Vec<FcgiCompiledSlot> {
    let mut names: Vec<String> = config.pools_fcgi.keys().cloned().collect();
    names.sort();
    names
        .into_iter()
        .enumerate()
        .map(|(idx, name)| {
            let pool = config
                .pools_fcgi
                .get(&name)
                .expect("pool name present in map");
            FcgiCompiledSlot {
                pool_id: idx as u32,
                name,
                address: pool.address.clone(),
                transport: pool.transport.clone(),
                document_root: pool.document_root.clone(),
                max_concurrency: pool.max_concurrency,
                max_connections: pool.max_connections.unwrap_or(pool.max_concurrency),
                idle_timeout_ms: pool.idle_timeout_ms,
                total_timeout_ms: pool.total_timeout_ms,
                checkout_timeout_ms: pool.checkout_timeout_ms,
            }
        })
        .collect()
}

/// Cap048: publish FastCGI pool generation via registered dispatch service.
pub fn bind_fcgi_compiled_pools(
    generation: u64,
    slots: &[FcgiCompiledSlot],
) -> Result<(), FcgiBindError> {
    let Some(service) = registered_service() else {
        return Ok(());
    };
    service.bind_compiled_pools(generation, slots)
}

/// Build a proxy dispatch request for the registered module runtime.
#[allow(clippy::too_many_arguments)]
pub fn build_proxy_dispatch_request(
    cluster_id: u32,
    method: ProxyMethod,
    path_and_query: impl Into<String>,
    host: Option<String>,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    remote_addr: impl Into<String>,
    scheme: impl Into<String>,
) -> ProxyDispatchRequest {
    ProxyDispatchRequest {
        cluster_id,
        method,
        path_and_query: path_and_query.into(),
        host,
        headers,
        body,
        remote_addr: remote_addr.into(),
        scheme: scheme.into(),
    }
}

pub fn materialize_proxy_materialized(
    response: ProxyMaterializedResponse,
) -> MaterializedBackendOutcome {
    MaterializedBackendOutcome {
        status: response.status,
        headers: response.headers,
        body: response.body,
    }
}

pub fn materialize_proxy_outcome(outcome: ProxyDispatchOutcome) -> MaterializedBackendOutcome {
    match outcome {
        ProxyDispatchOutcome::Materialized(response) => materialize_proxy_materialized(response),
        ProxyDispatchOutcome::BadGateway => MaterializedBackendOutcome {
            status: 502,
            headers: Vec::new(),
            body: Vec::new(),
        },
        ProxyDispatchOutcome::GatewayTimeout => MaterializedBackendOutcome {
            status: 504,
            headers: Vec::new(),
            body: Vec::new(),
        },
        ProxyDispatchOutcome::ServiceUnavailable => MaterializedBackendOutcome {
            status: 503,
            headers: Vec::new(),
            body: Vec::new(),
        },
        ProxyDispatchOutcome::NotRegistered => MaterializedBackendOutcome {
            status: 501,
            headers: Vec::new(),
            body: b"not implemented".to_vec(),
        },
        ProxyDispatchOutcome::Streaming { .. } | ProxyDispatchOutcome::Upgraded { .. } => {
            MaterializedBackendOutcome {
                status: 502,
                headers: Vec::new(),
                body: Vec::new(),
            }
        }
    }
}

pub async fn dispatch_proxy(request: ProxyDispatchRequest) -> ProxyDispatchOutcome {
    let Some(service) = registered_proxy_service() else {
        return ProxyDispatchOutcome::NotRegistered;
    };
    service.dispatch(request).await
}

/// Map contract outcome to live Hyper response (Materialized + attach Streaming/Upgraded).
pub fn proxy_outcome_to_hyper(
    outcome: ProxyDispatchOutcome,
) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>> {
    use http_body_util::{BodyExt, Full};
    use hyper::{Response, StatusCode};

    type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

    fn boxed(body: bytes::Bytes) -> BoxBody {
        Full::from(body).map_err(|never| match never {}).boxed()
    }

    fn text(status: StatusCode, message: &str) -> Response<BoxBody> {
        Response::builder()
            .status(status)
            .header("content-type", "text/plain; charset=utf-8")
            .body(boxed(bytes::Bytes::from(message.to_string())))
            .expect("valid response")
    }

    match outcome {
        ProxyDispatchOutcome::Materialized(m) => {
            let mut builder = Response::builder().status(m.status);
            for (name, value) in m.headers {
                builder = builder.header(name, value);
            }
            builder
                .body(boxed(bytes::Bytes::from(m.body)))
                .expect("materialized")
        }
        ProxyDispatchOutcome::Streaming { stream, .. } => exyonq_mod_proxy::take_streaming(stream)
            .unwrap_or_else(|| text(StatusCode::BAD_GATEWAY, "proxy stream attach failed")),
        ProxyDispatchOutcome::Upgraded(handle) => exyonq_mod_proxy::take_websocket(handle)
            .unwrap_or_else(|| text(StatusCode::BAD_GATEWAY, "proxy upgrade attach failed")),
        ProxyDispatchOutcome::BadGateway => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(boxed(bytes::Bytes::new()))
            .expect("502"),
        ProxyDispatchOutcome::GatewayTimeout => Response::builder()
            .status(StatusCode::GATEWAY_TIMEOUT)
            .body(boxed(bytes::Bytes::new()))
            .expect("504"),
        ProxyDispatchOutcome::ServiceUnavailable => Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(boxed(bytes::Bytes::new()))
            .expect("503"),
        ProxyDispatchOutcome::NotRegistered => text(StatusCode::NOT_IMPLEMENTED, "not implemented"),
    }
}

/// Build a static dispatch request. `request_path` is shared via `Arc<str>` when caller passes `Arc::clone`.
pub fn build_static_dispatch_request(
    root_slot: u32,
    method: StaticMethod,
    request_path: Arc<str>,
    headers: Vec<(String, String)>,
) -> StaticDispatchRequest {
    build_static_dispatch_request_with_budget(root_slot, method, request_path, headers, None)
}

/// Like [`build_static_dispatch_request`] with an optional materialization ceiling.
pub fn build_static_dispatch_request_with_budget(
    root_slot: u32,
    method: StaticMethod,
    request_path: Arc<str>,
    headers: Vec<(String, String)>,
    materialization_budget_bytes: Option<u64>,
) -> StaticDispatchRequest {
    StaticDispatchRequest {
        root_slot,
        method,
        request_path,
        headers,
        materialization_budget_bytes,
    }
}

/// Build request from `&str` path — **one allocation** (`Arc::from`) per call unless caller shares `Arc<str>`.
pub fn build_static_dispatch_request_from_str(
    root_slot: u32,
    method: StaticMethod,
    request_path: &str,
    headers: Vec<(String, String)>,
) -> StaticDispatchRequest {
    build_static_dispatch_request(root_slot, method, Arc::from(request_path), headers)
}

/// Build request from `&str` with an optional materialization ceiling (`None` = historical behaviour).
pub fn build_static_dispatch_request_from_str_with_budget(
    root_slot: u32,
    method: StaticMethod,
    request_path: &str,
    headers: Vec<(String, String)>,
    materialization_budget_bytes: Option<u64>,
) -> StaticDispatchRequest {
    build_static_dispatch_request_with_budget(
        root_slot,
        method,
        Arc::from(request_path),
        headers,
        materialization_budget_bytes,
    )
}

pub fn materialize_static_outcome(
    outcome: exyonq_module_api::static_dispatch::StaticDispatchOutcome,
) -> MaterializedBackendOutcome {
    registered_static_service()
        .map(|service| service.materialize_outcome(outcome))
        .unwrap_or(MaterializedBackendOutcome {
            status: 501,
            headers: Vec::new(),
            body: b"not implemented".to_vec(),
        })
}

pub fn invalidate_static_response_cache(path: &std::path::Path) {
    exyonq_mod_static::invalidate_static_path(exyonq_cache::global_response_cache(), path);
}

/// Cache policy delegation (KD2.5 — core store shell only).
pub fn static_snapshot_matches_current(
    snap: &exyonq_module_api::static_dispatch::StaticResourceIdentitySnapshot,
) -> bool {
    registered_static_service()
        .map(|s| s.snapshot_matches_current(snap))
        .unwrap_or(false)
}

pub fn static_canonical_path_for_invalidation(path: &std::path::Path) -> PathBuf {
    registered_static_service()
        .map(|s| s.canonical_path_for_invalidation(path))
        .unwrap_or_else(|| path.to_path_buf())
}

pub fn static_canonical_path_for_snapshot(
    snap: &exyonq_module_api::static_dispatch::StaticResourceIdentitySnapshot,
) -> PathBuf {
    registered_static_service()
        .map(|s| s.canonical_path_for_snapshot(snap))
        .unwrap_or_else(|| PathBuf::from(&snap.canonical_path))
}

pub fn static_cache_storage_method(client: StaticMethod) -> StaticMethod {
    registered_static_service()
        .map(|s| s.cache_storage_method(client))
        .unwrap_or(client)
}

pub fn note_static_cache_revalidation_success() {
    if let Some(s) = registered_static_service() {
        s.note_cache_revalidation_success();
    }
}

pub fn note_static_cache_revalidation_failure() {
    if let Some(s) = registered_static_service() {
        s.note_cache_revalidation_failure();
    }
}

pub fn note_static_cache_invalidation() {
    if let Some(s) = registered_static_service() {
        s.note_cache_invalidation();
    }
}

pub fn probe_static_index(root_slot: u32, dir_uri: &str, candidates: &[String]) -> Option<String> {
    registered_static_service().and_then(|s| s.probe_static_index(root_slot, dir_uri, candidates))
}

pub async fn serve_static_resolved_path(
    method: StaticMethod,
    path: &std::path::Path,
) -> MaterializedBackendOutcome {
    serve_static_resolved_path_with_budget(method, path, None).await
}

pub async fn serve_static_resolved_path_with_budget(
    method: StaticMethod,
    path: &std::path::Path,
    materialization_budget_bytes: Option<u64>,
) -> MaterializedBackendOutcome {
    let Some(service) = registered_static_service() else {
        return MaterializedBackendOutcome {
            status: 501,
            headers: Vec::new(),
            body: b"not implemented".to_vec(),
        };
    };
    materialize_static_outcome(
        service
            .serve_resolved_path(method, path, materialization_budget_bytes)
            .await,
    )
}

/// Composition-root registration — pass a module-built [`FcgiDispatchService`].
pub fn register_fcgi_dispatch_service(
    service: Arc<dyn FcgiDispatchService>,
) -> Result<(), FcgiRegisterError> {
    FCGI_SERVICE.register(service)
}

/// Deprecated — use `register_fcgi_dispatch_service(Arc::new(FcgiRuntime::new(reg)?))` from CLI/tests.
pub fn register_fcgi_runtime(
    registration: FcgiRuntimeRegistration,
) -> Result<(), FcgiRegisterError> {
    let _ = registration;
    Err(FcgiRegisterError::AlreadyRegistered)
}

/// Deprecated alias for [`register_fcgi_dispatch_service`].
pub fn register_fcgi_executor(
    service: Arc<dyn FcgiDispatchService>,
) -> Result<(), FcgiRegisterError> {
    register_fcgi_dispatch_service(service)
}

fn registered_service() -> Option<Arc<dyn FcgiDispatchService>> {
    let override_state = FCGI_TEST_OVERRIDE.with(|cell| cell.borrow().clone());
    FCGI_SERVICE.resolve(&override_state)
}

pub(crate) fn fcgi_dispatch_service_present() -> bool {
    registered_service().is_some()
}

/// Build a dispatch request from HTTP context (script resolution delegated to FastCGI module).
#[allow(clippy::too_many_arguments)]
pub fn build_fcgi_dispatch_request(
    pool_id: u32,
    pool_document_root: Option<&PathBuf>,
    method: &str,
    uri_path: &str,
    query_string: &str,
    request_uri: &str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    remote_addr: &str,
    default_server_port: u16,
) -> Result<FcgiDispatchRequest, FastcgiScriptResolutionOutcome> {
    let Some(resolver) = fastcgi_script_resolver() else {
        return Err(FastcgiScriptResolutionOutcome::ProbeError);
    };
    let outcome = resolver.resolve(&FastcgiScriptResolutionRequest {
        uri_path: uri_path.to_string(),
        pool_document_root: pool_document_root.cloned(),
        generation: 0,
        purpose: FastcgiScriptResolutionPurpose::Dispatch,
    });

    let FastcgiScriptResolutionOutcome::Resolved {
        script_filename,
        script_name,
        document_root,
        path_info,
    } = outcome
    else {
        return Err(outcome);
    };

    let (server_name, server_port) = server_name_port_from_headers(&headers, default_server_port);
    let content_type = if body.is_empty() {
        None
    } else {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.clone())
    };

    Ok(FcgiDispatchRequest {
        pool_id,
        method: method.to_string(),
        request_uri: request_uri.to_string(),
        query_string: query_string.to_string(),
        script_name,
        script_filename,
        path_info,
        document_root,
        server_name,
        server_port,
        remote_addr: remote_addr.to_string(),
        server_protocol: "HTTP/1.1".to_string(),
        content_type,
        body,
        headers,
    })
}

fn server_name_port_from_headers(headers: &[(String, String)], default_port: u16) -> (String, u16) {
    let host = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.as_str())
        .unwrap_or("localhost");
    if let Some((name, port_str)) = host.rsplit_once(':') {
        if !name.is_empty() {
            if let Ok(port) = port_str.parse::<u16>() {
                return (name.to_string(), port);
            }
        }
    }
    (host.to_string(), default_port)
}

/// Thin dispatch shell — delegates contract backends to registered module services.
pub async fn execute_backend(
    backend: &Backend,
    fcgi_request: Option<FcgiDispatchRequest>,
    static_request: Option<StaticDispatchRequest>,
    proxy_request: Option<ProxyDispatchRequest>,
) -> Option<ExecuteBackendOutcome> {
    match backend {
        Backend::Fastcgi { pool_id } => {
            let Some(service) = registered_service() else {
                note_fastcgi_http_501();
                return Some(MaterializedBackendOutcome {
                    status: 501,
                    headers: Vec::new(),
                    body: b"not implemented".to_vec(),
                });
            };
            let request = match fcgi_request {
                Some(mut request) => {
                    request.pool_id = *pool_id;
                    request
                }
                None => FcgiDispatchRequest {
                    pool_id: *pool_id,
                    method: String::new(),
                    request_uri: String::new(),
                    query_string: String::new(),
                    script_name: String::new(),
                    script_filename: String::new(),
                    path_info: None,
                    document_root: String::new(),
                    server_name: String::new(),
                    server_port: 80,
                    remote_addr: String::new(),
                    server_protocol: String::new(),
                    content_type: None,
                    body: Vec::new(),
                    headers: Vec::new(),
                },
            };
            Some(service.dispatch(request).await)
        }
        Backend::Static { root_slot } => {
            let Some(service) = registered_static_service() else {
                note_static_http_501();
                return Some(MaterializedBackendOutcome {
                    status: 501,
                    headers: Vec::new(),
                    body: b"not implemented".to_vec(),
                });
            };
            let request = match static_request {
                Some(mut request) => {
                    request.root_slot = *root_slot;
                    request
                }
                None => StaticDispatchRequest {
                    root_slot: *root_slot,
                    method: StaticMethod::Get,
                    request_path: Arc::from("/"),
                    headers: Vec::new(),
                    materialization_budget_bytes: None,
                },
            };
            let outcome = service.dispatch(request).await;
            Some(materialize_static_outcome(outcome))
        }
        Backend::Module { .. } => Some(MaterializedBackendOutcome {
            status: 501,
            headers: Vec::new(),
            body: b"not implemented".to_vec(),
        }),
        Backend::Proxy { cluster_id } => {
            let Some(service) = registered_proxy_service() else {
                note_proxy_http_501();
                return Some(MaterializedBackendOutcome {
                    status: 501,
                    headers: Vec::new(),
                    body: b"not implemented".to_vec(),
                });
            };
            let request = match proxy_request {
                Some(mut request) => {
                    request.cluster_id = *cluster_id;
                    request
                }
                None => ProxyDispatchRequest {
                    cluster_id: *cluster_id,
                    method: ProxyMethod::Get,
                    path_and_query: "/".into(),
                    host: None,
                    headers: Vec::new(),
                    body: None,
                    remote_addr: "127.0.0.1".into(),
                    scheme: "http".into(),
                },
            };
            let outcome = service.dispatch(request).await;
            Some(materialize_proxy_outcome(outcome))
        }
    }
}

/// Load static body + identity for response cache (module-owned).
pub async fn load_static_for_cache(
    request: StaticDispatchRequest,
) -> Option<exyonq_module_api::static_dispatch::StaticCacheLoadOutcome> {
    let service = registered_static_service()?;
    Some(service.load_for_cache(request).await)
}

/// Request body cap for FastCGI STDIN (re-exported seam constant).
pub const FCGI_REQUEST_BODY_LIMIT: usize = FCGI_MAX_REQUEST_BODY_BYTES;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use exyonq_mod_static::StaticRuntime;
    use exyonq_module_api::static_dispatch::StaticDispatchService;

    struct StubService(MaterializedBackendOutcome);

    #[async_trait]
    impl FcgiDispatchService for StubService {
        async fn dispatch(&self, _request: FcgiDispatchRequest) -> MaterializedBackendOutcome {
            self.0.clone()
        }
    }

    #[test]
    fn host_with_nonstandard_port_sets_server_name_and_port() {
        let headers = [("Host".to_string(), "localhost:5006".to_string())];
        let (name, port) = server_name_port_from_headers(&headers, 80);
        assert_eq!(name, "localhost");
        assert_eq!(port, 5006);
    }

    #[test]
    fn host_without_port_uses_the_listen_port() {
        let headers = [("Host".to_string(), "localhost".to_string())];
        let (name, port) = server_name_port_from_headers(&headers, 5006);
        assert_eq!(name, "localhost");
        assert_eq!(port, 5006);
    }

    #[tokio::test]
    async fn fastcgi_without_service_returns_501() {
        let _guard = FcgiDispatchTestGuard::force_absent();
        let outcome = execute_backend(&Backend::Fastcgi { pool_id: 0 }, None, None, None)
            .await
            .expect("Fastcgi");
        assert_eq!(outcome.status, 501);
    }

    #[tokio::test]
    async fn static_without_service_returns_501() {
        let _guard = StaticDispatchTestGuard::force_absent();
        let outcome = execute_backend(&Backend::Static { root_slot: 0 }, None, None, None)
            .await
            .expect("Static");
        assert_eq!(outcome.status, 501);
    }

    #[tokio::test]
    async fn proxy_delegated_to_legacy_paths() {
        let _guard = ProxyDispatchTestGuard::force_absent();
        let outcome = execute_backend(&Backend::Proxy { cluster_id: 0 }, None, None, None)
            .await
            .expect("Proxy");
        assert_eq!(outcome.status, 501);
    }

    #[tokio::test]
    async fn registered_service_dispatches() {
        let _guard =
            FcgiDispatchTestGuard::install(Arc::new(StubService(MaterializedBackendOutcome {
                status: 200,
                headers: vec![("content-type".into(), "text/plain".into())],
                body: b"ok".to_vec(),
            })));
        let outcome = execute_backend(
            &Backend::Fastcgi { pool_id: 0 },
            Some(FcgiDispatchRequest {
                pool_id: 0,
                method: "GET".into(),
                request_uri: "/".into(),
                query_string: String::new(),
                script_name: "/index.php".into(),
                script_filename: "/var/www/index.php".into(),
                path_info: None,
                document_root: "/var/www".into(),
                server_name: "localhost".into(),
                server_port: 80,
                remote_addr: "127.0.0.1".into(),
                server_protocol: "HTTP/1.1".into(),
                content_type: None,
                body: Vec::new(),
                headers: Vec::new(),
            }),
            None,
            None,
        )
        .await
        .expect("Fastcgi");
        assert_eq!(outcome.status, 200);
    }

    #[test]
    fn parallel_static_and_fcgi_overrides_are_isolated() {
        use std::sync::Barrier;
        use std::thread;

        let barrier = Arc::new(Barrier::new(2));
        let t1 = {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let _static = StaticDispatchTestGuard::force_absent();
                let _fcgi = FcgiDispatchTestGuard::force_absent();
                barrier.wait();
                assert!(registered_static_service().is_none());
                assert!(registered_service().is_none());
            })
        };
        let t2 = {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let _static = StaticDispatchTestGuard::install({
                    let runtime = Arc::new(StaticRuntime::new());
                    let service: Arc<dyn StaticDispatchService> = runtime.clone();
                    service
                });
                let _fcgi = FcgiDispatchTestGuard::install(Arc::new(StubService(
                    MaterializedBackendOutcome {
                        status: 200,
                        headers: Vec::new(),
                        body: b"x".to_vec(),
                    },
                )));
                barrier.wait();
                assert!(registered_static_service().is_some());
                assert!(registered_service().is_some());
            })
        };
        t1.join().unwrap();
        t2.join().unwrap();
    }
}
