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
//! Security integration tests for the Wasmtime host (ADR-009).

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use exyonq_wasm_host::{
    engine_for_tests, invoke_i32_with_fuel, invoke_with_epoch_deadline, WasmHostError,
};

const INFINITE_LOOP_WAT: &str = r#"
    (module
      (func (export "run") (result i32)
        (local $i i32)
        (local.set $i (i32.const 0))
        (loop $l
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br $l))
        (local.get $i))
    )
"#;

const OOB_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "run") (result i32)
        i32.const 0
        i32.load
        i32.const 0)
    )
"#;

const BUSY_WAT: &str = r#"
    (module
      (func (export "run")
        (local $i i32)
        (local.set $i (i32.const 0))
        (loop $l
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br $l)))
    )
"#;

#[test]
fn wasm_host_fuel_exhaustion_returns_error() {
    let err = invoke_i32_with_fuel(INFINITE_LOOP_WAT, "run", 5_000)
        .expect_err("infinite loop must hit fuel limit");
    assert!(matches!(err, WasmHostError::FuelExhausted));
}

#[test]
fn wasm_host_memory_trap_isolated() {
    let err = invoke_i32_with_fuel(OOB_WAT, "run", 1_000_000).expect_err("OOB must trap");
    assert!(
        matches!(err, WasmHostError::Engine(_)),
        "expected engine trap, got {err:?}"
    );
}

#[test]
fn wasm_host_epoch_timeout() {
    // KF-P16-011: fixed 20ms sleep before a single epoch tick is HOST_LOAD_SENSITIVE.
    // Tick the engine epoch until the busy invoke returns Timeout (condition-based).
    use std::sync::atomic::{AtomicBool, Ordering};

    let engine = engine_for_tests().expect("engine");
    let engine_ref = Arc::new(engine);
    let stop = Arc::new(AtomicBool::new(false));
    let engine_bg = Arc::clone(&engine_ref);
    let stop_bg = Arc::clone(&stop);
    let bump = thread::spawn(move || {
        // Allow store construction at epoch baseline, then keep ticking.
        thread::yield_now();
        while !stop_bg.load(Ordering::SeqCst) {
            engine_bg.increment_epoch();
            thread::sleep(Duration::from_millis(1));
        }
    });

    let err = invoke_with_epoch_deadline(&engine_ref, BUSY_WAT, "run", u64::MAX / 4, 1)
        .expect_err("busy loop must hit epoch deadline");
    stop.store(true, Ordering::SeqCst);
    assert!(
        matches!(err, WasmHostError::Timeout(_)),
        "expected Timeout (epoch), got {err:?}"
    );
    bump.join().expect("epoch thread");
}
