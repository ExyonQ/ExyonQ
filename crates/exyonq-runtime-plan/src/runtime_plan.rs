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
//! Compiled runtime plan (IR → lookup-oriented structures).

mod backend_plan;

pub use backend_plan::{compile_fcgi_pool_slots, CompiledFcgiPool};

use crate::backend::{Backend, BackendId, BackendTable};
use crate::router::RouteIndex;
use backend_plan::compile_backend_plan;
use exyonq_config_ir::{
    fingerprint as ir_fingerprint, AppConfig, CachePolicyConfig, HtaccessMode, IrFingerprint,
};
use exyonq_module_api::proxy_dispatch::ProxyCompiledSlot;
use exyonq_module_api::static_dispatch::StaticCompiledSlot;
use exyonq_module_api::CompiledCachePolicy;
use exyonq_module_pipeline::CrossCuttingPipeline;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

/// Deterministic fingerprint of compiled plan content.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnapshotFingerprint(pub String);

impl SnapshotFingerprint {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Compiled listener binding (lookup-oriented, no editorial IR fields).
#[derive(Debug, Clone)]
pub struct CompiledListener {
    pub listen: String,
    pub route_table_id: usize,
}

/// Prebuilt route table for a listener.
#[derive(Clone)]
pub struct CompiledRouteTable {
    pub route_index: RouteIndex,
    /// Parallel to IR `routes` vec order (same as `RouteIndex` source routes).
    pub route_backend_ids: Box<[Option<BackendId>]>,
}

/// Resolved upstream cluster metadata (compile-time only — no runtime client).
#[derive(Clone)]
pub struct CompiledCluster {
    pub target_uri: String,
    pub timeout_ms: u64,
}

/// Active filter hooks materialized from module IR.
#[derive(Debug, Clone)]
pub struct CompiledFilterChain {
    pub metrics: bool,
    pub compression: bool,
    pub ratelimit: bool,
}

/// Immutable runtime configuration snapshot consumed by the dataplane.
pub struct RuntimeSnapshot {
    pub generation: u64,
    pub fingerprint: SnapshotFingerprint,
    pub ir_fingerprint: IrFingerprint,
    pub config: AppConfig,
    pub listeners: Vec<CompiledListener>,
    pub route_tables: Vec<CompiledRouteTable>,
    pub clusters: HashMap<String, CompiledCluster>,
    /// Dense compiled static slots parallel to [`Backend::Static`] `root_slot` (IR route order).
    pub static_slots: Box<[StaticCompiledSlot]>,
    /// Fast-path alias: root_slot index for route name `"site"`.
    pub site_static_slot: Option<u32>,
    /// Dense compiled proxy slots parallel to [`Backend::Proxy`] `cluster_id` (sorted upstream names).
    pub proxy_compiled_slots: Box<[ProxyCompiledSlot]>,
    /// Dense FastCGI pool metadata parallel to [`Backend::Fastcgi`] `pool_id` (sorted pool names).
    pub fcgi_slots: Box<[CompiledFcgiPool]>,
    pub backend_table: BackendTable,
    pub filter_chain: CompiledFilterChain,
    pub module_state: CrossCuttingPipeline,
    /// Routes with `htaccess = overlay` (site_id = route name).
    pub htaccess_sites: Box<[HtaccessSiteBinding]>,
    /// Compiled cache policies (sorted by name).
    pub cache_policies: Box<[CompiledCachePolicy]>,
    /// Parallel to IR routes: policy slot id when `cache = "..."` on static routes.
    pub route_cache_policy_ids: Box<[Option<u16>]>,
    /// WC2B full-page cache compile result (lookup gate; default disabled).
    pub full_page_cache: CompiledFullPageCache,
}

/// Compiled `[full_page_cache]` (WC2B).
#[derive(Debug, Clone)]
pub struct CompiledFullPageCache {
    pub enabled: bool,
    pub max_entries: usize,
    pub max_total_bytes: usize,
    pub max_object_bytes: usize,
    pub default_ttl_secs: u64,
    pub max_ttl_secs: u64,
    /// Non-zero when enabled.
    pub namespace: u16,
    /// Parallel to IR routes: stable non-zero site id derived from route name.
    pub route_site_ids: Box<[u64]>,
}

impl Default for CompiledFullPageCache {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries: 10_000,
            max_total_bytes: 64 * 1024 * 1024,
            max_object_bytes: 1024 * 1024,
            default_ttl_secs: 30,
            max_ttl_secs: 3600,
            namespace: 4,
            route_site_ids: Box::new([]),
        }
    }
}

