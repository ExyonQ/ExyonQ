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
//! KD2.2 — StaticDispatchService contract tests (KD2D — parallel-safe registry).

use exyonq_core::{
    bind_static_compiled_slots, build_static_dispatch_request_from_str,
    clear_global_static_dispatch_for_register_once_test, contract_service_registration_test_gate,
    execute_backend, register_static_dispatch_service, Backend, StaticDispatchTestGuard,
    StaticMethod,
};
use exyonq_mod_static::StaticRuntime;
use exyonq_module_api::static_dispatch::{
    StaticCompiledSlot, StaticDispatchService, StaticRegisterError,
};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

fn register_runtime_with_root() -> (
    tempfile::TempDir,
    Arc<StaticRuntime>,
    StaticDispatchTestGuard,
) {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("page.html"), b"inline-body").unwrap();
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
            route_host: None,
            preload_max_file_bytes: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_FILE_BYTES,
            preload_max_total_bytes: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_TOTAL_BYTES,
            preload_max_entries: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_ENTRIES,
        }],
    );
    (dir, runtime, guard)
}

#[tokio::test]
async fn unregistered_static_returns_501() {
    let _guard = StaticDispatchTestGuard::force_absent();
    let outcome = execute_backend(
        &Backend::Static { root_slot: 0 },
        None,
        Some(build_static_dispatch_request_from_str(
            0,
            StaticMethod::Get,
            "/assets/page.html",
            Vec::new(),
        )),
        None,
    )
    .await
    .expect("static");
    assert_eq!(outcome.status, 501);
}

#[tokio::test]
async fn registered_inline_body() {
    let (_dir, _runtime, _guard) = register_runtime_with_root();
    let outcome = execute_backend(
        &Backend::Static { root_slot: 0 },
        None,
        Some(build_static_dispatch_request_from_str(
            0,
            StaticMethod::Get,
            "/assets/page.html",
            Vec::new(),
        )),
        None,
    )
    .await
    .expect("static");
    assert_eq!(outcome.status, 200);
    assert_eq!(outcome.body, b"inline-body".as_slice());
}

#[tokio::test]
async fn registered_head_empty_body() {
    let (_dir, _runtime, _guard) = register_runtime_with_root();
    let outcome = execute_backend(
        &Backend::Static { root_slot: 0 },
        None,
        Some(build_static_dispatch_request_from_str(
            0,
            StaticMethod::Head,
            "/assets/page.html",
            Vec::new(),
        )),
        None,
    )
    .await
    .expect("static");
    assert_eq!(outcome.status, 200);
    assert!(outcome.body.is_empty());
}

#[tokio::test]
async fn not_found_fallback() {
    let (_dir, _runtime, _guard) = register_runtime_with_root();
    let outcome = execute_backend(
        &Backend::Static { root_slot: 0 },
        None,
        Some(build_static_dispatch_request_from_str(
            0,
            StaticMethod::Get,
            "/assets/missing.html",
            Vec::new(),
        )),
        None,
    )
    .await
    .expect("static");
    assert_eq!(outcome.status, 404);
}

#[tokio::test]
async fn path_traversal_returns_403() {
    let (_dir, _runtime, _guard) = register_runtime_with_root();
    let outcome = execute_backend(
        &Backend::Static { root_slot: 0 },
        None,
        Some(build_static_dispatch_request_from_str(
            0,
            StaticMethod::Get,
            "/assets/../page.html",
            Vec::new(),
        )),
        None,
    )
    .await
    .expect("static");
    assert_eq!(outcome.status, 403);
}

#[tokio::test]
async fn duplicate_registration_rejected() {
    let _gate = contract_service_registration_test_gate();
    clear_global_static_dispatch_for_register_once_test();
    register_static_dispatch_service(Arc::new(StaticRuntime::new())).expect("first");
    assert_eq!(
        register_static_dispatch_service(Arc::new(StaticRuntime::new())),
        Err(StaticRegisterError::AlreadyRegistered)
    );
}

#[tokio::test]
async fn reload_generation_invalidates_stale_sendfile_handles() {
    let (_dir, runtime, _guard) = register_runtime_with_root();
    let handle = runtime.sendfile_handle_registry().issue(
        1,
        Arc::new(
            exyonq_mod_static::sendfile::SendfileAsset::open(std::path::Path::new("/etc/hosts"), 0)
                .expect("open hosts for stale-handle test"),
        ),
    );
    bind_static_compiled_slots(2, &[]);
    assert!(runtime.take_sendfile_handle(handle).is_none());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn sendfile_handle_taken_exactly_once() {
    // Cap067: Hyper dispatch no longer returns SendfileHandle (try_sendfile_outcome = None).
    // Take-once semantics are owned by the handle registry used by the epoll FSM.
    let (_dir, runtime, _guard) = register_runtime_with_root();
    let dir = tempdir().unwrap();
    let path = dir.path().join("64k.bin");
    fs::write(&path, vec![0u8; 65536]).unwrap();
    let asset =
        Arc::new(exyonq_mod_static::sendfile::SendfileAsset::open(&path, 65536).expect("open 64k"));
    let handle = runtime
        .sendfile_handle_registry()
        .issue(runtime.generation(), asset);
    assert!(runtime.take_sendfile_handle(handle).is_some());
    assert!(runtime.take_sendfile_handle(handle).is_none());
    runtime.release_sendfile_handle(handle);
}
