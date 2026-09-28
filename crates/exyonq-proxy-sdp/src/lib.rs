/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! SDP-P1 specialized proxy dataplane (mio TPC). Internal — not a stable public API.
//!
//! Geometry: ADR-021 C / SD-B B. Does not paste EP-C prototype sources.

#![allow(dead_code)] // Phase variants / matrix reserved for SDP-P2+ state expansion
#![forbid(unsafe_code)]

mod eligibility;
mod obs;
mod parse;
mod plan;
mod projection;
mod runtime;
mod shard;

pub use eligibility::{peek_sdp_eligible, SdpEligibility};
pub use obs::{DataplaneCounters, PathIdentity};
pub use plan::{ProxyExecutionPlan, WafAdmission};
pub use projection::{build_projection_from_parts, GenerationProjection, SharedProjection};
pub use runtime::{
    resolve_shard_count, sdp_p1_env_enabled, start_sdp_listen, HandoffFn, SdpListenHandle,
    SdpStartConfig,
};

/// Product path identity string for metrics / diagnostics.
pub const PATH_ID_SDP_P1: &str = "sdp-p1";
