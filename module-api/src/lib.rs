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
//! ExyonQ Module API v0 — static hooks for official modules.
//!
//! **Deprecated:** prefer [`exyonq_addon_api`] (addon-api 1.x, ADR-008).
//! This crate re-exports addon-api 1.x types and wraps [`AddonRegistry`] for
//! backward-compatible async [`Module`] pipeline integration.

pub mod acme_integration;
pub mod cache;
pub mod cache_coordination;
pub mod cache_purge;
pub mod cross_cutting_pipeline;
pub mod discovery_runtime;
pub mod fcgi_dispatch;
pub mod fcgi_script_resolver;
pub mod htaccess_overlay;
pub mod htaccess_runtime;
pub mod http3_runtime;
pub mod kernel_control;
pub mod kernel_observation;
pub mod observability_runtime;
pub mod proxy_dispatch;
pub mod proxy_wire;
pub mod reload_runtime;
pub mod route_rules;
pub mod static_dispatch;
#[cfg(target_os = "linux")]
pub mod static_epoll;
pub mod static_paths;
pub mod static_wire;
pub mod tls_runtime;

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use semver::Version;
use std::error::Error;
use std::sync::Arc;

// Re-export addon-api 1.x surface for backward-compatible imports.
pub use acme_integration::{
    acme_integration_registration_test_gate, acme_integration_service,
    clear_acme_integration_for_tests, register_acme_integration_service,
    AcmeIntegrationRegisterError, AcmeIntegrationService, AcmeIntegrationTestGuard,
    CertificatePublicationPort, ACME_HTTP01_WELL_KNOWN_PREFIX,
};
pub use cache::{
    aggregate_cookie_headers, assess_cacheability, cookie_header_names, filter_storable_headers,
    fpc_assess_store, fpc_canonicalize_path, fpc_cookie_bypass, fpc_cookie_name_hard_denied,
    fpc_path_query_bypass, fpc_query_decision, fpc_request_bypass, fpc_request_evaluate,
    request_eligible_for_cache, BypassReason, CacheRejection, CompiledCachePolicy,
    FpcQueryDecision, FpcStoreReject, CACHE_DEFAULT_MAX_OBJECT_BYTES, CACHE_DEFAULT_TTL_SECS,
    CACHE_MAX_ENTRIES, CACHE_MAX_TOTAL_BYTES, CACHE_STORE_HEADER_NAMES,
    FPC_COOKIE_HARD_DENY_PREFIXES,
};
pub use cache_coordination::{
    event_from_purge_op, invalidation_event_to_purge_op, now_unix_ms, validate_invalidation_event,
    CoordinationError, CoordinationRejectReason, DistributedCacheCoordConfig, GenerationStore,
    InvalidationEvent, InvalidationOperation, InvalidationPublisher, InvalidationSubscriber,
    UrlTarget, COORDINATION_PROTOCOL_VERSION, DEFAULT_DEDUP_CAPACITY, DEFAULT_DEDUP_TTL_MS,
    DEFAULT_MAX_EVENT_BYTES, DEFAULT_MAX_PENDING_EVENTS, MAX_NODE_ID_BYTES, MAX_URL_FIELD_BYTES,
};
pub use cache_purge::{CachePurgeOp, CachePurgeOutcome, CachePurgePort, CachePurgeSocketConfig};
pub use cross_cutting_pipeline::{
    CompiledPipelineFlags, FROZEN_PIPELINE_AROUND_CORE_ORDER, PIPELINE_STAGE_CORE_DISPATCH,
    PIPELINE_STAGE_REQUEST_FILTERS, PIPELINE_STAGE_RESPONSE_FILTERS,
};
pub use discovery_runtime::{
    clear_discovery_runtime_for_tests, discovery_path_from_env,
    discovery_runtime_registration_test_gate, discovery_runtime_service,
    register_discovery_runtime_service, DiscoveryConfigOverlayService,
    DiscoveryRuntimeRegisterError, DiscoveryRuntimeTestGuard,
};
pub use exyonq_addon_api::{
    Addon, AddonConfig, AddonDescriptor, AddonError, AddonRegistry, ApiVersion, BuildKind,
    Capability, ConnectionView, CostClass, Decision, HookContext, RequestCompleteStats,
    RequestView, ResponseStub, ResponseView, RouteMeta, RuntimeProfile,
};
pub use fcgi_script_resolver::{
    clear_fastcgi_script_resolver_for_tests, fastcgi_script_resolver,
    fastcgi_script_resolver_registration_test_gate, register_fastcgi_script_resolver,
    FastcgiScriptResolutionOutcome, FastcgiScriptResolutionPurpose, FastcgiScriptResolutionRequest,
    FastcgiScriptResolutionService, FastcgiScriptResolverRegisterError,
    FastcgiScriptResolverTestGuard,
};
pub use htaccess_overlay::{
    join_directory_index_uri, lookup_overlay, normalize_uri_path,
    validate_directory_index_candidate, validate_front_controller_target, validate_overlay,
    validate_publish, CompiledFrontController, CompiledRedirectRule, CompiledRewriteRedirect,
    NormalizedDirectory, OverlayEntry, OverlayLookupResult, OverlayPublishError,
    RuntimePatchVhostOverlay, VhostOverlay, MAX_DIRECTORY_INDEX_CANDIDATES,
    MAX_DIRECTORY_INDEX_CANDIDATE_LEN, MAX_FRONT_CONTROLLER_TARGET_LEN,
};
pub use htaccess_runtime::{
    HtaccessOverlaySource, HtaccessRegisterError, HtaccessResourceKind, HtaccessRouteBackend,
    HtaccessRouteEvaluationOutcome, HtaccessRouteEvaluationRequest, HtaccessRuntimeProbes,
    HtaccessRuntimeService,
};
pub use http3_runtime::{
    Http3ConnectionLifecycle, Http3DispatchService, Http3DrainRejected, Http3ListenerBinding,
    Http3MaterializedResponse,
};
pub use kernel_control::{
    clear_control_plane_for_tests, control_plane_registration_test_gate, control_plane_service,
    register_control_plane_service, ControlPlaneRegisterError, ControlPlaneService,
    ControlPlaneTestGuard, KernelControlPort, KernelStatusSnapshot, OpsCommand, OpsCommandOutcome,
};
pub use kernel_observation::{
    clear_kernel_observation_for_tests, kernel_observation_registration_test_gate,
    kernel_observation_service, note_fastcgi_http_501, note_proxy_http_501, note_static_http_501,
    register_kernel_observation_service, KernelObservationRegisterError, KernelObservationService,
    KernelObservationTestGuard,
};
#[doc(hidden)]
#[cfg(test)]
pub use observability_runtime::clear_prometheus_appenders_for_tests;
pub use observability_runtime::{
    append_registered_prometheus, ensure_runtime_prometheus_hook, register_prometheus_appender,
    PrometheusAppendFn,
};
pub use reload_runtime::{
    clear_reload_runtime_for_tests, register_reload_runtime_service,
    reload_runtime_registration_test_gate, reload_runtime_service, ReloadRuntimeRegisterError,
    ReloadRuntimeService, ReloadRuntimeSupervisor, ReloadRuntimeTestGuard,
};
pub use route_rules::{
    evaluate_structural_route_rules, RouteRuleInput, RouteRuleOutcome, DISPATCH_STAGE_BACKEND,
    DISPATCH_STAGE_HTACCESS_OVERLAY, DISPATCH_STAGE_ROUTE_LOOKUP, DISPATCH_STAGE_STRUCTURAL_RULES,
    FROZEN_DISPATCH_STAGE_ORDER,
};
pub use tls_runtime::{TlsAlpnProfile, TlsListenerBinding};

