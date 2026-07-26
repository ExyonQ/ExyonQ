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
//! STATIC_PRELOAD_MEMORY_FIX — bounded deterministic tree preload.

use exyonq_mod_static::{PreloadLimits, StaticRoot};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

fn write_file(dir: &std::path::Path, name: &str, bytes: &[u8]) {
    fs::write(dir.join(name), bytes).expect("write");
}

fn preload_with(limits: PreloadLimits, root: &std::path::Path) -> StaticRoot {
    let mut service =
        StaticRoot::new_with_preload_limits(root, "/site", None, limits).expect("root");
    service.preload_tree().expect("preload");
    service
}

#[test]
fn defaults_accept_small_tree() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    write_file(&root, "a.txt", b"hello");
    write_file(&root, "b.txt", b"world");
    let service = preload_with(PreloadLimits::DEFAULT, &root);
    assert_eq!(service.cache_len(), 2);
}

#[test]
fn disabled_zero_skips_tree_walk_bodies() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    write_file(&root, "a.txt", &vec![0u8; 1024]);
    let limits = PreloadLimits {
        max_file_bytes: 0,
        max_total_bytes: PreloadLimits::DEFAULT.max_total_bytes,
        max_entries: PreloadLimits::DEFAULT.max_entries,
    };
    let service = preload_with(limits, &root);
    assert_eq!(service.cache_len(), 0);
    // On-demand resolve still works.
    let path = service
        .resolve_live_file_path("/site/a.txt")
        .expect("live");
    assert!(path.ends_with("a.txt"));
}

#[test]
fn per_file_max_accepted_exact() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    let max = 64 * 1024u64;
    write_file(&root, "exact.bin", &vec![1u8; max as usize]);
    let limits = PreloadLimits {
        max_file_bytes: max,
        max_total_bytes: max * 4,
        max_entries: 10,
    };
    let service = preload_with(limits, &root);
    assert_eq!(service.cache_len(), 1);
}

#[test]
fn per_file_max_plus_one_skipped() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    let max = 64 * 1024u64;
    write_file(&root, "over.bin", &vec![2u8; (max + 1) as usize]);
    write_file(&root, "ok.txt", b"ok");
    let limits = PreloadLimits {
        max_file_bytes: max,
        max_total_bytes: max * 4,
        max_entries: 10,
    };
    let service = preload_with(limits, &root);
    assert_eq!(service.cache_len(), 1);
    assert!(service.serve_request("/site/ok.txt").is_ok());
    // Skipped oversized is not in preload cache.
    assert!(service.serve_request("/site/over.bin").is_err());
    // But live path remains.
    assert!(service.resolve_live_file_path("/site/over.bin").is_ok());
}

#[test]
fn total_budget_stops_accepting_deterministically() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    // Lexicographic UTF-8: a.bin, b.bin, c.bin
    write_file(&root, "a.bin", &vec![3u8; 1000]);
    write_file(&root, "b.bin", &vec![4u8; 1000]);
    write_file(&root, "c.bin", &vec![5u8; 1000]);
    let limits = PreloadLimits {
        max_file_bytes: 2000,
        max_total_bytes: 2000, // accepts a+b, stops before c
        max_entries: 100,
    };
    let service = preload_with(limits, &root);
    assert_eq!(service.cache_len(), 2);
    assert!(service.serve_request("/site/a.bin").is_ok());
    assert!(service.serve_request("/site/b.bin").is_ok());
    assert!(service.serve_request("/site/c.bin").is_err());
    assert!(service.resolve_live_file_path("/site/c.bin").is_ok());
}

#[test]
fn max_entries_stops_at_limit() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    for i in 0..10 {
        write_file(&root, &format!("f{i:02}.txt"), b"x");
    }
    let limits = PreloadLimits {
        max_file_bytes: 1024,
        max_total_bytes: 1024 * 1024,
        max_entries: 3,
    };
    let service = preload_with(limits, &root);
    assert_eq!(service.cache_len(), 3);
}

#[test]
fn deterministic_selection_independent_of_creation_order() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    // Create in reverse lexical order.
    write_file(&root, "z.txt", b"z");
    write_file(&root, "m.txt", b"m");
    write_file(&root, "a.txt", b"a");
    // total=2: a (1) + m (1) = 2 exact; z skipped after budget full
    let limits = PreloadLimits {
        max_file_bytes: 16,
        max_total_bytes: 2,
        max_entries: 10,
    };
    let service = preload_with(limits, &root);
    assert_eq!(service.cache_len(), 2);
    assert!(service.serve_request("/site/a.txt").is_ok());
    assert!(service.serve_request("/site/m.txt").is_ok());
    assert!(service.serve_request("/site/z.txt").is_err());
}

