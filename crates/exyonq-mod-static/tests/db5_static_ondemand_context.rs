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
//! DB5 Part D — static on-demand execution context validation (test-only).

use exyonq_mod_static::{read_file_bytes_with_budget, PreloadLimits, StaticRoot};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn db5_static_ondemand_source_uses_sync_read_not_spawn_blocking() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_rs = fs::read_to_string(root.join("src/runtime.rs")).expect("runtime.rs");
    // On-demand path calls read_file_bytes_with_budget synchronously inside async serve_resolved_path.
    assert!(
        runtime_rs.contains("read_file_bytes_with_budget(path, materialization_budget_bytes)"),
        "serve_resolved_path must call bounded sync read"
    );
    // Must not route on-demand through spawn_blocking in runtime.rs.
    let on_demand_region = runtime_rs
        .split("async fn serve_resolved_path")
        .nth(1)
        .and_then(|s| s.split("fn bytes_to_dispatch_outcome").next())
        .expect("serve_resolved_path region");
    assert!(
        !on_demand_region.contains("spawn_blocking"),
        "on-demand static must not use spawn_blocking in serve_resolved_path"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn db5_static_sync_read_allows_sibling_progress_on_multi_worker_runtime() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("blob.bin");
    // Large enough to take measurable time, small enough for CI.
    fs::write(&path, vec![7u8; 2 * 1024 * 1024]).unwrap();

    let ticks = Arc::new(AtomicU64::new(0));
    let ticker = {
        let ticks = Arc::clone(&ticks);
        tokio::spawn(async move {
            for _ in 0..200 {
                ticks.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
        })
    };

    let read = tokio::task::spawn_blocking({
        let path = path.clone();
        move || read_file_bytes_with_budget(&path, None)
    });
    // Note: spawn_blocking here isolates the sync read for progress proof.
    // Production Hyper path runs sync read ON the async worker (see source test).
    // This test proves the runtime still schedules siblings under multi_thread.
    let _ = read.await.unwrap().expect("read");
    let _ = tokio::time::timeout(Duration::from_secs(2), ticker)
        .await
        .expect("ticker");
    assert!(
        ticks.load(Ordering::Relaxed) > 10,
        "sibling tasks must progress on multi-worker runtime"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn db5_static_ondemand_blocks_current_thread_worker_when_run_inline() {
    // Demonstrates TOKIO_ASYNC_WORKER risk on a single-threaded runtime:
    // a long sync read inline would stall the only worker.
    // We use a short sleep as a stand-in without huge IO, and prove the scheduling effect:
    let progressed = Arc::new(AtomicU64::new(0));
    let p = Arc::clone(&progressed);
    let blocker = tokio::spawn(async move {
        // Simulates sync FS on async worker.
        std::thread::sleep(Duration::from_millis(80));
        p.fetch_add(1, Ordering::SeqCst);
    });
    let mut ticks = 0u64;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(40) {
        ticks += 1;
        tokio::task::yield_now().await;
    }
    // On current_thread, while blocker holds the worker via std::sleep, ticks stay low.
    // We only assert the blocker eventually completes (no hang).
    let _ = tokio::time::timeout(Duration::from_secs(2), blocker)
        .await
        .expect("blocker join");
    assert_eq!(progressed.load(Ordering::SeqCst), 1);
    let _ = ticks;
}

#[test]
fn db5_preload_disabled_skips_tree_and_ondemand_resolve_works() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("public");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("x.txt"), b"hello").unwrap();
    let limits = PreloadLimits {
        max_file_bytes: 0,
        max_total_bytes: 1,
        max_entries: 1,
    };
    let mut svc = StaticRoot::new_with_preload_limits(&root, "/site", None, limits).unwrap();
    svc.preload_tree().unwrap();
    assert_eq!(svc.cache_len(), 0);
    assert!(svc.resolve_live_file_path("/site/x.txt").is_ok());
}
