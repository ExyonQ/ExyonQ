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

use exyonq_core::kernel_control_port::CoreKernelControlPort;
use exyonq_core::lifecycle::LifecycleState;
use exyonq_core::reload::{read_state, reload_from_path, wrap_state};
use exyonq_core::server::state::ServerState;
use exyonq_mod_proxy::build_incoming_client;
use exyonq_mod_tls::{SharedTlsAcceptor, TlsSessionCache};
use exyonq_module_api::kernel_control::KernelControlPort;
use std::path::PathBuf;
use tempfile::TempDir;

fn write_valid_config(path: &std::path::Path) {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/static.toml");
    std::fs::write(path, std::fs::read_to_string(fixture).unwrap()).unwrap();
}

#[tokio::test]
async fn valid_reload_increments_generation() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let gen_before = read_state(&shared).generation;
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();

    reload_from_path(&path, &shared, &proxy, &tls, &cache)
        .await
        .unwrap();
    assert!(read_state(&shared).generation > gen_before);
}

#[tokio::test]
async fn invalid_reload_keeps_previous_generation() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let gen_before = read_state(&shared).generation;
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();

    std::fs::write(&path, "not valid toml [[[").unwrap();
    assert!(reload_from_path(&path, &shared, &proxy, &tls, &cache)
        .await
        .is_err());
    assert_eq!(read_state(&shared).generation, gen_before);
}

#[tokio::test]
async fn lifecycle_wait_for_shutdown_unblocks_after_request() {
    let ops = LifecycleState::new();
    let ops_wait = ops.clone();
    let handle = tokio::spawn(async move {
        LifecycleState::wait_for_shutdown(&ops_wait).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(!handle.is_finished());
    ops.request_shutdown();
    tokio::time::timeout(std::time::Duration::from_secs(1), handle)
        .await
        .expect("wait_for_shutdown should complete")
        .unwrap();
}

#[tokio::test]
async fn kernel_port_rejects_reload_after_shutdown() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);
    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let ops = LifecycleState::new();
    ops.request_shutdown();
    let port = CoreKernelControlPort {
        shared,
        proxy_client: proxy,
        tls_acceptor: SharedTlsAcceptor::new(),
        tls_session_cache: TlsSessionCache::default(),
        lifecycle: ops,
    };
    let outcome = port.request_reload(&path).await;
    assert!(!outcome.ok);
}
