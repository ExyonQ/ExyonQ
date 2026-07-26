//! Bench cache pin helpers for Linux accept paths (SyncBenchCache stays CORE).
//!
//! ConnectionPool / ConnJob moved to `exyonq-platform-linux` with sync_accept (PS3A-PM1).
//! Module name retained for D1 SyncBenchCache path assertion (PM4 rename deferred).

/// Bench hot-path snapshot pinned at connection accept/register (Phase 0 generation).
/// Epoll ConnState holds a copy pinned by `CoreConnectionExecutor` at admit/transfer.
#[derive(Clone, Copy)]
pub(crate) struct SyncBenchCache {
    pub site_static_slot: Option<u32>,
    pub modules_enabled: bool,
    pub generation: u64,
}

impl SyncBenchCache {
    pub(crate) fn from_state(state: &crate::server::state::ServerState) -> Self {
        Self {
            site_static_slot: state.site_static_slot,
            modules_enabled: state.modules_enabled(),
            generation: state.generation,
        }
    }

    pub(crate) fn as_generation_view(self) -> crate::kernel::GenerationView {
        crate::kernel::GenerationView {
            generation: self.generation,
            modules_enabled: self.modules_enabled,
            site_static_slot: self.site_static_slot,
        }
    }
}

impl From<SyncBenchCache> for crate::kernel::GenerationView {
    fn from(cache: SyncBenchCache) -> Self {
        cache.as_generation_view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use exyonq_mod_proxy::build_incoming_client;

    #[tokio::test]
    async fn sync_bench_cache_from_state_pins_generation() {
        let raw = include_str!("../../../tests/fixtures/minimal.toml");
        let config: AppConfig = raw.parse().expect("minimal config");
        let proxy_client = build_incoming_client();
        let state =
            crate::server::state::ServerState::new_with_generation(42, config, proxy_client)
                .await
                .expect("state");
        let cache = SyncBenchCache::from_state(state.as_ref());
        assert_eq!(cache.generation, 42);
    }
}