pub type BoxError = Box<dyn Error + Send + Sync>;
pub type Body = Full<Bytes>;
pub type HttpRequest = Request<Body>;
pub type HttpResponse = Response<Body>;

/// Internal header injected by core so modules can read the client IP.
pub const CLIENT_IP_HEADER: &str = "x-exyonq-client-ip";

/// Metadata registered for each compiled-in module (ADR-003 legacy).
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub api_version: &'static str,
}

impl ModuleInfo {
    pub fn from_descriptor(desc: &AddonDescriptor) -> Self {
        let api_version = match (desc.addon_api.major, desc.addon_api.minor) {
            (1, 0) => "1.0",
            (major, minor) => {
                // Fallback for future 1.x minors; callers should prefer static literals in hot paths.
                let _ = (major, minor);
                "1.x"
            }
        };
        Self {
            name: desc.name,
            version: desc.addon_version,
            api_version,
        }
    }
}

/// Six hook points for Module API v0 (ADR-003).
#[async_trait]
pub trait Module: Send + Sync {
    fn info(&self) -> ModuleInfo;

    async fn on_init(&self) -> Result<(), BoxError> {
        Ok(())
    }

    async fn on_request(&self, _req: &HttpRequest) -> Result<(), BoxError> {
        Ok(())
    }

