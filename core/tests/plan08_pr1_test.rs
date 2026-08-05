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
//! Plan 08 PR1 integration tests — config compile, 501 stub, RP-3 policy.

use exyonq_core::{compile_runtime_plan, execute_backend, AppConfig, Backend};
use exyonq_metrics::{fcgi_responses_501_total, KernelShellMetrics};
use exyonq_module_api::kernel_observation::KernelObservationTestGuard;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn plan08_fixture_accepts_structural_fcgi_pool() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/architecture/fixtures/plan08/minimal-fcgi.toml");
    let config = AppConfig::from_file(&path).expect("plan08 fixture parses");
    assert_eq!(config.pools_fcgi.len(), 1);
    assert_eq!(config.routes[0].fastcgi.as_deref(), Some("php"));
}

#[test]
fn runtime_snapshot_compiles_fastcgi_route_as_contract_backend() {
    let raw = include_str!("../../scripts/architecture/fixtures/plan08/minimal-fcgi.toml");
    let config: AppConfig = raw.parse().expect("fixture");
    let snap = compile_runtime_plan(1, config).expect("compile");
    let backend = snap.resolve_backend(0).expect("php route backend");
    assert!(matches!(backend, Backend::Fastcgi { .. }));
    assert!(backend.is_contract_only());
}

#[tokio::test]
async fn execute_backend_fastcgi_returns_501_without_registered_service() {
    let _obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    let before = fcgi_responses_501_total();
    let outcome = execute_backend(&Backend::Fastcgi { pool_id: 0 }, None, None, None)
        .await
        .expect("Fastcgi contract path");
    assert_eq!(outcome.status, 501);
    assert_eq!(fcgi_responses_501_total(), before + 1);
}

#[test]
fn rp3_no_runtime_patch_fcgi_pool_surface() {
    // RP-3 policy: structural fcgi_pool changes require reload, not RuntimePatch (Plan 08 §13).
    let core_src = std::fs::read_dir(env!("CARGO_MANIFEST_DIR").to_string() + "/src")
        .expect("core/src readable");
    for entry in core_src.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            let content = std::fs::read_to_string(&path).expect("read core src");
            assert!(
                !content.contains("struct RuntimePatch"),
                "PR1 must not add RuntimePatch in {}",
                path.display()
            );
        }
    }
}

#[test]
fn guards_script_enforces_fastcgi_and_handler_bans() {
    let verify = include_str!("../../scripts/architecture/verify-phase0-kernel.sh");
    for check in [
        "check_fastcgi_runtime",
        "check_handler_table",
        "check_forbidden_hotpath",
        "check_mod_to_core",
    ] {
        assert!(verify.contains(check), "missing guard check `{check}`");
    }
}

#[test]
fn pr1_execute_backend_contract_stub_unchanged_in_core() {
    let execute_backend_src = include_str!("../src/execute_backend.rs");
    assert!(
        execute_backend_src.contains("Backend::Fastcgi"),
        "contract stub must retain Fastcgi arm"
    );
    assert!(
        !execute_backend_src.contains("connect("),
        "PR2-A must not add socket transport to execute_backend"
    );
}
