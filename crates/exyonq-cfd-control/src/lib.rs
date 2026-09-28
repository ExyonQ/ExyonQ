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

//! Control-plane helpers for Competitive Frontier dataplane foundations.
//!
//! Tokio-free (std only). Invoked from the product control process.

mod budget;
mod launch;
mod project;
mod waf;

pub use budget::{resolve_competitive_h1_shards, ExyonqParallelismBudget, MAX_SHARDS, MIN_SHARDS};
pub use launch::{
    assert_no_listen_collision, dataplane_enabled, resolve_dataplane_bin, wait_status, CfdChild,
    CfdLaunchConfig, CfdLaunchError, DataplaneStatus, ENV_BIN, ENV_ENABLED, ENV_GEN_DIR,
    ENV_LISTEN, ENV_SCHEMA, ENV_SHARDS, READY_TIMEOUT_DEFAULT,
};
pub use project::{
    load_routes_from_env, parse_routes_file, parse_routes_text, table_from_entries, ProjectError,
    ENV_ROUTES,
};
pub use waf::{
    compile_input_from_projection, projection_from_compile_input, publish_routes_and_waf,
    representative_phase3_waf_input, WafProjectError,
};

pub use exyonq_cfd_gen::{
    BackendKind, BackendTarget, CompiledFcgiPool, CompiledRoute, CompiledStaticPolicy,
    CompiledUpstream, CompositeProjection, FcgiTransport, GenDir, GenError, Generation, RouteTable,
    RouteTableError, SCHEMA_VERSION,
};
