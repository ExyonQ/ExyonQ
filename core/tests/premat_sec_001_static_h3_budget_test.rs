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
//! PREMAT-SEC-001 — H3 budget injection seam + static BudgetExceeded marker.

use exyonq_core::{
    bind_static_compiled_slots, build_static_dispatch_request_from_str,
    build_static_dispatch_request_from_str_with_budget, execute_backend, Backend,
    StaticDispatchTestGuard, StaticMethod,
};
use exyonq_mod_static::StaticRuntime;
use exyonq_module_api::static_dispatch::{
    StaticCompiledSlot, StaticDispatchService, MATERIALIZATION_BUDGET_EXCEEDED_HEADER,
};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

fn register_runtime_with_file(bytes: &[u8]) -> (tempfile::TempDir, StaticDispatchTestGuard) {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("big.bin"), bytes).unwrap();
    let runtime = Arc::new(StaticRuntime::new());
    let service: Arc<dyn StaticDispatchService> = runtime.clone();
    let guard = StaticDispatchTestGuard::install(service);
    exyonq_mod_static::install_kernel_hooks(Arc::clone(&runtime));
    bind_static_compiled_slots(
        1,
        &[StaticCompiledSlot {
            filesystem_root: root,
            route_prefix: "/assets".into(),
            index_file: None,
            route_name: "assets".into(),
            preload_max_file_bytes: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_FILE_BYTES,
            preload_max_total_bytes: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_TOTAL_BYTES,
            preload_max_entries: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_ENTRIES,
        }],
    );
    (dir, guard)
}

#[test]
fn h1_builder_defaults_budget_none() {
    let req = build_static_dispatch_request_from_str(0, StaticMethod::Get, "/x", Vec::new());
    assert_eq!(req.materialization_budget_bytes, None);
}

#[tokio::test]
async fn budget_none_serves_body_over_tiny_limit() {
    let (_dir, _guard) = register_runtime_with_file(&[0u8; 128]);
    let outcome = execute_backend(
        &Backend::Static { root_slot: 0 },
        None,
        Some(build_static_dispatch_request_from_str(
            0,
            StaticMethod::Get,
            "/assets/big.bin",
            Vec::new(),
        )),
        None,
    )
    .await
    .expect("static");
    assert_eq!(outcome.status, 200);
    assert_eq!(outcome.body.len(), 128);
}

#[tokio::test]
async fn budget_some_rejects_oversize_with_private_marker() {
    let (_dir, _guard) = register_runtime_with_file(&[0u8; 128]);
    let outcome = execute_backend(
        &Backend::Static { root_slot: 0 },
        None,
        Some(build_static_dispatch_request_from_str_with_budget(
            0,
            StaticMethod::Get,
            "/assets/big.bin",
            Vec::new(),
            Some(64),
        )),
        None,
    )
    .await
    .expect("static");
    assert!(
        outcome.headers.iter().any(|(n, v)| {
            n.eq_ignore_ascii_case(MATERIALIZATION_BUDGET_EXCEEDED_HEADER) && v == "1"
        }),
        "expected budget-exceeded marker, got {outcome:?}"
    );
    assert!(outcome.body.is_empty());
}

#[tokio::test]
async fn budget_some_allows_exact_limit() {
    let (_dir, _guard) = register_runtime_with_file(&[1u8; 64]);
    let outcome = execute_backend(
        &Backend::Static { root_slot: 0 },
        None,
        Some(build_static_dispatch_request_from_str_with_budget(
            0,
            StaticMethod::Get,
            "/assets/big.bin",
            Vec::new(),
            Some(64),
        )),
        None,
    )
    .await
    .expect("static");
    assert_eq!(outcome.status, 200);
    assert_eq!(outcome.body.len(), 64);
}
