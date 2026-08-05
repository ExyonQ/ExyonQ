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
//! Plan 08 PR5-B1 — integration smoke (no test-harness APIs in production path).

use exyonq_core::{execute_backend, Backend};
use exyonq_metrics::{fcgi_responses_501_total, KernelShellMetrics};
use exyonq_module_api::kernel_observation::KernelObservationTestGuard;
use std::sync::Arc;

#[tokio::test]
async fn pr5_b1_no_executor_returns_501_stub_metric() {
    let _obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    let before = fcgi_responses_501_total();
    let outcome = execute_backend(&Backend::Fastcgi { pool_id: 0 }, None, None, None)
        .await
        .expect("Fastcgi path");
    assert_eq!(outcome.status, 501);
    assert_ne!(outcome.status, 503, "PR5-B1 must not emit live 503");
    assert_eq!(fcgi_responses_501_total(), before + 1);
}

#[test]
fn pr5_b1_handler_and_wire_dispatch_untouched_wiring() {
    let handler = include_str!("../src/server/handler.rs");
    assert!(
        !handler.contains("set_fcgi_executor_for_tests"),
        "handler must not register PR5 test executor"
    );
    let wire_dispatch = include_str!("../src/server/wire_dispatch.rs");
    assert!(
        !wire_dispatch.contains("FcgiBackendExecutor"),
        "wire_dispatch must remain unwired for PR5-B1"
    );
}

#[test]
fn pr5_b1_runtime_uses_spawn_blocking_not_block_in_place() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/exyonq-mod-fastcgi/src/runtime.rs");
    let source = std::fs::read_to_string(path).expect("runtime.rs");
    assert!(
        source.contains("spawn_blocking"),
        "FcgiRuntime must offload via spawn_blocking"
    );
    assert!(
        !source.contains("block_in_place"),
        "FcgiRuntime must not use block_in_place for live FastCGI"
    );
}
