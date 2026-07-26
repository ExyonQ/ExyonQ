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
//! ExyonQ Addon SDK — thin helpers over [`exyonq-addon-api`].
//!
//! Official addons should depend on this crate for descriptor builders and re-exports.

pub use exyonq_addon_api::*;

use semver::VersionReq;

/// Build a static addon descriptor for compile-time registration.
pub fn static_descriptor(
    name: &'static str,
    addon_version: &'static str,
    core_compat: &str,
    capabilities: &'static [exyonq_addon_api::Capability],
    cost_class: exyonq_addon_api::CostClass,
) -> exyonq_addon_api::AddonDescriptor {
    exyonq_addon_api::AddonDescriptor {
        name,
        addon_version,
        addon_api: exyonq_addon_api::ApiVersion::V1_0,
        core_compat: VersionReq::parse(core_compat)
            .unwrap_or_else(|err| panic!("invalid core_compat for {name}: {err}")),
        capabilities,
        cost_class,
        build: exyonq_addon_api::BuildKind::Static,
    }
}

/// Convenience for official modules targeting the v0.3.x core line.
pub fn official_core_compat() -> &'static str {
    ">=0.1.0, <0.4.0"
}
