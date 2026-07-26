//! Micro-benchmark: Wasmtime host invoke latency (baseline propia, not P10).

use std::hint::black_box;

use exyonq_wasm_host::invoke_i32_with_fuel;

const NOOP_WAT: &str = r#"
    (module
      (func (export "run") (result i32)
        i32.const 42)
    )
"#;

fn bench_invoke_noop(c: &mut criterion::Criterion) {
    c.bench_function("invoke_noop_i32", |b| {
        b.iter(|| {
            let v = invoke_i32_with_fuel(NOOP_WAT, "run", 1_000_000).expect("invoke");
            black_box(v);
        });
    });
}

criterion::criterion_group!(benches, bench_invoke_noop);
criterion::criterion_main!(benches);
