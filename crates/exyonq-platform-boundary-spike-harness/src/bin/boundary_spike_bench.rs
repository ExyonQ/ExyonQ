//! PS0A micro-benchmark: monolith (M) vs cross-static (X) vs fn-pointer (F).

use std::hint::black_box;
use std::time::Instant;

use exyonq_platform_boundary_spike_harness::{
    read_wire_plan_cross_fnptr, read_wire_plan_cross_static, read_wire_plan_monolith, BENCH_STATE,
    HEAD_HYPER_FALLBACK, HEAD_P1_PROXY, HEAD_P1_STATIC, REST_EMPTY,
};
use exyonq_platform_boundary_spike_kernel::plan_fn_ptr;

const ITERS: u32 = 5_000_000;
const WARMUP: u32 = 200_000;

fn bench_label(name: &str, iters: u32, mut f: impl FnMut()) {
    for _ in 0..WARMUP {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let elapsed = start.elapsed();
    let ns = elapsed.as_nanos() as f64 / f64::from(iters);
    println!("{name}: {ns:.2} ns/op ({iters} iters)");
}

fn main() {
    let state = &BENCH_STATE;
    let plan_ptr = black_box(plan_fn_ptr());

    println!("PS0A boundary-spike-bench (release profile of workspace)");
    println!("iters={ITERS} warmup={WARMUP}");

    bench_label("M_monolith_P1_static", ITERS, || {
        let r = read_wire_plan_monolith(HEAD_P1_STATIC, REST_EMPTY, state);
        black_box(r);
    });

    bench_label("X_cross_static_P1_static", ITERS, || {
        let r = read_wire_plan_cross_static(HEAD_P1_STATIC, REST_EMPTY, state);
        black_box(r);
    });

    bench_label("F_cross_fnptr_P1_static", ITERS, || {
        let r = read_wire_plan_cross_fnptr(HEAD_P1_STATIC, REST_EMPTY, state, plan_ptr);
        black_box(r);
    });

    bench_label("M_monolith_P1_proxy", ITERS, || {
        let r = read_wire_plan_monolith(HEAD_P1_PROXY, REST_EMPTY, state);
        black_box(r);
    });

    bench_label("X_cross_static_P1_proxy", ITERS, || {
        let r = read_wire_plan_cross_static(HEAD_P1_PROXY, REST_EMPTY, state);
        black_box(r);
    });

    bench_label("F_cross_fnptr_P1_proxy", ITERS, || {
        let r = read_wire_plan_cross_fnptr(HEAD_P1_PROXY, REST_EMPTY, state, plan_ptr);
        black_box(r);
    });

    bench_label("M_monolith_hyper_fallback", ITERS, || {
        let r = read_wire_plan_monolith(HEAD_HYPER_FALLBACK, REST_EMPTY, state);
        black_box(r);
    });

    bench_label("X_cross_static_hyper_fallback", ITERS, || {
        let r = read_wire_plan_cross_static(HEAD_HYPER_FALLBACK, REST_EMPTY, state);
        black_box(r);
    });

    bench_label("F_cross_fnptr_hyper_fallback", ITERS, || {
        let r = read_wire_plan_cross_fnptr(HEAD_HYPER_FALLBACK, REST_EMPTY, state, plan_ptr);
        black_box(r);
    });
}
