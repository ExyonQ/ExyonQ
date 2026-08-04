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

mod common;

use common::{stub_port, TestReloadRuntime};
use exyonq_module_api::reload_runtime::ReloadRuntimeService;
use exyonq_reload_runtime::config_watcher;
use notify::EventKind;

#[test]
fn relevant_event_filter() {
    assert!(config_watcher::is_relevant_event(EventKind::Modify(
        notify::event::ModifyKind::Data(notify::event::DataChange::Any)
    )));
    assert!(config_watcher::is_relevant_event(EventKind::Create(
        notify::event::CreateKind::Any
    )));
    assert!(config_watcher::is_relevant_event(EventKind::Remove(
        notify::event::RemoveKind::Any
    )));
    assert!(!config_watcher::is_relevant_event(EventKind::Access(
        notify::event::AccessKind::Read
    )));
}

#[test]
fn event_targets_config_filters_unrelated_paths() {
    use std::path::PathBuf;
    let cfg = PathBuf::from("/tmp/exyonq-cfg.toml");
    let hit = notify::Event {
        kind: EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        )),
        paths: vec![cfg.clone()],
        attrs: Default::default(),
    };
    let miss = notify::Event {
        kind: EventKind::Create(notify::event::CreateKind::Any),
        paths: vec![PathBuf::from("/tmp/unrelated.sock")],
        attrs: Default::default(),
    };
    assert!(config_watcher::event_targets_config(&hit, &cfg));
    assert!(!config_watcher::event_targets_config(&miss, &cfg));
}

#[tokio::test]
#[cfg(target_os = "linux")]
async fn watcher_debounced_reload_requests_port() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("exyonq.toml");
    std::fs::write(&config_path, "listen = \"127.0.0.1:0\"\n").unwrap();

    let port = stub_port();
    let cancel = tokio_util::sync::CancellationToken::new();
    let handle = exyonq_reload_runtime::config_watcher::spawn_config_watcher(
        config_path.clone(),
        port.clone(),
        cancel.clone(),
    );

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    for _ in 0..5 {
        std::fs::write(&config_path, "listen = \"127.0.0.1:0\"\n").unwrap();
    }

    tokio::time::sleep(std::time::Duration::from_millis(
        config_watcher::CONFIG_RELOAD_DEBOUNCE_MS + 400,
    ))
    .await;

    let reload_count = port.reloads.lock().unwrap().len();
    assert!(reload_count >= 1, "expected at least one reload request");

    cancel.cancel();
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn supervisor_cancels_watcher_task() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("exyonq.toml");
    std::fs::write(&config_path, "listen = \"127.0.0.1:0\"\n").unwrap();

    let port = stub_port();
    let supervisor = TestReloadRuntime.start(Some(config_path), port);
    drop(supervisor);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}
