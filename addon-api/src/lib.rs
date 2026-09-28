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
//! ExyonQ Addon API 1.x — stable extension contract (ADR-008).
//!
//! Minimal surface: manifest, handshake, opaque views, and object-safe hooks.
//! See [`docs/adr/008-addon-api-1x.md`](../../docs/adr/008-addon-api-1x.md).

use semver::{Version, VersionReq};
use std::sync::Arc;
use thiserror::Error;

/// Supported addon API semver (major/minor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApiVersion {
    pub major: u16,
    pub minor: u16,
}

impl ApiVersion {
    pub const V1_0: Self = Self { major: 1, minor: 0 };

    pub fn matches_host(&self, host: ApiVersion) -> bool {
        self.major == host.major
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// How an addon is linked into the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuildKind {
    Static,
}

/// Declared runtime cost class for budget enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CostClass {
    ZeroCostWhenDisabled,
    LowCostHeaderOnly,
    StreamingCost,
    ExternalCall,
    Experimental,
}

/// Runtime profile used during handshake cost-class checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeProfile {
    Edge,
    Lb,
    Dev,
}

impl RuntimeProfile {
    pub fn allows_cost_class(self, cost: CostClass) -> bool {
        match self {
            Self::Edge => matches!(
                cost,
                CostClass::ZeroCostWhenDisabled | CostClass::LowCostHeaderOnly
            ),
            Self::Lb => !matches!(cost, CostClass::Experimental),
            Self::Dev => true,
        }
    }
}

/// Capabilities declared in the addon manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Capability {
    ConnectionPolicy,
    RequestFilter,
    ResponseFilter,
    UpstreamPolicy,
    Telemetry,
}

/// Hook return value for connection/request/response phases.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Decision {
    Continue,
    Respond(ResponseStub),
    CloseConnection,
}

/// Minimal short-circuit response (no heavy body types in the contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseStub {
    pub status: u16,
    pub content_type: Option<&'static str>,
    pub body: &'static [u8],
}

impl ResponseStub {
    pub fn text(status: u16, body: &'static str) -> Self {
        Self {
            status,
            content_type: Some("text/plain; charset=utf-8"),
            body: body.as_bytes(),
        }
    }
}

/// Opaque connection view (v1 — populated by core in future hooks).
#[derive(Debug, Clone)]
pub struct ConnectionView {
    pub remote_addr: Option<&'static str>,
}

/// Opaque request header view.
#[derive(Debug, Clone)]
pub struct RequestView<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub host: Option<&'a str>,
    pub client_ip: Option<&'a str>,
}

/// Opaque response header view.
#[derive(Debug, Clone)]
pub struct ResponseView<'a> {
    pub status: u16,
    pub content_type: Option<&'a str>,
}

/// Per-hook execution context.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    pub request_id: u64,
    pub conn_id: u64,
    pub monotonic_ns: u64,
}

/// Route metadata attached to a hook invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteMeta {
    pub route_name: Option<&'static str>,
    pub listener: Option<&'static str>,
    pub profile: RuntimeProfile,
}

impl Default for RouteMeta {
    fn default() -> Self {
        Self {
            route_name: None,
            listener: None,
            profile: RuntimeProfile::Edge,
        }
    }
}

/// Telemetry stats emitted at request completion.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestCompleteStats {
    pub status: u16,
    pub duration_ns: u64,
}

/// Opaque addon configuration slice (core-owned in v1).
#[derive(Debug, Clone, Default)]
pub struct AddonConfig {}

/// Static manifest for a compiled-in addon.
#[derive(Debug, Clone)]
pub struct AddonDescriptor {
    pub name: &'static str,
    pub addon_version: &'static str,
    pub addon_api: ApiVersion,
    pub core_compat: VersionReq,
    pub capabilities: &'static [Capability],
    pub cost_class: CostClass,
    pub build: BuildKind,
}

