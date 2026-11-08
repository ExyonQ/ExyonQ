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
    /// Fast-path alias: compiled `root_slot` of the first static slot (wire/epoll).
    /// Historical name; the value is not tied to a route called `site` (L-02).
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
        Self::build_with_waf_binding(
            generation,
            config,
            proxy_client,
            crate::waf::waf_binding_for_new_state(),
        )
        .await
    }

    /// Test-only published constructor with an explicit WAF binding: tests that
    /// need a rejecting/inspecting WAF binding must not install it through
    /// `install_waf_runtime_binding` — the process-global binding slot is
    /// snapshotted by every concurrent `ServerState` build, so an
    /// `AlwaysRateLimit`-style gate would leak into unrelated tests and 429 them.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) async fn new_with_waf_binding_for_test(
        generation: u64,
        config: AppConfig,
        proxy_client: ProxyClient,
        waf_binding: WafRuntimeBinding,
    ) -> anyhow::Result<Arc<Self>> {
        let state =
            Self::build_with_waf_binding(generation, config, proxy_client, waf_binding).await?;
        state.publish_compiled_module_slots()?;
        crate::reload::publish_runtime_generation(state.generation);
        Ok(state)
    }

    async fn build_with_waf_binding(
        generation: u64,
        config: AppConfig,
        proxy_client: ProxyClient,
        waf_binding: WafRuntimeBinding,
    ) -> anyhow::Result<Arc<Self>> {
        let snapshot = compile_runtime_plan(generation, config.clone())?;
        snapshot.module_state.init().await?;

        let route_index = snapshot.route_tables[0].route_index.clone();
        let fpc_cache = if snapshot.full_page_cache.enabled {
            Some(Arc::new(
                exyonq_cache::ResponseCache::with_limits(
                    snapshot.full_page_cache.max_entries,
                    snapshot.full_page_cache.max_total_bytes,
                )
                .with_stale_retention(std::time::Duration::from_secs(
                    snapshot.full_page_cache.stale_if_error_secs,
                )),
            ))
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
            waf: waf_binding.engine,
            waf_abuse: waf_binding.abuse,
            waf_enforce: waf_binding.enforce,
            waf_wire_inspection_active: waf_binding.wire_inspection_active,
        });
        Ok(state)
    }

    /// Bind proxy/static/FCGI compiled slots for this state's generation (COMMIT / startup).
    ///
    /// Also publishes Cap055/Cap054 process-wide wire flags (ratelimit params + metrics
    /// enable) so Hyper `on_route` and Cap067 `wire_admit` share the committed burst/rps —
    /// not the ProcessModuleLimiter defaults (100/200).
    pub fn publish_compiled_module_slots(&self) -> anyhow::Result<()> {
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
        // Cap055 COMMIT: snapshot-owned params → process limiter / wire_admit_active.
        self.snapshot.module_state.publish_wire_module_state();
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

    /// NG-18: any route or the global module enables `gunzip`.
    pub fn gunzip_configured(&self) -> bool {
        self.config.modules.gunzip.enabled
            || self.config.routes.iter().any(|r| r.gunzip == Some(true))
    }

    /// NG-18: any route mirrors, so requests must reach the dispatch body hook.
    pub fn mirror_configured(&self) -> bool {
        !self.config.modules.mirror.routes.is_empty()
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

    /// Route-bound features the fast paths (wire proxy GET, epoll sendfile,
    /// blocking static pool, static shortcut) cannot honor — the
    /// request must take the full dispatch path.
    ///
    /// AUDIT-D1/D2/D7: wire and static fast paths implement transport only —
    /// they cannot emit a route 401 (`auth_basic`), per-zone 429
    /// (`limit_req_zone`), `sub_filter` transforms, mirror subrequests,
    /// `slice` assembly, `.htaccess` overlays, `[[cache_policy]]` stores, or
    /// `[[route.response_headers]]`. Cap056 added this gate for `route.cache`
    /// alone; every route-bound feature shipped since silently no-ops on
    /// fast paths. Extend this check whenever a feature binds to a route.
    pub fn route_requires_full_dispatch(&self, route_idx: usize) -> bool {
        self.snapshot.cache_policy_for_route(route_idx).is_some()
            || !self.snapshot.route_response_headers(route_idx).is_empty()
            || self
                .snapshot
                .htaccess_site_id_for_route(route_idx)
                .is_some()
            || self.config.routes.get(route_idx).is_some_and(|route| {
                let modules = &self.config.modules;
                let name = route.name.as_str();
                // EXY-16/ADR-050: the wire transport cannot substitute a
                // server error document after an upstream 4xx/5xx. Keep such
                // routes on the full dispatch path, where error_pages runs.
                let server_has_error_pages = self.config.servers.iter().any(|server| {
                    !server.error_pages.is_empty()
                        && server.routes.iter().any(|candidate| candidate == name)
                });
                // cache_headers needs the Hyper path to emit Cache-Control —
                // the wire surface cannot materialize it (EXY-01/EXY-05 trap).
                // dir_redirect is NOT gated here — the wire path emits the
                // NGINX-style 301 inline (EXY-03), keeping sendfile eligible.
                server_has_error_pages
                    || route.cache_headers.is_some()
                    // Structural rewrite/redirect is evaluated only after route
                    // lookup in full dispatch. The wire path would answer 404
                    // (rewrite) or bypass the route redirect entirely.
                    || route.rewrite.is_some()
                    || route.redirect.is_some()
                    || route.access.is_some()
                    || modules.auth.routes.contains_key(name)
                    || modules.ratelimit.client.enabled
                    || modules.ratelimit.routes.contains_key(name)
                    || modules.sub_filter.routes.contains_key(name)
                    || modules.mirror.routes.contains_key(name)
                    || modules.slice.routes.contains_key(name)
            })
    }

    /// Generation-wide features no fast path can honor at all: FPC must see
    /// every eligible GET (lookup/store/stale/`X-Cache`), gunzip transforms
    /// response bodies, and response-phase WAF rules inspect what fast paths
    /// emit unchecked.
    pub fn wire_incompatible_features_active(&self) -> bool {
        self.snapshot.full_page_cache.enabled
            || self.gunzip_configured()
            || self.config.modules.ratelimit.client.enabled
            || self.waf.inspects_responses()
    }
}