    async fn on_route(&self, _req: &HttpRequest) -> Result<Option<HttpResponse>, BoxError> {
        Ok(None)
    }

    async fn on_upstream(&self, _req: &HttpRequest) -> Result<(), BoxError> {
        Ok(())
    }

    async fn on_response(
        &self,
        _req: &HttpRequest,
        _resp: &mut HttpResponse,
    ) -> Result<(), BoxError> {
        Ok(())
    }

    async fn on_shutdown(&self) -> Result<(), BoxError> {
        Ok(())
    }
}

/// Registers compiled-in modules and validates addon handshake at startup.
#[derive(Default)]
pub struct ModuleRegistry {
    addon_registry: AddonRegistry,
    modules: Vec<Arc<dyn Module>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a module that also implements [`Addon`] (official modules v0.2+).
    pub fn register_addon<M>(&mut self, module: Arc<M>) -> Result<(), AddonError>
    where
        M: Module + Addon + 'static,
    {
        self.addon_registry.register(module.clone())?;
        self.modules.push(module);
        Ok(())
    }

    /// Legacy register for modules without an [`Addon`] impl.
    pub fn register(&mut self, module: Arc<dyn Module>) {
        self.modules.push(module);
    }

    pub fn modules(&self) -> &[Arc<dyn Module>] {
        &self.modules
    }

    pub fn addon_registry(&self) -> &AddonRegistry {
        &self.addon_registry
    }

    pub fn addon_registry_mut(&mut self) -> &mut AddonRegistry {
        &mut self.addon_registry
    }

    pub fn validate_handshake(
        &self,
        core_version: &Version,
        profile: RuntimeProfile,
    ) -> Result<(), AddonError> {
        self.addon_registry
            .validate_handshake(core_version, profile)
    }

    pub async fn init_all(&self) -> Result<(), BoxError> {
        for module in &self.modules {
            module.on_init().await?;
        }
        Ok(())
    }

    pub async fn shutdown_all(&self) -> Result<(), BoxError> {
        for module in &self.modules {
            module.on_shutdown().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_addon_api::{ApiVersion, BuildKind, Capability, CostClass};
    use semver::VersionReq;

    struct DualModule {
        desc: &'static AddonDescriptor,
    }

    #[async_trait]
    impl Module for DualModule {
        fn info(&self) -> ModuleInfo {
            ModuleInfo::from_descriptor(self.desc)
        }
    }

    impl Addon for DualModule {
        fn descriptor(&self) -> &'static AddonDescriptor {
            self.desc
        }
    }

    #[test]
    fn register_addon_validates_via_wrapper() {
        let desc = Box::leak(Box::new(AddonDescriptor {
            name: "dual",
            addon_version: "0.1.0",
            addon_api: ApiVersion::V1_0,
            core_compat: VersionReq::parse(">=0.1.0, <0.5.0").unwrap(),
            capabilities: &[Capability::Telemetry],
            cost_class: CostClass::LowCostHeaderOnly,
            build: BuildKind::Static,
        }));
        let mut registry = ModuleRegistry::new();
        registry
            .register_addon(Arc::new(DualModule { desc }))
            .unwrap();
        registry
            .validate_handshake(&Version::new(0, 1, 0), RuntimeProfile::Edge)
            .unwrap();
    }
}
