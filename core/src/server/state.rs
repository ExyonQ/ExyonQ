//! Shared server runtime state (thin wrapper over `RuntimeSnapshot`).

use crate::config::AppConfig;
use crate::execute_backend;
use exyonq_mod_proxy::ProxyClient;
use exyonq_runtime_plan::{compile_runtime_plan, RouteIndex, RuntimePlan};
use std::sync::Arc;

pub struct ServerState {
    pub generation: u64,
    pub snapshot: Arc<RuntimePlan>,
    pub config: AppConfig,
    pub route_index: RouteIndex,
    /// Fast-path alias: compiled `root_slot` for route `"site"` (wire/epoll/H3 bench paths).
    pub site_static_slot: Option<u32>,
    pub proxy_client: ProxyClient,
    /// Composition-owned FPC L1 store (WC2B). `None` when `full_page_cache.enabled = false`.
    pub fpc_cache: Option<Arc<exyonq_cache::ResponseCache>>,
}

impl ServerState {
    pub async fn new(config: AppConfig, proxy_client: ProxyClient) -> anyhow::Result<Arc<Self>> {
        Self::new_with_generation(1, config, proxy_client).await
    }

    pub async fn new_with_generation(
        generation: u64,
        config: AppConfig,
        proxy_client: ProxyClient,
    ) -> anyhow::Result<Arc<Self>> {
        let snapshot = compile_runtime_plan(generation, config.clone())?;
        snapshot.module_state.init().await?;

        let route_index = snapshot.route_tables[0].route_index.clone();
        let fpc_cache = if snapshot.full_page_cache.enabled {
            Some(Arc::new(exyonq_cache::ResponseCache::with_limits(
                snapshot.full_page_cache.max_entries,
                snapshot.full_page_cache.max_total_bytes,
            )))
        } else {
            None
        };

        let state = Arc::new(Self {
            generation: snapshot.generation,
            config: snapshot.config.clone(),
            route_index,
            site_static_slot: snapshot.site_static_slot,
            snapshot,
            proxy_client,
            fpc_cache,
        });
        crate::reload::publish_runtime_generation(state.generation);
        execute_backend::bind_static_compiled_slots(state.generation, &state.snapshot.static_slots);
        execute_backend::bind_proxy_compiled_slots(
            state.generation,
            state.snapshot.proxy_compiled_slots(),
        );
        Ok(state)
    }

    pub fn fingerprint(&self) -> &str {
        self.snapshot.fingerprint.as_str()
    }

    pub fn modules_enabled(&self) -> bool {
        self.snapshot.modules_enabled()
    }
}
