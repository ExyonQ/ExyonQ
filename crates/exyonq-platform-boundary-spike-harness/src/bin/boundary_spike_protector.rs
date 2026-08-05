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
