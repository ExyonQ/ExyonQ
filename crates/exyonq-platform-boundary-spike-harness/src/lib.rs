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
