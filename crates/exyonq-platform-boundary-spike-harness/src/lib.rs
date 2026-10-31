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
//! PS0A/PS0B experimental harness — NOT production architecture.

mod bench_full_path;
mod full_path;
mod ps0a;

pub use bench_full_path::{
    bench_full_path_variant, expected_kind, mode_label, run_protector_loop, variant_label,
    BenchMode, BenchShape, BenchStats, BenchVariant, FULL_PATH_ITERS, FULL_PATH_RUNS,
    FULL_PATH_WARMUP,
};
pub use full_path::{
    consume_wire_plan, fixtures, plan_wire_monolith_full, read_wire_plan_f2, read_wire_plan_m2,
    read_wire_plan_x2, static_response_token, FixtureSet,
};
pub use ps0a::*;
