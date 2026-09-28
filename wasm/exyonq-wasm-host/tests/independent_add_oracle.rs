//! Independent arithmetic oracle for the Wasmtime host.
//!
//! The expected `i32` is `40 + 2` written in this test. It is not produced by
//! calling the host twice, and it is not read from a constant the host owns.
//! This does not prove a request path: the host is not wired into `exyonq` serve.

use exyonq_wasm_host::{invoke_i32_with_fuel, WasmHostError};

const LEFT: i32 = 40;
const RIGHT: i32 = 2;

#[test]
fn exported_add_matches_arithmetic_spec() {
    let wat = format!(
        r#"(module
            (func (export "answer") (result i32)
                i32.const {LEFT}
                i32.const {RIGHT}
                i32.add))"#
    );
    let got = invoke_i32_with_fuel(&wat, "answer", 10_000).expect("invoke");
    assert_eq!(got, LEFT.wrapping_add(RIGHT));
}

#[test]
fn fuel_budget_stops_a_nonterminating_loop() {
    let wat = r#"(module
        (func (export "spin") (result i32)
            (loop $forever
                br $forever)
            i32.const 1))"#;
    let err = invoke_i32_with_fuel(wat, "spin", 50).expect_err("spin must trap");
    assert!(matches!(err, WasmHostError::FuelExhausted));
}
