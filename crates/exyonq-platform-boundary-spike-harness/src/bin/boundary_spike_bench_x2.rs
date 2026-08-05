//! PS0B X2 cross-crate static full-path bench (isolated binary).

use exyonq_platform_boundary_spike_harness::{
    bench_full_path_variant, mode_label, variant_label, BenchMode, BenchShape, BenchVariant,
    FULL_PATH_ITERS, FULL_PATH_RUNS, FULL_PATH_WARMUP,
};

fn main() {
    let variant = BenchVariant::X2;
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
