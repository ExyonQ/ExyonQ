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
//! RuntimePlan compiler: config IR → immutable lookup-oriented plan.

pub mod backend;
pub mod config_load;
pub mod router;
pub mod routing;
pub mod runtime_plan;

pub use backend::{Backend, BackendId, BackendTable};
pub use config_load::{
    apply_discovery, load_config_for_reload, DiscoveryCluster, DiscoveryOverlay, Profile,
};
pub use router::RouteIndex;
pub use routing::RouteDecision;
pub use runtime_plan::{
    compile_fcgi_pool_slots, compile_full_page_cache, compile_htaccess_site_bindings,
    compile_runtime_plan, compile_runtime_plan_from_ir, stable_fpc_site_id, CompiledFcgiPool,
    CompiledFullPageCache, HtaccessSiteBinding, RuntimePlan, RuntimeSnapshot, SnapshotFingerprint,
};
