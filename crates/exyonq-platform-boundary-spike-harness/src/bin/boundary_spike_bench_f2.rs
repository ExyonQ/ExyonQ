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
//! PS0B F2 opaque OnceLock fn-pointer full-path bench (isolated binary).

use exyonq_platform_boundary_spike_harness::{
    bench_full_path_variant, mode_label, variant_label, BenchMode, BenchShape, BenchVariant,
    FULL_PATH_ITERS, FULL_PATH_RUNS, FULL_PATH_WARMUP,
};
use exyonq_platform_boundary_spike_kernel::KERNEL_OPAQUE_PLAN;

fn main() {
    let _ = KERNEL_OPAQUE_PLAN.ensure_registered();
    let variant = BenchVariant::F2;
    println!(
        "PS0B boundary-spike-bench-{} iters={} warmup={} runs={}",
        variant_label(variant),
        FULL_PATH_ITERS,
        FULL_PATH_WARMUP,
        FULL_PATH_RUNS
    );
    for mode in [
        BenchMode::PlannerOnly,
        BenchMode::WirePlanConstruct,
        BenchMode::FullConsume,
    ] {
        for shape in [
            BenchShape::P1Static,
            BenchShape::P1Proxy,
            BenchShape::HyperFallback,
        ] {
            let stats = bench_full_path_variant(variant, mode, shape);
            stats.print(&format!(
                "{}_{}_{}",
                variant_label(variant),
                mode_label(mode),
                shape.label()
            ));
        }
    }
}