/// Stable non-zero site id from route name (never Host; never silent 0).
pub fn stable_fpc_site_id(route_name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "exyonq.fpc.site.v1".hash(&mut hasher);
    route_name.hash(&mut hasher);
    let v = hasher.finish();
    if v == 0 {
        1
    } else {
        v
    }
}

pub fn compile_full_page_cache(config: &AppConfig) -> CompiledFullPageCache {
    let cfg = &config.full_page_cache;
    let route_site_ids = config
        .routes
        .iter()
        .map(|r| stable_fpc_site_id(&r.name))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    CompiledFullPageCache {
        enabled: cfg.enabled,
        max_entries: cfg.max_entries,
        max_total_bytes: cfg.max_total_bytes,
        max_object_bytes: cfg.max_object_bytes,
        default_ttl_secs: cfg.default_ttl_seconds,
        max_ttl_secs: cfg.max_ttl_seconds,
        namespace: cfg.namespace,
        route_site_ids,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtaccessSiteBinding {
    pub route_idx: usize,
    pub site_id: String,
    pub document_root: PathBuf,
}

impl RuntimeSnapshot {
    pub fn route_index(&self) -> &RouteIndex {
        self.route_tables
            .first()
            .map(|t| &t.route_index)
            .expect("plan has route table")
    }

    pub fn backend_table(&self) -> &BackendTable {
        &self.backend_table
    }

    pub fn route_backend_id(&self, route_idx: usize) -> Option<BackendId> {
        self.route_tables
            .first()?
            .route_backend_ids
            .get(route_idx)
            .copied()
            .flatten()
    }

    /// Resolve a compiled backend for `route_idx`.
    #[inline]
    pub fn resolve_backend(&self, route_idx: usize) -> Option<&Backend> {
        let id = self.route_backend_id(route_idx)?;
        self.backend_table().get(id)
    }

    /// Compiled static slot metadata for a [`Backend::Static`] `root_slot` (O(1)).
    #[inline]
    pub fn static_slot_for_root(&self, root_slot: u32) -> Option<&StaticCompiledSlot> {
        self.static_slots.get(root_slot as usize)
    }

    /// Compiled static slot for a route's backend when it resolves to [`Backend::Static`].
    #[inline]
    pub fn static_slot_for_route_backend(&self, route_idx: usize) -> Option<&StaticCompiledSlot> {
        match self.resolve_backend(route_idx)? {
            Backend::Static { root_slot } => self.static_slot_for_root(*root_slot),
            _ => None,
        }
    }

    /// Compiled proxy cluster metadata for `cluster_id` (O(1)).
    #[inline]
    pub fn proxy_compiled_slot(&self, cluster_id: u32) -> Option<&ProxyCompiledSlot> {
        self.proxy_compiled_slots.get(cluster_id as usize)
    }

    /// Proxy `cluster_id` for a route's compiled backend when it resolves to [`Backend::Proxy`].
    #[inline]
    pub fn proxy_cluster_for_route(&self, route_idx: usize) -> Option<u32> {
        match self.resolve_backend(route_idx)? {
            Backend::Proxy { cluster_id } => Some(*cluster_id),
            _ => None,
        }
    }

    /// Compiled proxy cluster metadata (sorted upstream name order).
    #[inline]
    pub fn proxy_compiled_slots(&self) -> &[ProxyCompiledSlot] {
        &self.proxy_compiled_slots
    }

    /// Document root for a compiled [`Backend::Fastcgi`] `pool_id` (O(1)).
    #[inline]
    pub fn fcgi_pool_slot(&self, pool_id: u32) -> Option<&CompiledFcgiPool> {
        self.fcgi_slots.get(pool_id as usize)
    }

    /// Resolved structural document root for a FastCGI pool (config only — env applied at dispatch).
    #[inline]
    pub fn fcgi_pool_document_root(&self, pool_id: u32) -> Option<&PathBuf> {
        self.fcgi_pool_slot(pool_id)
            .and_then(|slot| slot.document_root.as_ref())
    }

    pub fn modules_enabled(&self) -> bool {
        !self.module_state.is_empty()
    }

    pub fn htaccess_site_id_for_route(&self, route_idx: usize) -> Option<&str> {
        self.htaccess_binding_for_route(route_idx)
            .map(|b| b.site_id.as_str())
    }

    pub fn htaccess_binding_for_route(&self, route_idx: usize) -> Option<&HtaccessSiteBinding> {
        self.htaccess_sites
            .iter()
            .find(|b| b.route_idx == route_idx)
    }

    pub fn cache_policy_for_route(&self, route_idx: usize) -> Option<&CompiledCachePolicy> {
        let slot = *self.route_cache_policy_ids.get(route_idx)?;
        let slot = slot? as usize;
        self.cache_policies.get(slot)
    }
}

/// Phase 0 canonical name for [`RuntimeSnapshot`] (same type, zero layout change).
pub type RuntimePlan = RuntimeSnapshot;

/// Compile overlay site bindings from validated IR (no request-time config reads).
pub fn compile_htaccess_site_bindings(config: &AppConfig) -> Vec<HtaccessSiteBinding> {
    config
        .routes
        .iter()
        .enumerate()
        .filter_map(|(idx, route)| {
            if route.htaccess != HtaccessMode::Overlay {
                return None;
            }
            let document_root = route.htaccess_document_root(&config.pools_fcgi)?.clone();
            Some(HtaccessSiteBinding {
                route_idx: idx,
                site_id: route.name.clone(),
                document_root,
            })
        })
        .collect()
}

fn policy_generation_hash(policy: &CachePolicyConfig) -> u64 {
    let mut hasher = DefaultHasher::new();
    policy.name.hash(&mut hasher);
    policy.ttl_seconds.hash(&mut hasher);
    policy.max_object_bytes.hash(&mut hasher);
    hasher.finish()
}

pub fn compile_cache_policies(
    config: &AppConfig,
) -> (Box<[CompiledCachePolicy]>, Box<[Option<u16>]>) {
    let mut names: Vec<String> = config.cache_policies.keys().cloned().collect();
    names.sort();
    let mut name_to_slot = HashMap::new();
    let mut policies = Vec::with_capacity(names.len());
    for name in names {
        let cfg = &config.cache_policies[&name];
        let slot = policies.len() as u16;
        name_to_slot.insert(name.clone(), slot);
        policies.push(CompiledCachePolicy {
            name: cfg.name.clone(),
            ttl: std::time::Duration::from_secs(cfg.ttl_seconds),
            max_object_bytes: cfg.max_object_bytes,
            policy_generation: policy_generation_hash(cfg),
        });
    }
    let route_ids = config
        .routes
        .iter()
        .map(|route| {
            route
                .cache
                .as_ref()
                .and_then(|n| name_to_slot.get(n).copied())
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    (policies.into_boxed_slice(), route_ids)
}

/// Compile validated IR into an immutable [`RuntimePlan`].
pub fn compile_runtime_plan_from_ir(
    generation: u64,
    config: AppConfig,
) -> anyhow::Result<Arc<RuntimePlan>> {
    let ir_fp = ir_fingerprint(&config);
    let modules = CrossCuttingPipeline::from_config(&config.modules, env!("CARGO_PKG_VERSION"))
        .map_err(|err| anyhow::anyhow!("addon handshake: {err}"))?;

    let backend_plan = compile_backend_plan(&config.routes, &config.upstreams, &config.pools_fcgi);
    let fcgi_slots = compile_fcgi_pool_slots(&config.pools_fcgi);
    for slot in fcgi_slots.iter() {
        if slot.document_root.is_none() {
            tracing::warn!(
                pool = %slot.name,
                "fcgi_pool.document_root missing; EXYONQ_FCGI_DOCUMENT_ROOT fallback or 502 at request time"
            );
        }
    }

    let mut static_slots = Vec::new();
    let mut site_static_slot = None;
    for route in &config.routes {
        if let Some(root) = &route.root {
            let slot_idx = static_slots.len() as u32;
            if route.name == "site" {
                site_static_slot = Some(slot_idx);
            }
            static_slots.push(StaticCompiledSlot {
                filesystem_root: root.clone(),
                route_prefix: route.r#match.path.clone(),
                index_file: route.index.clone(),
                route_name: route.name.clone(),
                preload_max_file_bytes: config.static_section.preload.max_file_bytes,
                preload_max_total_bytes: config.static_section.preload.max_total_bytes,
                preload_max_entries: config.static_section.preload.max_entries,
            });
        }
    }
    let static_slots = static_slots.into_boxed_slice();

    let route_index = RouteIndex::new(config.routes.clone());

    let mut clusters = HashMap::new();
    let proxy_compiled_slots = compile_proxy_compiled_slots(&config, &mut clusters)?;

    let filter_chain = CompiledFilterChain {
        metrics: config.modules.metrics.enabled,
        compression: config.modules.compression.enabled,
        ratelimit: config.modules.ratelimit.enabled,
    };

    let route_tables = vec![CompiledRouteTable {
        route_index,
        route_backend_ids: backend_plan.route_backend_ids,
    }];
    let listeners = config
        .servers
        .iter()
        .enumerate()
        .map(|(id, server)| CompiledListener {
            listen: server.listen.clone(),
            route_table_id: id.min(route_tables.len().saturating_sub(1)),
        })
        .collect();

    let htaccess_sites = compile_htaccess_site_bindings(&config);
    let (cache_policies, route_cache_policy_ids) = compile_cache_policies(&config);
    let full_page_cache = compile_full_page_cache(&config);
    if full_page_cache.enabled {
        tracing::info!(
            enabled = true,
            max_entries = full_page_cache.max_entries,
            max_total_bytes = full_page_cache.max_total_bytes,
            ttl_secs = full_page_cache.default_ttl_secs,
            namespace = full_page_cache.namespace,
            "full_page_cache L1 lookup gate enabled (insert deferred to WC2C)"
        );
    }

    let plan = RuntimeSnapshot {
        generation,
        fingerprint: compute_snapshot_fingerprint(&config, &filter_chain),
        ir_fingerprint: ir_fp,
        config,
        listeners,
        route_tables,
        clusters,
        static_slots,
        site_static_slot,
        proxy_compiled_slots,
        fcgi_slots,
        backend_table: backend_plan.backend_table,
        filter_chain,
        module_state: modules,
        htaccess_sites: htaccess_sites.into_boxed_slice(),
        cache_policies,
        route_cache_policy_ids,
        full_page_cache,
    };

    Ok(Arc::new(plan))
}

/// Compile from config IR (public alias kept stable).
pub fn compile_runtime_plan(
    generation: u64,
    config: AppConfig,
) -> anyhow::Result<Arc<RuntimePlan>> {
    compile_runtime_plan_from_ir(generation, config)
}

fn compile_proxy_compiled_slots(
    config: &AppConfig,
    clusters: &mut HashMap<String, CompiledCluster>,
) -> anyhow::Result<Box<[ProxyCompiledSlot]>> {
    use std::time::Duration;

    let mut upstream_names: Vec<String> = config.upstreams.keys().cloned().collect();
    upstream_names.sort();
    let mut slots = Vec::with_capacity(upstream_names.len());
    for (cluster_id, name) in upstream_names.into_iter().enumerate() {
        let upstream = config
            .upstreams
            .get(&name)
            .ok_or_else(|| anyhow::anyhow!("upstream {name}: missing from config"))?;
        upstream
            .target
            .parse::<http::Uri>()
            .map_err(|err| anyhow::anyhow!("upstream {name}: {err}"))?;
        clusters.insert(
            name.clone(),
            CompiledCluster {
                target_uri: upstream.target.clone(),
                timeout_ms: upstream.timeout_ms,
            },
        );
        slots.push(ProxyCompiledSlot {
            cluster_id: cluster_id as u32,
            upstream_name: name,
            target: upstream.target.clone(),
            timeout: Duration::from_millis(upstream.timeout_ms),
        });
    }
    Ok(slots.into_boxed_slice())
}

fn compute_snapshot_fingerprint(
    config: &AppConfig,
    filters: &CompiledFilterChain,
) -> SnapshotFingerprint {
    let ir = ir_fingerprint(config);
    let tag = format!(
        "{}:m{}c{}r{}",
        ir.as_str(),
        filters.metrics as u8,
        filters.compression as u8,
        filters.ratelimit as u8
    );
    SnapshotFingerprint(tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use exyonq_config_ir::{
        RedirectConfig, RouteConfig, RouteMatch, ServerConfig, ServerNames, UpstreamConfig,
    };

    fn assert_no_contract_only_backends(table: &BackendTable) {
        for idx in 0..table.len() {
            let backend = table
                .get(BackendId::from_index(idx as u32))
                .expect("backend slot");
            assert!(
                !backend.is_contract_only(),
                "unexpected contract-only backend at {idx}: {backend:?}"
            );
        }
    }

    fn compile_config(config: AppConfig) -> Arc<RuntimeSnapshot> {
        compile_runtime_plan_from_ir(1, config).unwrap()
    }

    fn route(name: &str, path: &str) -> RouteConfig {
        RouteConfig {
            name: name.into(),
            r#match: RouteMatch {
                path: path.into(),
                host: None,
            },
            upstream: None,
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        }
    }

    #[test]
    fn compile_is_deterministic() {
        let raw = include_str!("../../../tests/fixtures/minimal.toml");
        let config: AppConfig = raw.parse().unwrap();
        let a = compile_runtime_plan_from_ir(1, config.clone()).unwrap();
        let b = compile_runtime_plan_from_ir(1, config).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.ir_fingerprint, b.ir_fingerprint);
        assert_eq!(a.backend_table, b.backend_table);
    }

    #[test]
    fn backend_table_populated_minimal_toml() {
        let raw = include_str!("../../../tests/fixtures/minimal.toml");
        let config: AppConfig = raw.parse().unwrap();
        let snap = compile_config(config);
        assert_eq!(snap.backend_table.len(), 1);
        let api_id = snap.route_backend_id(0).expect("api route backend id");
        assert!(matches!(
            snap.backend_table.get(api_id),
            Some(Backend::Proxy { cluster_id: 0 })
        ));
        assert_no_contract_only_backends(&snap.backend_table);
    }

    #[test]
    fn redirect_route_has_no_backend_id() {
        let mut redir = route("legacy", "/");
        redir.redirect = Some(RedirectConfig {
            status: 302,
            location: "/new".into(),
        });
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["legacy".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![redir],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
};
        let snap = compile_config(config);
        assert!(snap.route_backend_id(0).is_none());
    }

    #[test]
    fn upstream_cluster_id_matches_sorted_order() {
        let mut upstreams = HashMap::new();
        upstreams.insert(
            "zebra".into(),
            UpstreamConfig {
                name: "zebra".into(),
                target: "http://127.0.0.1:1".into(),
                timeout_ms: 1000,
            },
        );
        upstreams.insert(
            "alpha".into(),
            UpstreamConfig {
                name: "alpha".into(),
                target: "http://127.0.0.1:2".into(),
                timeout_ms: 1000,
            },
        );
        let mut api = route("api", "/api");
        api.upstream = Some("alpha".into());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["api".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![api],
            upstreams,
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
};
        let snap = compile_config(config);
        let api_id = snap.route_backend_id(0).unwrap();
        assert!(matches!(
            snap.backend_table.get(api_id),
            Some(Backend::Proxy { cluster_id: 0 })
        ));
        assert!(matches!(
            snap.backend_table.get(BackendId::from_index(1)),
            Some(Backend::Proxy { cluster_id: 1 })
        ));
        assert_eq!(snap.proxy_compiled_slots.len(), 2);
        assert_eq!(snap.proxy_compiled_slot(0).unwrap().upstream_name, "alpha");
        assert_eq!(snap.proxy_compiled_slot(1).unwrap().upstream_name, "zebra");
        assert_eq!(snap.proxy_cluster_for_route(0), Some(0));
        assert_eq!(
            snap.proxy_compiled_slot(0).unwrap().target,
            snap.clusters.get("alpha").unwrap().target_uri
        );
    }
}
