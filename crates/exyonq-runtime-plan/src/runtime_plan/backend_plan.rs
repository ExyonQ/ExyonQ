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
//! Compile-time backend plan (ADR-029 PR-5): populate [`BackendTable`] and per-route ids.

use crate::backend::{Backend, BackendId, BackendTable};
use exyonq_config_ir::{FcgiPoolConfig, RouteConfig, UpstreamConfig};
use std::collections::HashMap;
use std::path::PathBuf;

/// Compiled FastCGI pool metadata parallel to [`Backend::Fastcgi`] `pool_id` (sorted pool names).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFcgiPool {
    pub name: String,
    pub document_root: Option<PathBuf>,
}

/// Dense fcgi pool slots in the same order as [`Backend::Fastcgi`] entries in [`BackendTable`].
pub fn compile_fcgi_pool_slots(
    pools_fcgi: &HashMap<String, FcgiPoolConfig>,
) -> Box<[CompiledFcgiPool]> {
    let mut pool_names: Vec<String> = pools_fcgi.keys().cloned().collect();
    pool_names.sort();
    pool_names
        .into_iter()
        .map(|name| {
            let pool = pools_fcgi.get(&name).expect("pool name in map");
            CompiledFcgiPool {
                name,
                document_root: pool.document_root.clone(),
            }
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

/// Output of backend plan compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendCompileResult {
    pub backend_table: BackendTable,
    pub route_backend_ids: Box<[Option<BackendId>]>,
}

/// Build dense [`BackendTable`] and parallel route → [`BackendId`] map.
pub fn compile_backend_plan(
    routes: &[RouteConfig],
    upstreams: &HashMap<String, UpstreamConfig>,
    pools_fcgi: &HashMap<String, FcgiPoolConfig>,
) -> BackendCompileResult {
    let mut root_slot_by_route: HashMap<String, u32> = HashMap::new();
    let mut static_backends: Vec<Backend> = Vec::new();
    for route in routes {
        if route.root.is_some() {
            let slot = static_backends.len() as u32;
            root_slot_by_route.insert(route.name.clone(), slot);
            static_backends.push(Backend::Static { root_slot: slot });
        }
    }

    let mut upstream_names: Vec<String> = upstreams.keys().cloned().collect();
    upstream_names.sort();
    let mut cluster_id_by_name: HashMap<String, u32> = HashMap::new();
    let mut proxy_backends: Vec<Backend> = Vec::new();
    for name in &upstream_names {
        let cluster_id = proxy_backends.len() as u32;
        cluster_id_by_name.insert(name.clone(), cluster_id);
        proxy_backends.push(Backend::Proxy { cluster_id });
    }

    let static_base = 0u32;
    let proxy_base = static_backends.len() as u32;

    let mut pool_names: Vec<String> = pools_fcgi.keys().cloned().collect();
    pool_names.sort();
    let mut pool_id_by_name: HashMap<String, u32> = HashMap::new();
    let mut fastcgi_backends: Vec<Backend> = Vec::new();
    for name in &pool_names {
        let pool_id = fastcgi_backends.len() as u32;
        pool_id_by_name.insert(name.clone(), pool_id);
        fastcgi_backends.push(Backend::Fastcgi { pool_id });
    }

    let fastcgi_base = proxy_base + proxy_backends.len() as u32;

    let mut table_entries = static_backends;
    table_entries.extend(proxy_backends);
    table_entries.extend(fastcgi_backends);
    let backend_table = BackendTable::from_backends(table_entries);

    let route_backend_ids: Vec<Option<BackendId>> = routes
        .iter()
        .map(|route| {
            route_backend_id(
                route,
                static_base,
                proxy_base,
                fastcgi_base,
                &root_slot_by_route,
                &cluster_id_by_name,
                &pool_id_by_name,
            )
        })
        .collect();

    BackendCompileResult {
        backend_table,
        route_backend_ids: route_backend_ids.into_boxed_slice(),
    }
}

fn route_backend_id(
    route: &RouteConfig,
    static_base: u32,
    proxy_base: u32,
    fastcgi_base: u32,
    root_slot_by_route: &HashMap<String, u32>,
    cluster_id_by_name: &HashMap<String, u32>,
    pool_id_by_name: &HashMap<String, u32>,
) -> Option<BackendId> {
    if route.redirect.is_some() {
        return None;
    }
    if route.root.is_some() {
        let slot = *root_slot_by_route.get(&route.name)?;
        return Some(BackendId::from_index(static_base + slot));
    }
    if let Some(upstream) = &route.upstream {
        let cluster_id = *cluster_id_by_name.get(upstream)?;
        return Some(BackendId::from_index(proxy_base + cluster_id));
    }
    if let Some(pool) = &route.fastcgi {
        let pool_id = *pool_id_by_name.get(pool)?;
        return Some(BackendId::from_index(fastcgi_base + pool_id));
    }
    None
}