/// Errors during registration or handshake.
#[derive(Debug, Error)]
pub enum AddonError {
    #[error("addon API major mismatch: addon {addon} requires {required}, host supports {host}")]
    ApiMajorMismatch {
        addon: &'static str,
        required: ApiVersion,
        host: ApiVersion,
    },
    #[error("core version {core} does not satisfy addon {addon} requirement {requirement}")]
    CoreCompatMismatch {
        addon: &'static str,
        core: Version,
        requirement: VersionReq,
    },
    #[error("addon {addon} cost class {cost_class:?} denied for runtime profile {profile:?}")]
    CostClassDenied {
        addon: &'static str,
        cost_class: CostClass,
        profile: RuntimeProfile,
    },
    #[error("addon {addon} self_test failed: {reason}")]
    SelfTestFailed { addon: &'static str, reason: String },
    #[error("duplicate addon name: {name}")]
    DuplicateName { name: &'static str },
    #[error("required capability {capability:?} missing from addon {addon}")]
    MissingCapability {
        addon: &'static str,
        capability: Capability,
    },
}

/// Object-safe addon trait with default no-op hooks.
pub trait Addon: Send + Sync {
    fn descriptor(&self) -> &'static AddonDescriptor;

    fn on_init(&self, _cfg: &AddonConfig) -> Result<(), AddonError> {
        Ok(())
    }

    fn on_connection(&self, _: &ConnectionView, _: &mut HookContext) -> Decision {
        Decision::Continue
    }

    fn on_request_headers(&self, _: &RequestView<'_>, _: &mut HookContext) -> Decision {
        Decision::Continue
    }

    fn on_request_body(&self, _: &[u8], _: &mut HookContext) -> Decision {
        Decision::Continue
    }

    fn on_upstream(&self, _: &RequestView<'_>, _: &mut HookContext) -> Result<(), AddonError> {
        Ok(())
    }

    fn on_response_headers(&self, _: &ResponseView<'_>, _: &mut HookContext) -> Decision {
        Decision::Continue
    }

    fn on_response_body_chunk(&self, _: &[u8], _: &mut HookContext) -> Decision {
        Decision::Continue
    }

    fn on_request_complete(&self, _: &HookContext, _: RequestCompleteStats) {}

    fn self_test(&self) -> Result<(), AddonError> {
        Ok(())
    }

    fn on_shutdown(&self) -> Result<(), AddonError> {
        Ok(())
    }
}

/// Static addon registry with startup handshake validation.
#[derive(Default)]
pub struct AddonRegistry {
    addons: Vec<Arc<dyn Addon>>,
}

impl AddonRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, addon: Arc<dyn Addon>) -> Result<&mut Self, AddonError> {
        let name = addon.descriptor().name;
        if self.addons.iter().any(|a| a.descriptor().name == name) {
            return Err(AddonError::DuplicateName { name });
        }
        self.addons.push(addon);
        Ok(self)
    }

    pub fn modules(&self) -> &[Arc<dyn Addon>] {
        &self.addons
    }

    pub fn len(&self) -> usize {
        self.addons.len()
    }

    pub fn is_empty(&self) -> bool {
        self.addons.is_empty()
    }

    /// Validates API major, core compat, cost class, and runs `self_test` for each addon.
    pub fn validate_handshake(
        &self,
        core_version: &Version,
        profile: RuntimeProfile,
    ) -> Result<(), AddonError> {
        let host_api = ApiVersion::V1_0;
        for addon in &self.addons {
            let desc = addon.descriptor();
            if !desc.addon_api.matches_host(host_api) {
                return Err(AddonError::ApiMajorMismatch {
                    addon: desc.name,
                    required: desc.addon_api,
                    host: host_api,
                });
            }
            if !desc.core_compat.matches(core_version) {
                return Err(AddonError::CoreCompatMismatch {
                    addon: desc.name,
                    core: core_version.clone(),
                    requirement: desc.core_compat.clone(),
                });
            }
            if !profile.allows_cost_class(desc.cost_class) {
                return Err(AddonError::CostClassDenied {
                    addon: desc.name,
                    cost_class: desc.cost_class,
                    profile,
                });
            }
            addon.self_test().map_err(|err| match err {
                AddonError::SelfTestFailed { .. } => err,
                other => AddonError::SelfTestFailed {
                    addon: desc.name,
                    reason: other.to_string(),
                },
            })?;
        }
        Ok(())
    }

    /// Ensures every required capability is declared by at least one registered addon.
    pub fn ensure_capabilities(&self, required: &[Capability]) -> Result<(), AddonError> {
        for cap in required {
            let satisfied = self
                .addons
                .iter()
                .any(|addon| addon.descriptor().capabilities.contains(cap));
            if !satisfied {
                return Err(AddonError::MissingCapability {
                    addon: "<registry>",
                    capability: *cap,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::VersionReq;

    struct TestAddon {
        desc: &'static AddonDescriptor,
    }

    impl Addon for TestAddon {
        fn descriptor(&self) -> &'static AddonDescriptor {
            self.desc
        }
    }

    fn leak_descriptor(name: &'static str, cost_class: CostClass) -> &'static AddonDescriptor {
        Box::leak(Box::new(AddonDescriptor {
            name,
            addon_version: "0.1.0",
            addon_api: ApiVersion::V1_0,
            core_compat: VersionReq::parse(">=0.1.0, <0.5.0").unwrap(),
            capabilities: &[Capability::Telemetry],
            cost_class,
            build: BuildKind::Static,
        }))
    }

    #[test]
    fn handshake_accepts_compatible_addon() {
        let mut registry = AddonRegistry::new();
        registry
            .register(Arc::new(TestAddon {
                desc: leak_descriptor("test", CostClass::LowCostHeaderOnly),
            }))
            .unwrap();
        registry
            .validate_handshake(&Version::new(0, 1, 0), RuntimeProfile::Edge)
            .unwrap();
    }

    #[test]
    fn handshake_rejects_streaming_on_edge() {
        let mut registry = AddonRegistry::new();
        registry
            .register(Arc::new(TestAddon {
                desc: leak_descriptor("stream", CostClass::StreamingCost),
            }))
            .unwrap();
        let err = registry
            .validate_handshake(&Version::new(0, 1, 0), RuntimeProfile::Edge)
            .unwrap_err();
        assert!(matches!(err, AddonError::CostClassDenied { .. }));
    }
}