#[test]
fn mmap_sized_file_within_limit_accepted() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    // ≥ 64 KiB triggers mmap path
    let size = 128 * 1024usize;
    write_file(&root, "mapped.bin", &vec![7u8; size]);
    let limits = PreloadLimits {
        max_file_bytes: size as u64,
        max_total_bytes: size as u64,
        max_entries: 4,
    };
    let service = preload_with(limits, &root);
    assert_eq!(service.cache_len(), 1);
}

#[test]
fn mmap_oversize_not_mapped() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    let size = 128 * 1024usize;
    write_file(&root, "big.bin", &vec![8u8; size]);
    let limits = PreloadLimits {
        max_file_bytes: (size as u64) - 1,
        max_total_bytes: size as u64 * 2,
        max_entries: 4,
    };
    let service = preload_with(limits, &root);
    assert_eq!(service.cache_len(), 0);
}

#[tokio::test]
async fn dispatch_serves_skipped_file_on_demand() {
    use exyonq_mod_static::StaticRuntime;
    use exyonq_module_api::static_dispatch::{
        StaticCompiledSlot, StaticDispatchRequest, StaticDispatchService, StaticMethod,
    };

    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    let over = 100 * 1024usize;
    write_file(&root, "over.bin", &vec![9u8; over]);
    write_file(&root, "ok.txt", b"ok");

    let runtime = Arc::new(StaticRuntime::new());
    runtime.bind_compiled_slots(
        1,
        &[StaticCompiledSlot {
            filesystem_root: root,
            route_prefix: "/site".into(),
            index_file: None,
            route_name: "site".into(),
            preload_max_file_bytes: 1024, // skip over.bin
            preload_max_total_bytes: 1024 * 1024,
            preload_max_entries: 64,
        }],
    );

    let skipped = runtime
        .dispatch(StaticDispatchRequest {
            root_slot: 0,
            method: StaticMethod::Get,
            request_path: Arc::from("/site/over.bin"),
            headers: Vec::new(),
            materialization_budget_bytes: None,
        })
        .await;
    assert_eq!(skipped.status, 200, "skipped preload must serve on demand");
    assert_eq!(skipped.body.inline_len(), Some(over));

    let ok = runtime
        .dispatch(StaticDispatchRequest {
            root_slot: 0,
            method: StaticMethod::Get,
            request_path: Arc::from("/site/ok.txt"),
            headers: Vec::new(),
            materialization_budget_bytes: None,
        })
        .await;
    assert_eq!(ok.status, 200);
}

#[tokio::test]
async fn reload_failure_keeps_old_snapshot() {
    use exyonq_mod_static::StaticRuntime;
    use exyonq_module_api::static_dispatch::{
        StaticCompiledSlot, StaticDispatchRequest, StaticDispatchService, StaticMethod,
    };

    let dir = tempdir().unwrap();
    let root_ok = dir.path().join("ok");
    fs::create_dir_all(&root_ok).unwrap();
    write_file(&root_ok, "a.txt", b"alive");

    let runtime = Arc::new(StaticRuntime::new());
    runtime.bind_compiled_slots(
        1,
        &[StaticCompiledSlot {
            filesystem_root: root_ok.clone(),
            route_prefix: "/site".into(),
            index_file: None,
            route_name: "site".into(),
            preload_max_file_bytes: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_FILE_BYTES,
            preload_max_total_bytes: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_TOTAL_BYTES,
            preload_max_entries: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_ENTRIES,
        }],
    );
    assert_eq!(runtime.generation(), 1);

    // Bind with non-existent root → hard failure → old retained.
    runtime.bind_compiled_slots(
        2,
        &[StaticCompiledSlot {
            filesystem_root: dir.path().join("missing-root"),
            route_prefix: "/site".into(),
            index_file: None,
            route_name: "site".into(),
            preload_max_file_bytes: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_FILE_BYTES,
            preload_max_total_bytes: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_TOTAL_BYTES,
            preload_max_entries: StaticCompiledSlot::DEFAULT_PRELOAD_MAX_ENTRIES,
        }],
    );
    assert_eq!(runtime.generation(), 1, "old snapshot retained");

    let outcome = runtime
        .dispatch(StaticDispatchRequest {
            root_slot: 0,
            method: StaticMethod::Get,
            request_path: Arc::from("/site/a.txt"),
            headers: Vec::new(),
            materialization_budget_bytes: None,
        })
        .await;
    assert_eq!(outcome.status, 200);
}
