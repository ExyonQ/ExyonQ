/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use arc_swap::ArcSwap;
use exyonq_runtime_plan::{Backend, RouteIndex, RuntimePlan};
use exyonq_waf_api::WafEngine;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

/// Immutable generation projection published to SDP shards (no hot Mutex).
pub struct GenerationProjection {
    pub generation_id: u64,
    pub plan: Arc<RuntimePlan>,
    pub route_index: RouteIndex,
    pub waf: Arc<dyn WafEngine>,
    pub waf_enforce: bool,
    pub waf_wire_inspection_active: bool,
    pub default_connect_timeout: Duration,
    pub default_header_timeout: Duration,
    pub default_idle_timeout: Duration,
}

pub type SharedProjection = Arc<ArcSwap<GenerationProjection>>;

pub fn build_projection_from_parts(
    generation_id: u64,
    plan: Arc<RuntimePlan>,
    route_index: RouteIndex,
    waf: Arc<dyn WafEngine>,
    waf_enforce: bool,
    waf_wire_inspection_active: bool,
) -> Arc<GenerationProjection> {
    Arc::new(GenerationProjection {
        generation_id,
        plan,
        route_index,
        waf,
        waf_enforce,
        waf_wire_inspection_active,
        default_connect_timeout: Duration::from_secs(5),
        default_header_timeout: Duration::from_secs(30),
        default_idle_timeout: Duration::from_secs(60),
    })
}

impl GenerationProjection {
    /// Resolve `/api/`-class proxy route → first executable upstream addr.
    pub fn resolve_proxy_upstream(
        &self,
        path: &str,
        host: Option<&str>,
    ) -> Option<(usize, u32, SocketAddr, String, Duration)> {
        let (route_idx, _route) = self.route_index.match_route_index_with_host(path, host)?;
        let cluster_id = self.plan.proxy_cluster_for_route(route_idx)?;
        let slot = self.plan.proxy_compiled_slot(cluster_id)?;
        if !slot.single_endpoint_executable && !slot.multi_endpoint_executable {
            return None;
        }
        let uri = if !slot.target.is_empty() {
            slot.target.as_str()
        } else {
            slot.endpoints.first()?.http_uri.as_str()
        };
        let addr = parse_http_target_addr(uri)?;
        let host_hdr = authority_host(uri).unwrap_or_else(|| addr.to_string());
        Some((route_idx, cluster_id, addr, host_hdr, slot.timeout))
    }

    pub fn route_is_proxy(&self, route_idx: usize) -> bool {
        matches!(
            self.plan.resolve_backend(route_idx),
            Some(Backend::Proxy { .. })
        )
    }
}

fn authority_host(uri: &str) -> Option<String> {
    let rest = uri
        .strip_prefix("http://")
        .or_else(|| uri.strip_prefix("https://"))?;
    let hostport = rest.split('/').next()?;
    Some(hostport.to_string())
}

fn parse_http_target_addr(uri: &str) -> Option<SocketAddr> {
    let rest = uri
        .strip_prefix("http://")
        .or_else(|| uri.strip_prefix("https://"))?;
    let hostport = rest.split('/').next()?;
    hostport
        .to_socket_addrs()
        .ok()?
        .find(|a| matches!(a, SocketAddr::V4(_) | SocketAddr::V6(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_literal_ipv4_target() {
        let a = parse_http_target_addr("http://127.0.0.1:9000").expect("addr");
        assert_eq!(a.port(), 9000);
    }
}
