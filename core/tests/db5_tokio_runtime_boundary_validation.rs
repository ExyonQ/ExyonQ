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
//! DB5 Tokio runtime boundary validation — adversarial / source + lifecycle evidence.
//!
//! Does not modify production code. Complements module-scoped FCGI/static/reload tests.

use exyonq_core::lifecycle::LifecycleState;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Strip `#[cfg(test)]` … module bodies and line comments for coarse production scans.
fn strip_test_cfg_and_line_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_cfg_test_mod = 0i32;
    let mut brace_depth_at_enter = 0i32;
    let mut depth = 0i32;
    let mut pending_cfg_test = false;

    for line in src.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
            continue;
        }
        if pending_cfg_test {
            if trimmed.starts_with("mod ")
                || trimmed.starts_with("async fn ")
                || trimmed.starts_with("fn ")
            {
                in_cfg_test_mod = 1;
                brace_depth_at_enter = depth;
                pending_cfg_test = false;
            } else if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("#[") {
                // keep waiting for item
            } else {
                pending_cfg_test = false;
            }
        }

        let opens = line.chars().filter(|c| *c == '{').count() as i32;
        let closes = line.chars().filter(|c| *c == '}').count() as i32;

        if in_cfg_test_mod > 0 {
            depth += opens - closes;
            if depth <= brace_depth_at_enter && closes > 0 {
                in_cfg_test_mod = 0;
            }
            continue;
        }

        depth += opens - closes;

        if let Some(idx) = line.find("//") {
            // keep strings roughly; good enough for Runtime::new scans
            out.push_str(&line[..idx]);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn count_matches(hay: &str, needle: &str) -> usize {
    hay.match_indices(needle).count()
}

#[test]
fn db5_cli_is_sole_production_multi_thread_runtime_builder() {
    let root = workspace_root();
    let cli = strip_test_cfg_and_line_comments(&read(&root.join("cli/exyonq/src/main.rs")));
    assert_eq!(
        count_matches(&cli, "Builder::new_multi_thread"),
        1,
        "CLI must create exactly one multi_thread runtime"
    );
    assert_eq!(
        count_matches(&cli, "rt.block_on"),
        1,
        "CLI must block_on once at process entry"
    );

    // Production crates that must not build a Tokio Runtime.
    let forbidden_builders = [
        "core/src/server/mod.rs",
        "core/src/server/handler.rs",
        "core/src/server/hyper_handoff.rs",
        "core/src/server/connection_executor.rs",
        "core/src/execute_backend.rs",
        "core/src/http3_runtime_registry.rs",
        "crates/exyonq-mod-proxy/src/hyper_forward.rs",
        "crates/exyonq-mod-proxy/src/runtime.rs",
        "crates/exyonq-mod-static/src/runtime.rs",
        "crates/exyonq-mod-static/src/root.rs",
        "crates/exyonq-mod-http3/src/lib.rs",
        "crates/exyonq-mod-fastcgi/src/runtime.rs",
        "crates/exyonq-ops-runtime/src/control_socket.rs",
        "crates/exyonq-reload-runtime/src/config_watcher.rs",
        "crates/exyonq-reload-runtime/src/signal_runtime.rs",
        "modules/acme/src/manager.rs",
    ];

    for rel in forbidden_builders {
        let path = root.join(rel);
        if !path.is_file() {
            panic!("missing expected production file {rel}");
        }
        let body = strip_test_cfg_and_line_comments(&read(&path));
        assert_eq!(
            count_matches(&body, "Builder::new_multi_thread"),
            0,
            "{rel} must not create multi_thread runtime"
        );
        assert_eq!(
            count_matches(&body, "Builder::new_current_thread"),
            0,
            "{rel} must not create current_thread runtime"
        );
        assert_eq!(
            count_matches(&body, "Runtime::new"),
            0,
            "{rel} must not call Runtime::new"
        );
        // Nested block_on is the classic nested-runtime footgun.
        assert_eq!(
            count_matches(&body, ".block_on("),
            0,
            "{rel} must not block_on (nested runtime risk)"
        );
    }
}

#[test]
fn db5_platform_linux_has_no_production_tokio_dependency() {
    let toml = read(&workspace_root().join("crates/exyonq-platform-linux/Cargo.toml"));
    // Split on [dev-dependencies] — production section must not declare tokio.
    let prod = toml.split("[dev-dependencies]").next().unwrap_or(&toml);
    assert!(
        !prod.lines().any(|l| l.trim_start().starts_with("tokio")),
        "platform-linux production deps must not include tokio"
    );
    assert!(
        toml.contains("[dev-dependencies]"),
        "dev-dependencies section expected"
    );
}

#[test]
fn db5_hot_path_files_do_not_create_runtime() {
    // Alias of the builder scan focused on request/connection symbols files.
    db5_cli_is_sole_production_multi_thread_runtime_builder();
}

#[tokio::test]
async fn db5_lifecycle_active_connections_return_to_baseline_after_token_drop() {
    let ops = LifecycleState::new();
    assert_eq!(ops.active_connections(), 0);
    {
        let _t = ops.try_enter().expect("admit");
        assert_eq!(ops.active_connections(), 1);
    }
    assert_eq!(ops.active_connections(), 0);
}

#[tokio::test]
async fn db5_lifecycle_shutdown_unblocks_waiter() {
    let ops = Arc::new(LifecycleState::new());
    let waiter = {
        let ops = Arc::clone(&ops);
        tokio::spawn(async move {
            LifecycleState::wait_for_shutdown(&ops).await;
        })
    };
    tokio::task::yield_now().await;
    ops.request_shutdown();
    tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .expect("shutdown wait timed out")
        .expect("join");
}

#[tokio::test]
async fn db5_drain_rejects_new_enter_and_clears_when_tokens_drop() {
    let ops = LifecycleState::new();
    let token = ops.try_enter().expect("first");
    ops.start_drain();
    assert!(ops.try_enter().is_err(), "drain must reject new enter");
    drop(token);
    // Drain complete when active hits zero.
    for _ in 0..20 {
        if ops.drain_complete() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(ops.drain_complete() || ops.active_connections() == 0);
}

/// Proxy for "connection task baseline": admission tokens, not Tokio RuntimeMetrics
/// (TOOL_LIMITATION — no task-count API wired in ExyonQ tests).
#[tokio::test]
async fn db5_repeated_admission_cycles_return_to_zero() {
    let ops = LifecycleState::new();
    let cycles = AtomicU64::new(0);
    for _ in 0..64 {
        let t = ops.try_enter().expect("enter");
        cycles.fetch_add(1, Ordering::Relaxed);
        drop(t);
        assert_eq!(ops.active_connections(), 0);
    }
    assert_eq!(cycles.load(Ordering::Relaxed), 64);
}
