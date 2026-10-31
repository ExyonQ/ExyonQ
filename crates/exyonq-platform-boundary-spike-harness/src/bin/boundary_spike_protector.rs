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
//! PS0B Level-2 representative protector — full-path loop (isolated binary).

use std::env;

use exyonq_platform_boundary_spike_harness::{
    run_protector_loop, variant_label, BenchVariant, FULL_PATH_RUNS,
};

fn main() {
    let variant = match env::var("PS0B_VARIANT").ok().as_deref() {
        Some("X2") => BenchVariant::X2,
        Some("F2") => BenchVariant::F2,
        Some("N2") => BenchVariant::N2,
        _ => BenchVariant::M2,
    };
    if matches!(variant, BenchVariant::F2) {
        let _ = exyonq_platform_boundary_spike_kernel::KERNEL_OPAQUE_PLAN.ensure_registered();
    }

    let iters: u32 = env::var("PS0B_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500_000);

    println!(
        "PS0B boundary-spike-protector variant={} iters={} runs={}",
        variant_label(variant),
        iters,
        FULL_PATH_RUNS
    );

    let mut samples = Vec::with_capacity(FULL_PATH_RUNS);
    for run in 0..FULL_PATH_RUNS {
        let ns = run_protector_loop(variant, iters);
        samples.push(ns);
        println!("run={run} ns/op={ns:.2}");
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples[samples.len() / 2];
    println!("protector_median_ns={median:.2}");
}
