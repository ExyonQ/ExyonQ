//! Micro-benchmark: Wasmtime host invoke latency (baseline propia, not P10).

use std::hint::black_box;

use exyonq_wasm_host::invoke_i32_with_fuel;

const CONST_ANSWER_WAT: &str = r#"
    (module
      (func (export "run") (result i32)
        i32.const 42)
    )
"#;

fn bench_invoke_const_answer(c: &mut criterion::Criterion) {
    c.bench_function("invoke_const_answer_i32", |b| {
        b.iter(|| {
            let v = invoke_i32_with_fuel(CONST_ANSWER_WAT, "run", 1_000_000).expect("invoke");
            black_box(v);
        });
    });
}

criterion::criterion_group!(benches, bench_invoke_const_answer);
criterion::criterion_main!(benches);
