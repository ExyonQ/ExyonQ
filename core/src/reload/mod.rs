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
//! Hot reload: generation swap and config reload mechanism (watcher lives in reload-runtime).

use crate::config::AppConfig;
use crate::discovery_overlay;
use crate::server::state::ServerState;
use crate::tls::{SharedTlsAcceptor, TlsSessionCache, TlsSettings};
use exyonq_mod_proxy::ProxyClient;
use exyonq_runtime_plan::load_config_for_reload;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

pub type SharedServerState = Arc<RwLock<Arc<ServerState>>>;

static RELOAD_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Process-visible runtime generation (advanced on reload; published on ServerState build).
pub fn active_runtime_generation() -> u64 {
    RELOAD_GENERATION.load(Ordering::SeqCst)
}

/// Publish the live generation observed by in-flight FPC fill / reload consumers.
pub fn publish_runtime_generation(generation: u64) {
    RELOAD_GENERATION.store(generation, Ordering::SeqCst);
}

pub fn wrap_state(state: Arc<ServerState>) -> SharedServerState {
    publish_runtime_generation(state.generation);
    Arc::new(RwLock::new(state))
}

pub fn read_state(shared: &SharedServerState) -> Arc<ServerState> {
    Arc::clone(&*shared.read().expect("server state lock poisoned"))
}

/// Apply config from disk and swap the live snapshot (control socket / KernelControlPort).
pub async fn reload_from_path(
    path: &Path,
    shared: &SharedServerState,
    proxy_client: &ProxyClient,
    tls_acceptor: &SharedTlsAcceptor,
    tls_session_cache: &TlsSessionCache,
) -> anyhow::Result<Arc<ServerState>> {
    let state =
        reload_from_path_standalone(path, proxy_client, tls_acceptor, tls_session_cache).await?;
    *shared.write().expect("server state lock poisoned") = Arc::clone(&state);
    Ok(state)
}

pub async fn reload_from_path_standalone(
    path: &Path,
    proxy_client: &ProxyClient,
    tls_acceptor: &SharedTlsAcceptor,
    tls_session_cache: &TlsSessionCache,
) -> anyhow::Result<Arc<ServerState>> {
    let mut config = load_config_for_reload(path)?;
    config = discovery_overlay::apply_env_discovery_overlay(config);
    reload_tls_acceptor(&config, tls_acceptor, tls_session_cache)?;
    let generation = RELOAD_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    // fetch_add already published the previous value+1 into RELOAD_GENERATION after add;
    // the new state's generation must match what in-flight fill sees as "live".
    let state = ServerState::new_with_generation(generation, config, proxy_client.clone()).await?;
    // new_with_generation publishes `generation` again (idempotent).
    exyonq_cache::global_response_cache()
        .invalidate_runtime_generation(generation.saturating_sub(1));
    Ok(state)
}

pub fn reload_tls_acceptor(
    config: &AppConfig,
    tls_acceptor: &SharedTlsAcceptor,
    tls_session_cache: &TlsSessionCache,
) -> anyhow::Result<()> {
    if let Some(tls) = &config.primary_server().tls {
        tls_acceptor
            .load(
                &TlsSettings {
                    cert_path: tls.cert.clone(),
                    key_path: tls.key.clone(),
                },
                tls_session_cache,
                &[b"h2", b"http/1.1"],
            )
            .map_err(|err| anyhow::anyhow!("tls reload: {err}"))?;
    } else {
        tls_acceptor.clear();
    }
    Ok(())
}

// Config load lives in `exyonq-runtime-plan` (KD4.6: compiler boundary).
