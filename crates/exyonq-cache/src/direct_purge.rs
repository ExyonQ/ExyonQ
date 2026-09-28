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
//! Direct L1 purge port for WC7B1 local / multinode tests (no ServerState).

use crate::{
    build_storage_cache_key, normalize_host, CacheKeyParts, CacheNamespace, NamespaceMetrics,
    ResponseCache,
};
use exyonq_module_api::cache_purge::{CachePurgeOp, CachePurgeOutcome, CachePurgePort};
use exyonq_module_api::{fpc_canonicalize_path, fpc_query_decision, FpcQueryDecision};
use std::sync::Arc;
use std::time::Duration;

/// Applies WC3-shaped purge ops directly to a [`ResponseCache`].
pub struct DirectL1PurgePort {
    pub cache: Arc<ResponseCache>,
    pub namespace: u16,
    pub backend_id: u32,
    pub route_idx: usize,
    pub runtime_generation: u64,
    /// Allowed site ids (cross-tenant reject).
    pub allowed_sites: Vec<u64>,
}

impl DirectL1PurgePort {
    pub fn new(cache: Arc<ResponseCache>, site_id: u64) -> Self {
        Self {
            cache,
            namespace: 4,
            backend_id: 1,
            route_idx: 0,
            runtime_generation: 1,
            allowed_sites: vec![site_id],
        }
    }

    fn authorized(&self, site_id: u64) -> bool {
        self.allowed_sites.contains(&site_id)
    }
}

impl CachePurgePort for DirectL1PurgePort {
    fn purge(&self, op: CachePurgeOp) -> CachePurgeOutcome {
        let gen = self.runtime_generation;
        match op {
            CachePurgeOp::Tag { site_id, .. } => {
                CachePurgeOutcome::fail("purge.tag", site_id, gen, "unsupported_operation")
            }
            CachePurgeOp::Site { site_id } => {
                if !self.authorized(site_id) {
                    return CachePurgeOutcome::fail("purge.site", site_id, gen, "unauthorized");
                }
                let stats = self.cache.invalidate_site(site_id);
                CachePurgeOutcome::success(
                    "purge.site",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    gen,
                )
            }
            CachePurgeOp::Generation {
                site_id,
                generation,
            } => {
                if !self.authorized(site_id) {
                    return CachePurgeOutcome::fail(
                        "purge.generation",
                        site_id,
                        gen,
                        "unauthorized",
                    );
                }
                let stats = self
                    .cache
                    .invalidate_site_runtime_generation(site_id, generation);
                CachePurgeOutcome::success(
                    "purge.generation",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    gen,
                )
            }
            CachePurgeOp::Url {
                site_id,
                scheme,
                host,
                path,
                query,
            } => {
                if !self.authorized(site_id) {
                    return CachePurgeOutcome::fail("purge.url", site_id, gen, "unauthorized");
                }
                let Some(canon_path) = fpc_canonicalize_path(&path) else {
                    return CachePurgeOutcome::fail("purge.url", site_id, gen, "invalid_key");
                };
                let canon_query = match fpc_query_decision(&query, &[]) {
                    FpcQueryDecision::Eligible { canonical } => canonical,
                    FpcQueryDecision::Bypass(_) => {
                        return CachePurgeOutcome::fail("purge.url", site_id, gen, "invalid_key");
                    }
                };
                let key = build_storage_cache_key(CacheKeyParts {
                    site_id,
                    namespace: self.namespace,
                    backend_id: self.backend_id,
                    runtime_generation: self.runtime_generation,
                    policy_generation: 0,
                    route_idx: self.route_idx,
                    method: "GET".into(),
                    scheme,
                    host: normalize_host(Some(&host)),
                    path: canon_path,
                    query: canon_query,
                    content_encoding: "identity".into(),
                });
                let stats = self.cache.invalidate_key(&key);
                if stats.purged_entries == 0 {
                    return CachePurgeOutcome::fail("purge.url", site_id, gen, "not_found");
                }
                CachePurgeOutcome::success(
                    "purge.url",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    gen,
                )
            }
        }
    }
}

/// Insert a simple GET identity entry for tests.
///
/// `host_path` is `(host, path)` so the helper stays under Clippy's argument budget.
pub fn test_insert(
    cache: &ResponseCache,
    site_id: u64,
    route_idx: usize,
    backend_id: u32,
    gen: u64,
    host_path: (&str, &str),
    body: &'static [u8],
) {
    let (host, path) = host_path;
    let key = build_storage_cache_key(CacheKeyParts {
        site_id,
        namespace: 4,
        backend_id,
        runtime_generation: gen,
        policy_generation: 0,
        route_idx,
        method: "GET".into(),
        scheme: "http".into(),
        host: host.into(),
        path: path.into(),
        query: String::new(),
        content_encoding: "identity".into(),
    });
    cache.insert_entry(
        key,
        site_id,
        route_idx,
        gen,
        Duration::from_secs(60),
        200,
        vec![("content-type".into(), "text/plain".into())],
        bytes::Bytes::from_static(body),
        None,
        CacheNamespace::new(4),
        Arc::from([]),
        NamespaceMetrics::NONE,
    );
}

/// Lookup helper returning whether an entry is present.
pub fn test_has(
    cache: &ResponseCache,
    site_id: u64,
    route_idx: usize,
    backend_id: u32,
    gen: u64,
    host: &str,
    path: &str,
) -> bool {
    let key = build_storage_cache_key(CacheKeyParts {
        site_id,
        namespace: 4,
        backend_id,
        runtime_generation: gen,
        policy_generation: 0,
        route_idx,
        method: "GET".into(),
        scheme: "http".into(),
        host: host.into(),
        path: path.into(),
        query: String::new(),
        content_encoding: "identity".into(),
    });
    cache.lookup(&key).is_some()
}
