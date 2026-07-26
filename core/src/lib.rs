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
//! ExyonQ core HTTP runtime.

mod acme_http01_adapter;
pub mod cache_purge_port;
mod certificate_publication_port;
pub mod config;
pub mod contract_service_registry;
pub mod coordinating_purge_port;
pub mod discovery_overlay;
pub mod execute_backend;
pub mod fcgi_cache_registry;
pub mod fpc_lookup;
pub mod fpc_store;
pub mod htaccess_probes;
pub mod htaccess_runtime_registry;
pub mod http3_runtime_registry;
pub mod http_cache;
pub mod kernel;
pub mod kernel_control_port;
pub mod lab_coord_hooks;
pub mod lifecycle;
pub mod pipeline_registry;
pub mod platform;
pub mod reload;
pub mod server;
pub mod structural_route_rules;

/// TLS runtime (KD4.5 — implementation in `exyonq-mod-tls`).
pub use exyonq_mod_tls as tls;

/// Exposed for fuzz targets and tests.
pub fn find_header_end(buf: &[u8]) -> Option<usize> {
    server::io::find_header_end(buf)
}

pub use exyonq_cache::{
    build_cache_key, build_storage_cache_key, global_response_cache, global_singleflight,
    normalize_host, set_singleflight_follower_hook_for_tests, CacheKey, CacheKeyParts,
    CacheStoreSnapshot, CachedEntry, ResponseCache, Singleflight,
};
pub use exyonq_mod_proxy::{request_headers_safe_for_proxy, ProxyClient};
pub use exyonq_mod_static::{invalidate_static_path, serve_static_with_cache, StaticCacheLoad};
pub use exyonq_runtime_plan::{Backend, BackendId, BackendTable, RouteDecision, RouteIndex};
pub use fcgi_cache_registry::{
    fcgi_cache_hooks_from_module, prepare_fcgi_cache_load, register_fcgi_cache_hooks,
    FcgiCacheHooks, FcgiCacheMetricsFns, ServeFn,
};
pub use http_cache::{apply_plan10b_cache_headers, plan10b_cache_headers_enabled};

/// Backward-compat type alias (KD4.1 — store lives in `exyonq-cache`).
pub type CachedHttpResponse = CachedEntry;

/// Process-wide response cache accessor (prefer [`global_response_cache`]).
#[allow(non_snake_case)]
pub fn GLOBAL_RESPONSE_CACHE() -> &'static ResponseCache {
    global_response_cache()
}

/// Process-wide singleflight accessor (prefer [`global_singleflight`]).
#[allow(non_snake_case)]
pub fn GLOBAL_SINGLEFLIGHT() -> &'static Singleflight {
    global_singleflight()
}

pub use config::{AppConfig, ConfigError};
#[doc(hidden)]
pub use contract_service_registry::contract_service_registration_test_gate;
#[doc(hidden)]
pub use contract_service_registry::fcgi_service_registration_test_gate;
#[doc(hidden)]
pub use execute_backend::fcgi_metrics_assert_guard;
pub use execute_backend::{
    bind_proxy_compiled_slots, bind_static_compiled_slots, build_fcgi_dispatch_request,
    build_proxy_dispatch_request, build_static_dispatch_request,
    build_static_dispatch_request_from_str, build_static_dispatch_request_from_str_with_budget,
    clear_global_fcgi_dispatch_for_register_once_test,
    clear_global_proxy_dispatch_for_register_once_test,
    clear_global_static_dispatch_for_register_once_test, dispatch_proxy, execute_backend,
    materialize_proxy_outcome, materialize_static_outcome, proxy_outcome_to_hyper,
    register_fcgi_dispatch_service, register_fcgi_executor, register_fcgi_runtime,
    register_proxy_dispatch_service, register_static_dispatch_service, ExecuteBackendOutcome,
    FcgiDispatchTestGuard, FcgiRegisterError, FcgiRuntimeRegistration, ProxyDispatchTestGuard,
    StaticDispatchTestGuard, StaticRegisterError, StaticRuntimeRegistration,
    DEFAULT_FCGI_MAX_CONCURRENCY, FCGI_REQUEST_BODY_LIMIT,
};
pub use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchRequest, FcgiDispatchService,
    FcgiMetricsSnapshot, FcgiSuccessResponse, MaterializedBackendOutcome,
};
pub use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest, ProxyDispatchService,
    ProxyMaterializedResponse, ProxyMethod, ProxyMetricsSnapshot, ProxyRegisterError,
    ProxyRuntimeRegistration, ProxyStreamHandle, ProxyWebSocketHandle,
};
pub use exyonq_module_api::static_dispatch::{
    SendfileHandle, StaticDispatchBody, StaticDispatchOutcome, StaticDispatchRequest,
    StaticDispatchService, StaticMethod, StaticMetricsSnapshot,
};
pub use exyonq_runtime_plan::{
    compile_fcgi_pool_slots, compile_htaccess_site_bindings, compile_runtime_plan,
    CompiledFcgiPool, HtaccessSiteBinding, RuntimePlan, RuntimeSnapshot, SnapshotFingerprint,
};
pub use htaccess_probes::CoreHtaccessProbes;
#[doc(hidden)]
pub use htaccess_runtime_registry::{
    clear_htaccess_runtime_for_tests, HtaccessDispatchAdjustment, HtaccessRuntimeTestGuard,
};
pub use htaccess_runtime_registry::{htaccess_adjust_route, register_htaccess_runtime_service};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
