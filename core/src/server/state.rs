//! Shared server runtime state (thin wrapper over `RuntimeSnapshot`).

use crate::config::AppConfig;
use crate::execute_backend;
use crate::waf::WafRuntimeBinding;
use exyonq_mod_proxy::ProxyClient;
use exyonq_runtime_plan::{compile_runtime_plan, RouteIndex, RuntimePlan};
use exyonq_waf_api::{WafAbuseGate, WafEngine};
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
    /// Generation-frozen WAF engine (Cap013/Cap015 — not a shared mutable ArcSwap).
    pub waf: Arc<dyn WafEngine>,
    /// Generation-frozen abuse gate (None = abuse off for this generation).
    pub waf_abuse: Option<Arc<dyn WafAbuseGate>>,
    /// Host enforce flag bound to this generation.
    pub waf_enforce: bool,
    /// ARCH-002: generation-frozen wire WAF materialization (signatures and/or enforce+abuse).
    pub waf_wire_inspection_active: bool,
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
        let state = Self::new_with_generation_unpublished(generation, config, proxy_client).await?;
        state.publish_compiled_module_slots()?;
        crate::reload::publish_runtime_generation(state.generation);
        Ok(state)
    }

    /// Build a generation-scoped `ServerState` without publishing `RELOAD_GENERATION`
    /// and without binding proxy/static/FCGI module slots.
    ///
    /// Cap048 / Cap013: module slot bind is part of COMMIT. Binding during prepare lets a
    /// late failure leave ClusterTable at N+1 while status/generation remain N.
    pub async fn new_with_generation_unpublished(
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

        let waf_binding: WafRuntimeBinding = crate::waf::waf_binding_for_new_state();
        let state = Arc::new(Self {
            generation: snapshot.generation,
            config: snapshot.config.clone(),
            route_index,
            site_static_slot: snapshot.site_static_slot,
            snapshot,
            proxy_client,
            fpc_cache,
            waf: waf_binding.engine,
            waf_abuse: waf_binding.abuse,
            waf_enforce: waf_binding.enforce,
            waf_wire_inspection_active: waf_binding.wire_inspection_active,
        });
        Ok(state)
    }

    /// Bind proxy/static/FCGI compiled slots for this state's generation (COMMIT / startup).
    pub fn publish_compiled_module_slots(&self) -> anyhow::Result<()> {
        self.snapshot.module_state.publish_wire_module_state();
        publish_response_headers(&self.config);
        execute_backend::bind_static_compiled_slots(self.generation, &self.snapshot.static_slots);
        execute_backend::bind_proxy_compiled_slots(
            self.generation,
            self.snapshot.proxy_compiled_slots(),
        );
        execute_backend::bind_fcgi_compiled_pools(
            self.generation,
            &execute_backend::fcgi_compiled_slots_from_config(&self.config),
        )
        .map_err(|err| anyhow::anyhow!("fcgi reload bind failed: {err}"))?;
        Ok(())
    }

    pub fn fingerprint(&self) -> &str {
        self.snapshot.fingerprint.as_str()
    }

    pub fn modules_enabled(&self) -> bool {
        self.snapshot.modules_enabled()
    }

    /// Connection-pin Hyper force (no headers yet). Compression uses per-request AE gate.
    pub fn hyper_required_for_modules(&self) -> bool {
        self.snapshot.hyper_required_for_modules()
    }

    pub fn compression_configured(&self) -> bool {
        self.snapshot.compression_configured()
    }

    pub fn wire_cheap_modules_active(&self) -> bool {
        self.snapshot.wire_cheap_modules_active()
    }

    /// Cap067 early gate: compression configured ∧ AE prefers content coding.
    pub fn modules_require_hyper_for_request_head(&self, head: &[u8]) -> bool {
        self.snapshot.modules_require_hyper_for_request_head(head)
    }

    /// LA-CAP054-008: OpenMetrics scrape / metrics health must use CrossCuttingPipeline
    /// even when AE-idle wire-cheap would otherwise take `fast_bench_request`.
    pub fn path_requires_module_pipeline(&self, path: &str) -> bool {
        let m = &self.config.modules.metrics;
        m.enabled && (path == m.path.as_str() || path == m.health_path.as_str())
    }
}

static RESPONSE_HEADERS: std::sync::RwLock<
    Vec<(hyper::header::HeaderName, hyper::header::HeaderValue)>,
> = std::sync::RwLock::new(Vec::new());

pub(crate) fn publish_response_headers(config: &AppConfig) {
    let mut pairs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for server in &config.servers {
        for header in &server.response_headers {
            let Ok(name) = hyper::header::HeaderName::from_bytes(header.name.as_bytes()) else {
                continue;
            };
            let Ok(value) = hyper::header::HeaderValue::from_str(&header.value) else {
                continue;
            };
            if seen.insert(name.clone()) {
                pairs.push((name, value));
            }
        }
    }
    if let Ok(mut slot) = RESPONSE_HEADERS.write() {
        *slot = pairs;
    }
}

/// Same artifact version `exyonq --version` prints. Official builds set
/// `EXYONQ_ARTIFACT_VERSION`; otherwise this is the Cargo package version.
pub(crate) const SERVER_PRODUCT: &str = concat!("ExyonQ/", env!("EXYONQ_ARTIFACT_VERSION"));

pub(crate) fn apply_published_response_headers(headers: &mut hyper::HeaderMap) {
    if let Ok(pairs) = RESPONSE_HEADERS.read() {
        for (name, value) in pairs.iter() {
            headers.insert(name.clone(), value.clone());
        }
    }
    if !headers.contains_key(hyper::header::SERVER) {
        headers.insert(
            hyper::header::SERVER,
            hyper::header::HeaderValue::from_static(SERVER_PRODUCT),
        );
    }
}

#[cfg(test)]
mod server_header_tests {
    #[test]
    fn server_header_matches_the_artifact_version() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert(
            hyper::header::SERVER,
            hyper::header::HeaderValue::from_static("custom"),
        );
        super::apply_published_response_headers(&mut headers);
        assert_eq!(headers.get(hyper::header::SERVER).unwrap(), "custom");

        let mut fresh = hyper::HeaderMap::new();
        super::apply_published_response_headers(&mut fresh);
        let server = fresh
            .get(hyper::header::SERVER)
            .and_then(|value| value.to_str().ok())
            .unwrap();
        if server != "custom" {
            assert_eq!(server, super::SERVER_PRODUCT);
        }
        assert!(server.starts_with("ExyonQ/"));
    }
}
