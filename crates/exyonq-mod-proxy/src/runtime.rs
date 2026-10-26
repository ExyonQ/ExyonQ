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
//! `ProxyDispatchService` runtime — Hyper upstream execution (KD3.2) + P2B WRR.

use crate::dns_refresh::{DnsClusterSpec, DnsPeer, DnsRefreshSupervisor};
use crate::headers::{request_headers_safe_for_proxy, strip_hop_by_hop_headers};
use crate::health::{peer_key_from_http_uri, HealthSupervisor};
use crate::hyper_client::{build_incoming_client, post_body_client, ProxyClient};
use crate::hyper_forward::{
    attempt_forward_empty_body, classify_hyper_response, forward_get_streaming, forward_request,
    note_connect_retry, response_for_attempt_class_ctx, ProxyHyperMetrics,
};
use crate::retry::{
    classify_hyper_error, may_retry_connect, remaining_budget, shared_deadline,
    UpstreamAttemptClass,
};
use crate::runtime_diag::{emit_no_eligible, emit_upstream_class, RtFailCtx};
use crate::selector::{
    endpoint_transport_identity, EndpointAdmin, EndpointSelector, EndpointSpec, FailoverMode,
    InflightLease, PassivePeer, SelectionOutcome,
};
use crate::sse::upstream_timeout_for_path;
use crate::upstream_target::UpstreamTarget;
use crate::UpstreamDescriptor;
use async_trait::async_trait;
use exyonq_module_api::proxy_dispatch::ProxySelectionPolicy;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest, ProxyDispatchService,
    ProxyMethod, ProxyMetricsSnapshot, PROXY_MAX_REQUEST_BODY_BYTES,
};
use http::Uri;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, HOST};
use hyper::{Method, Request};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tracing::warn;

enum ClusterBinding {
    NoEligible,
    /// Sole-eligible upstream (nginx single-server exemption). `admin` keeps the
    /// member administrable via `exyonqctl server set` without a selector.
    Single {
        target: Box<UpstreamTarget>,
        admin: Arc<EndpointAdmin>,
        endpoint_id: String,
    },
    Multi {
        selector: Arc<EndpointSelector>,
        /// Parallel to `selector.endpoints()` — `None` when ineligible / unparseable.
        targets_by_index: Box<[Option<UpstreamTarget>]>,
    },
}

/// NG-07: per-request affinity inputs for endpoint selection.
///
/// `client_ip` is the *effective* client address — callers pass the leftmost
/// `X-Forwarded-For` token (real client behind PROXY protocol / trusted LB) or
/// the observed remote addr. `headers` carries raw request header pairs for
/// cookie/header affinity; `path_key` is path+query for `consistent_hash`.
#[derive(Debug, Default, Clone, Copy)]
pub struct AffinityCtx<'a> {
    pub client_ip: Option<&'a str>,
    pub headers: &'a [(String, String)],
    pub path_key: &'a str,
}

impl AffinityCtx<'_> {
    /// No request context — falls back to the base policy selection.
    pub const NONE: AffinityCtx<'static> = AffinityCtx {
        client_ip: None,
        headers: &[],
        path_key: "",
    };

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// First `name=value` pair across ALL `Cookie` request headers (a route
    /// cookie in a second Cookie header must not be invisible).
    fn cookie_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("cookie"))
            .find_map(|(_, v)| {
                v.split(';').map(str::trim).find_map(|pair| {
                    pair.strip_prefix(name)
                        .and_then(|rest| rest.strip_prefix('='))
                })
            })
    }
}

/// Leftmost XFF token (the client) — used as the `source_affinity` key.
fn client_ip_from_xff(xff: Option<&HeaderValue>) -> Option<&str> {
    xff.and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Bare IP key from `remote_addr` forms `ip`, `ip:port`, `[v6]`, `[v6]:port`.
fn client_ip_key(remote_addr: &str) -> &str {
    let s = remote_addr.trim();
    if let Some(rest) = s.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match s.rsplit_once(':') {
        Some((host, port)) if port.parse::<u16>().is_ok() && !host.contains(':') => host,
        _ => s,
    }
}

struct ClusterTable {
    generation: u64,
    /// Index = cluster_id.
    bindings: Box<[Option<ClusterBinding>]>,
    /// AP-01: upstream name → cluster_id, for the member control surface.
    upstream_index: HashMap<String, u32>,
    /// F3-R20: response Location rewrite rules per cluster (empty = none).
    location_rewrites: HashMap<u32, Arc<Vec<(String, String)>>>,
}

impl Default for ClusterTable {
    fn default() -> Self {
        Self {
            generation: 0,
            bindings: Box::new([]),
            upstream_index: HashMap::new(),
            location_rewrites: HashMap::new(),
        }
    }
}

/// AP-01: member row for `exyonqctl server list/set`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MemberEntry {
    pub upstream: String,
    pub endpoint_id: String,
    pub enabled: bool,
    pub weight: u32,
    /// Composite selectability now (admin gate + passive/health), informational.
    pub selectable: bool,
}

/// Module-owned proxy runtime.
pub struct ProxyRuntime {
    clusters: RwLock<ClusterTable>,
    metrics: Arc<ProxyHyperMetrics>,
    incoming_client: ProxyClient,
    responses_503: AtomicU64,
    /// Cap024 active health supervisor (generation-scoped probe tasks).
    health: HealthSupervisor,
    /// NG-09 periodic DNS re-resolve supervisor (generation-scoped).
    dns: DnsRefreshSupervisor,
}

impl Default for ProxyRuntime {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_METRICS: std::sync::OnceLock<Arc<ProxyHyperMetrics>> = std::sync::OnceLock::new();

pub fn global_hyper_metrics() -> Arc<ProxyHyperMetrics> {
    GLOBAL_METRICS
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(ProxyHyperMetrics::default()))
}

impl ProxyRuntime {
    pub fn new() -> Self {
        let metrics = Arc::new(ProxyHyperMetrics::default());
        let _ = GLOBAL_METRICS.set(Arc::clone(&metrics));
        Self {
            clusters: RwLock::new(ClusterTable::default()),
            metrics,
            incoming_client: build_incoming_client().clone(),
            responses_503: AtomicU64::new(0),
            health: HealthSupervisor::new(),
            dns: DnsRefreshSupervisor::new(),
        }
    }

    pub fn incoming_client(&self) -> ProxyClient {
        self.incoming_client.clone()
    }

    pub fn hyper_metrics(&self) -> Arc<ProxyHyperMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Cap024/NG-09: abort health probe + DNS refresh tasks (process shutdown /
    /// test teardown).
    /// AP-01: member rows for `exyonqctl server list`. `upstream = None` lists
    /// every upstream with a member table (deterministic upstream-name order).
    pub fn member_list(&self, upstream: Option<&str>) -> Result<Vec<MemberEntry>, String> {
        let clusters = self
            .clusters
            .read()
            .map_err(|_| "proxy clusters poisoned".to_string())?;
        let mut names: Vec<&String> = clusters.upstream_index.keys().collect();
        names.sort();
        let mut out = Vec::new();
        for name in names {
            if let Some(filter) = upstream {
                if name != filter {
                    continue;
                }
            }
            let cluster_id = clusters.upstream_index[name];
            match clusters
                .bindings
                .get(cluster_id as usize)
                .and_then(|b| b.as_ref())
            {
                Some(ClusterBinding::Multi {
                    selector,
                    targets_by_index,
                }) => {
                    for (i, m) in selector.member_list().into_iter().enumerate() {
                        let has_target = targets_by_index.get(i).is_some_and(|t| t.is_some());
                        out.push(MemberEntry {
                            upstream: name.clone(),
                            endpoint_id: m.endpoint_id,
                            enabled: m.enabled,
                            weight: m.weight,
                            // Unparseable upstream URI → member exists in the
                            // table but can never serve — report it.
                            selectable: m.selectable && has_target,
                        });
                    }
                }
                Some(ClusterBinding::Single {
                    target,
                    admin,
                    endpoint_id,
                }) => {
                    out.push(MemberEntry {
                        upstream: name.clone(),
                        endpoint_id: endpoint_id.clone(),
                        enabled: admin.enabled(),
                        weight: admin.weight(),
                        selectable: admin.enabled()
                            && admin.weight() > 0
                            && target.passive.selectable(),
                    });
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// AP-01: hot member mutation (`exyonqctl server set`). With `upstream`
    /// unset the endpoint_id must resolve to exactly one member table.
    pub fn member_set(
        &self,
        upstream: Option<&str>,
        endpoint_id: &str,
        enabled: Option<bool>,
        weight: Option<u32>,
    ) -> Result<MemberEntry, String> {
        let clusters = self
            .clusters
            .read()
            .map_err(|_| "proxy clusters poisoned".to_string())?;
        // Collect (upstream_name, binding) candidates whose table contains the id.
        enum Found<'a> {
            Multi {
                selector: &'a EndpointSelector,
                targets_by_index: &'a [Option<UpstreamTarget>],
            },
            Single {
                target: &'a UpstreamTarget,
                admin: &'a Arc<EndpointAdmin>,
            },
        }
        let mut hits: Vec<(&String, Found<'_>)> = Vec::new();
        for (name, &cid) in clusters
            .upstream_index
            .iter()
            .filter(|(name, _)| upstream.is_none_or(|u| name.as_str() == u))
        {
            match clusters.bindings.get(cid as usize).and_then(|b| b.as_ref()) {
                Some(ClusterBinding::Multi {
                    selector,
                    targets_by_index,
                }) if selector
                    .endpoints()
                    .iter()
                    .any(|e| e.endpoint_id == endpoint_id) =>
                {
                    hits.push((
                        name,
                        Found::Multi {
                            selector: selector.as_ref(),
                            targets_by_index,
                        },
                    ));
                }
                Some(ClusterBinding::Single {
                    target,
                    admin,
                    endpoint_id: id,
                }) if id == endpoint_id => {
                    hits.push((name, Found::Single { target, admin }));
                }
                _ => {}
            }
        }
        let [(name, found)] = hits.as_slice() else {
            return match hits.len() {
                0 => Err(format!("no member `{endpoint_id}`")),
                _ => Err(format!(
                    "member `{endpoint_id}` is ambiguous across upstreams: {}",
                    hits.iter()
                        .map(|(n, _)| n.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            };
        };
        let entry = match found {
            Found::Multi {
                selector,
                targets_by_index,
            } => {
                let m = selector
                    .member_set(endpoint_id, enabled, weight)
                    .expect("endpoint checked above");
                let has_target = selector
                    .endpoints()
                    .iter()
                    .position(|e| e.endpoint_id == endpoint_id)
                    .is_some_and(|i| targets_by_index.get(i).is_some_and(|t| t.is_some()));
                MemberEntry {
                    upstream: (*name).clone(),
                    endpoint_id: m.endpoint_id,
                    enabled: m.enabled,
                    weight: m.weight,
                    selectable: m.selectable && has_target,
                }
            }
            Found::Single { target, admin } => {
                if let Some(v) = enabled {
                    admin.set_enabled(v);
                }
                if let Some(v) = weight {
                    admin.set_weight(v);
                }
                MemberEntry {
                    upstream: (*name).clone(),
                    endpoint_id: endpoint_id.to_string(),
                    enabled: admin.enabled(),
                    weight: admin.weight(),
                    selectable: admin.enabled()
                        && admin.weight() > 0
                        && target.passive.selectable(),
                }
            }
        };
        Ok(entry)
    }

    pub fn shutdown_health(&self) {
        self.health.shutdown();
        self.dns.shutdown();
    }

    pub fn bind_compiled_slots(&self, generation: u64, slots: &[ProxyCompiledSlot]) {
        let mut prior: HashMap<String, UpstreamTarget> = HashMap::new();
        if let Ok(prev) = self.clusters.read() {
            for binding in prev.bindings.iter().flatten() {
                match binding {
                    ClusterBinding::Single { target, .. } => {
                        if let Some(k) = endpoint_transport_identity_from_target(target) {
                            prior.insert(k, (**target).clone());
                        }
                    }
                    ClusterBinding::Multi {
                        targets_by_index, ..
                    } => {
                        for t in targets_by_index.iter().flatten() {
                            if let Some(k) = endpoint_transport_identity_from_target(t) {
                                prior.insert(k, t.clone());
                            }
                        }
                    }
                    ClusterBinding::NoEligible => {}
                }
            }
        }

        let len = slots
            .iter()
            .map(|s| s.cluster_id as usize + 1)
            .max()
            .unwrap_or(0);
        let mut built: Vec<Option<ClusterBinding>> =
            (0..len.max(slots.len())).map(|_| None).collect();
        for slot in slots {
            let idx = slot.cluster_id as usize;
            if idx >= built.len() {
                built.extend((built.len()..=idx).map(|_| None));
            }
            built[idx] = Some(build_cluster_binding(generation, slot, &prior));
        }
        // Cap024: publish cluster bindings BEFORE health views so a reload that
        // removes a peer cannot race (old bindings + new health map fail-open).
        let mut clusters = self.clusters.write().expect("proxy clusters poisoned");
        clusters.generation = generation;
        clusters.upstream_index = slots
            .iter()
            .filter(|s| !s.upstream_name.is_empty())
            .map(|s| (s.upstream_name.clone(), s.cluster_id))
            .collect();
        clusters.bindings = built.into_boxed_slice();
        clusters.location_rewrites = slots
            .iter()
            .filter(|s| !s.location_rewrites.is_empty())
            .map(|s| (s.cluster_id, Arc::new(s.location_rewrites.clone())))
            .collect();
        drop(clusters);

        let mut health_clusters = Vec::new();
        for slot in slots {
            if !slot.health_check.enabled {
                continue;
            }
            let mut peers = Vec::new();
            for ep in slot.endpoints.iter() {
                if !(ep.admin_enabled && ep.weight > 0) {
                    continue;
                }
                if let Some(key) = peer_key_from_http_uri(&ep.http_uri) {
                    peers.push((key, ep.http_uri.clone()));
                }
            }
            if peers.is_empty() && !slot.target.is_empty() {
                if let Some(key) = peer_key_from_http_uri(&slot.target) {
                    peers.push((key, slot.target.clone()));
                }
            }
            health_clusters.push((
                slot.cluster_id,
                slot.upstream_name.clone(),
                slot.health_check.clone(),
                peers,
            ));
        }
        self.health.replace_generation(generation, &health_clusters);

        let mut dns_clusters: Vec<DnsClusterSpec> = Vec::new();
        for slot in slots {
            if slot.dns_refresh_ms == 0 {
                continue;
            }
            let mut peers: Vec<DnsPeer> = slot
                .endpoints
                .iter()
                .filter_map(|ep| {
                    crate::dns_refresh::dns_peer_from_http_uri(&slot.upstream_name, &ep.http_uri)
                })
                .collect();
            if peers.is_empty() && !slot.target.is_empty() {
                if let Some(peer) =
                    crate::dns_refresh::dns_peer_from_http_uri(&slot.upstream_name, &slot.target)
                {
                    peers.push(peer);
                }
            }
            if !peers.is_empty() {
                dns_clusters.push((
                    slot.cluster_id,
                    slot.upstream_name.clone(),
                    std::time::Duration::from_millis(slot.dns_refresh_ms),
                    peers,
                ));
            }
        }
        self.dns.replace_generation(&dns_clusters);
    }

    /// F3-R20: location rewrite rules for `cluster_id` (None when unconfigured).
    /// `pub(crate)`: wire_conn consults this to bypass the verbatim raw relay —
    /// a byte-level relay cannot rewrite response headers.
    pub(crate) fn location_rewrite_rules(
        &self,
        cluster_id: u32,
    ) -> Option<Arc<Vec<(String, String)>>> {
        self.clusters
            .read()
            .expect("proxy clusters poisoned")
            .location_rewrites
            .get(&cluster_id)
            .cloned()
    }

    pub fn bind_upstream_targets(&self, generation: u64, targets: &[UpstreamTarget]) {
        let mut clusters = self.clusters.write().expect("proxy clusters poisoned");
        clusters.generation = generation;
        clusters.upstream_index.clear();
        clusters.bindings = targets
            .iter()
            .cloned()
            .map(|t| {
                let admin = t.admin.clone();
                Some(ClusterBinding::Single {
                    endpoint_id: t.peer_key.clone(),
                    target: Box::new(t),
                    admin,
                })
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
    }

    pub fn upstream_for_cluster(&self, cluster_id: u32) -> Option<UpstreamTarget> {
        self.upstream_for_cluster_ctx(cluster_id, &AffinityCtx::NONE)
            .map(|sel| sel.target)
    }

    /// NG-07: whether the cluster's policy needs request headers (cookie/header
    /// affinity). Wire paths can skip header parsing otherwise.
    pub fn cluster_uses_header_affinity(&self, cluster_id: u32) -> bool {
        let Ok(table) = self.clusters.read() else {
            return false;
        };
        match table.bindings.get(cluster_id as usize) {
            Some(Some(ClusterBinding::Multi { selector, .. })) => matches!(
                selector.policy(),
                ProxySelectionPolicy::CookieAffinity | ProxySelectionPolicy::HeaderAffinity
            ),
            _ => false,
        }
    }

    /// NG-07: policies whose selection inputs vary per request (keyed hashes,
    /// cookie/header pins, live-load algorithms) must resolve on every request.
    /// WRR keeps its per-connection pin on the wire path; source_affinity keys
    /// on the connection-constant client IP, so per-connection == per-request.
    pub fn cluster_resolves_per_request(&self, cluster_id: u32) -> bool {
        let Ok(table) = self.clusters.read() else {
            return false;
        };
        match table.bindings.get(cluster_id as usize) {
            Some(Some(ClusterBinding::Multi { selector, .. })) => !matches!(
                selector.policy(),
                ProxySelectionPolicy::WeightedRoundRobin | ProxySelectionPolicy::SourceAffinity
            ),
            _ => false,
        }
    }

    /// NG-07: resolve honoring the cluster selection policy and request affinity.
    pub fn upstream_for_cluster_ctx(
        &self,
        cluster_id: u32,
        ctx: &AffinityCtx<'_>,
    ) -> Option<ResolveSelection> {
        match self.resolve_excluding(cluster_id, None, ctx) {
            Resolve::Selected(s) => Some(*s),
            Resolve::NoEligible | Resolve::Missing => None,
        }
    }

    pub fn cluster_generation(&self) -> u64 {
        self.clusters
            .read()
            .map(|table| table.generation)
            .unwrap_or(0)
    }

    fn resolve(&self, cluster_id: u32, ctx: &AffinityCtx<'_>) -> Resolve {
        self.resolve_excluding(cluster_id, None, ctx)
    }

    /// Cap021: on connect-only retry, skip the peer that just failed (request-local).
    /// Cap024: skip peers marked UNHEALTHY by active health (cross-request).
    fn resolve_excluding(
        &self,
        cluster_id: u32,
        exclude_peer: Option<&str>,
        ctx: &AffinityCtx<'_>,
    ) -> Resolve {
        let health = self.health.view(cluster_id);
        let peer_ok = |peer_key: &str| -> bool {
            health
                .as_ref()
                .map(|h| h.is_selectable(peer_key))
                .unwrap_or(true)
        };
        let Ok(table) = self.clusters.read() else {
            return Resolve::Missing;
        };
        match table.bindings.get(cluster_id as usize) {
            None | Some(None) => Resolve::Missing,
            Some(Some(ClusterBinding::NoEligible)) => Resolve::NoEligible,
            Some(Some(ClusterBinding::Single {
                target: t, admin, ..
            })) => {
                // AP-01: hot-disabled sole member → no live upstreams (503).
                if !admin.enabled() || admin.weight() == 0 {
                    return Resolve::NoEligible;
                }
                // NG-08: passively banned sole endpoint → no live upstreams.
                if !t.passive.selectable() || !peer_ok(&t.peer_key) {
                    return Resolve::NoEligible;
                }
                if exclude_peer.is_some_and(|k| k == t.peer_key) {
                    // Sole peer failed connect — still return it so final attempt can map 502.
                    // Caller stops retrying when may_retry is false / same peer only.
                }
                Resolve::Selected(Box::new(ResolveSelection {
                    target: (**t).clone(),
                    route_cookie: None,
                    lease: None,
                }))
            }
            Some(Some(ClusterBinding::Multi {
                selector,
                targets_by_index,
            })) => {
                // Cap045 / LA-CAP045-001: WRR among currently selectable peers only.
                // Do not charge SWWR on Cap024-unhealthy / Cap021-excluded peers, and
                // do not fall back to first-match scan among remaining healthy peers.
                let mut live = |endpoint_index: usize| {
                    let Some(t) = targets_by_index
                        .get(endpoint_index)
                        .and_then(|slot| slot.as_ref())
                    else {
                        return false;
                    };
                    if exclude_peer.is_some_and(|k| k == t.peer_key) {
                        return false;
                    }
                    peer_ok(&t.peer_key)
                };
                // NG-07: policy dispatch — affinity pin/hash first, algorithmic
                // load-aware policies mint an in-flight lease; unknown/empty
                // keys fall back to the WRR base inside the preferred band.
                let (outcome, lease) = match selector.policy() {
                    ProxySelectionPolicy::LeastConn => selector.select_least_conn(&mut live),
                    ProxySelectionPolicy::PowerOfTwo => selector.select_power_of_two(&mut live),
                    ProxySelectionPolicy::ConsistentHash => {
                        (selector.select_key_filtered(ctx.path_key, &mut live), None)
                    }
                    ProxySelectionPolicy::SourceAffinity => (
                        match ctx.client_ip {
                            Some(ip) => selector.select_key_filtered(ip, &mut live),
                            None => selector.select_filtered(&mut live),
                        },
                        None,
                    ),
                    ProxySelectionPolicy::HeaderAffinity => (
                        match selector.sticky_header().and_then(|name| ctx.header(name)) {
                            Some(key) => selector.select_key_filtered(key, &mut live),
                            None => selector.select_filtered(&mut live),
                        },
                        None,
                    ),
                    ProxySelectionPolicy::CookieAffinity => {
                        let pinned = selector
                            .sticky_cookie()
                            .and_then(|name| ctx.cookie_value(name))
                            .map(|id| selector.select_by_id_filtered(id, &mut live));
                        (
                            match pinned {
                                Some(s @ SelectionOutcome::Selected { .. }) => s,
                                _ => selector.select_filtered(&mut live),
                            },
                            None,
                        )
                    }
                    ProxySelectionPolicy::WeightedRoundRobin => {
                        (selector.select_filtered(&mut live), None)
                    }
                };
                match outcome {
                    SelectionOutcome::NoEligibleEndpoint => Resolve::NoEligible,
                    SelectionOutcome::Selected { endpoint_index, .. } => {
                        match targets_by_index.get(endpoint_index).and_then(|t| t.clone()) {
                            Some(t) => {
                                // Cookie affinity mints/refreshes the route token
                                // on every selection (first visit or re-pin).
                                let route_cookie = selector.sticky_cookie().and_then(|name| {
                                    (selector.policy() == ProxySelectionPolicy::CookieAffinity)
                                        .then(|| {
                                            let id =
                                                &selector.endpoints()[endpoint_index].endpoint_id;
                                            format!("{name}={id}; Path=/; HttpOnly")
                                        })
                                });
                                Resolve::Selected(Box::new(ResolveSelection {
                                    target: t,
                                    route_cookie,
                                    lease,
                                }))
                            }
                            None => Resolve::NoEligible,
                        }
                    }
                }
            }
        }
    }

    /// Default: do **not** trust inbound `X-Forwarded-For` — use peer `remote_addr` only.
    fn xff_from_request(request: &ProxyDispatchRequest) -> Option<HeaderValue> {
        HeaderValue::from_str(&request.remote_addr).ok()
    }

    fn xfp_from_request(request: &ProxyDispatchRequest) -> Option<HeaderValue> {
        if request.scheme.is_empty() {
            None
        } else {
            HeaderValue::from_str(&request.scheme).ok()
        }
    }

    fn hyper_method(method: ProxyMethod) -> Method {
        match method {
            ProxyMethod::Get => Method::GET,
            ProxyMethod::Head => Method::HEAD,
            ProxyMethod::Post => Method::POST,
            ProxyMethod::Put => Method::PUT,
            ProxyMethod::Patch => Method::PATCH,
            ProxyMethod::Delete => Method::DELETE,
            ProxyMethod::Options => Method::OPTIONS,
            ProxyMethod::Other => Method::GET,
        }
    }

    async fn attempt_get_like(
        &self,
        target: &UpstreamTarget,
        request: &ProxyDispatchRequest,
        budget: std::time::Duration,
    ) -> Result<ProxyDispatchOutcome, UpstreamAttemptClass> {
        let xff = Self::xff_from_request(request);
        let xfp = Self::xfp_from_request(request);
        let method = match request.method {
            ProxyMethod::Head => Method::HEAD,
            _ => Method::GET,
        };
        let mut response = crate::hyper_forward::attempt_forward_empty_body_with_headers(
            target,
            &request.path_and_query,
            method,
            xff.as_ref(),
            xfp.as_ref(),
            &request.headers,
            &self.metrics,
            budget,
        )
        .await?;
        // F3-R20: rewrite before classify — Streaming stores this same
        // header set, so the outcome and the stored response stay identical.
        if let Some(rules) = self.location_rewrite_rules(request.cluster_id) {
            rewrite_location_header_map(response.headers_mut(), &rules);
        }
        Ok(classify_hyper_response(response, &request.path_and_query, request.method).await)
    }

    async fn attempt_post_like(
        &self,
        target: &UpstreamTarget,
        request: &ProxyDispatchRequest,
        budget: std::time::Duration,
    ) -> Result<ProxyDispatchOutcome, UpstreamAttemptClass> {
        if budget.is_zero() {
            return Err(UpstreamAttemptClass::TimedOut);
        }
        let body = request.body.clone().unwrap_or_default();
        if body.len() > PROXY_MAX_REQUEST_BODY_BYTES {
            return Ok(ProxyDispatchOutcome::BadGateway);
        }

        let mut header_map = hyper::HeaderMap::new();
        for (name, value) in &request.headers {
            if name.eq_ignore_ascii_case("x-forwarded-for")
                || name.eq_ignore_ascii_case("x-forwarded-proto")
                || name.eq_ignore_ascii_case("x-forwarded-host")
                || name.eq_ignore_ascii_case("forwarded")
            {
                continue;
            }
            if let (Ok(n), Ok(v)) = (
                hyper::header::HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                header_map.insert(n, v);
            }
        }
        if !request_headers_safe_for_proxy(&header_map) {
            return Ok(ProxyDispatchOutcome::BadGateway);
        }
        // Strip on the full map *before* Request::builder: Hyper may drop Connection
        // during build, which would skip RFC 7230 nomination and leak hop fields.
        strip_hop_by_hop_headers(&mut header_map);

        let path_and_query = request.path_and_query.clone();
        let mut builder = Request::builder()
            .method(Self::hyper_method(request.method))
            .uri(target.uri_for(&path_and_query));

        for (name, value) in header_map.iter() {
            builder = builder.header(name, value);
        }
        if let Some(host) = &request.host {
            builder = builder.header("host", host.as_str());
        } else if let Some(host) = &target.host {
            builder = builder.header(HOST, host.clone());
        }

        let mut req = match builder.body(Full::from(body)) {
            Ok(req) => req,
            Err(err) => {
                warn!(%err, "proxy request builder failed (operator/network headers)");
                return Ok(ProxyDispatchOutcome::BadGateway);
            }
        };
        {
            let headers = req.headers_mut();
            // Re-inject intermediary identity after strip so Connection cannot wipe them.
            if let Some(host) = &request.host {
                if let Ok(v) = HeaderValue::from_str(host.as_str()) {
                    headers.insert(HOST, v);
                }
            } else if let Some(host) = &target.host {
                headers.insert(HOST, host.clone());
            }
            if let Some(xff) = Self::xff_from_request(request) {
                headers.insert("x-forwarded-for", xff);
            }
            if let Some(xfp) = Self::xfp_from_request(request) {
                headers.insert("x-forwarded-proto", xfp);
            }
        }

        let upstream_req = post_body_client().request(req);
        match tokio::time::timeout(budget, upstream_req).await {
            Ok(Ok(resp)) => {
                // NG-08: upstream responded — passive health success.
                target.passive.record(true);
                use http_body_util::BodyExt;
                let (mut parts, body) = resp.into_parts();
                strip_hop_by_hop_headers(&mut parts.headers);
                if let Some(rules) = self.location_rewrite_rules(request.cluster_id) {
                    rewrite_location_header_map(&mut parts.headers, &rules);
                }
                let response: hyper::Response<
                    http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>,
                > = hyper::Response::from_parts(parts, body.boxed());
                Ok(classify_hyper_response(response, &path_and_query, request.method).await)
            }
            Ok(Err(err)) => {
                let class = classify_hyper_error(&err);
                warn!(%err, ?class, "upstream error");
                // NG-08: transport failure against this peer.
                target.passive.record(false);
                Err(class)
            }
            Err(_) => {
                warn!(budget_ms = budget.as_millis(), "upstream attempt timeout");
                target.passive.record(false);
                Err(UpstreamAttemptClass::TimedOut)
            }
        }
    }

    fn outcome_for_class(
        &self,
        class: UpstreamAttemptClass,
        request: &ProxyDispatchRequest,
        peer: Option<&str>,
        detail: Option<&str>,
    ) -> ProxyDispatchOutcome {
        emit_upstream_class(
            &RtFailCtx {
                cluster_id: request.cluster_id,
                path: &request.path_and_query,
                peer,
                host: request.host.as_deref(),
                detail,
            },
            class,
        );
        match class {
            UpstreamAttemptClass::TimedOut => {
                self.metrics.responses_504.fetch_add(1, Ordering::Relaxed);
                ProxyDispatchOutcome::GatewayTimeout
            }
            UpstreamAttemptClass::ConnectFailed | UpstreamAttemptClass::UnretryableError => {
                self.metrics.responses_502.fetch_add(1, Ordering::Relaxed);
                ProxyDispatchOutcome::BadGateway
            }
        }
    }

    /// Forward a live Hyper request (WebSocket upgrade path — preserves `Incoming` body).
    pub async fn forward_incoming_request(
        &self,
        target: &UpstreamTarget,
        req: Request<Incoming>,
        x_forwarded_for: Option<&HeaderValue>,
    ) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>> {
        forward_request(
            &self.incoming_client,
            target,
            req,
            x_forwarded_for,
            &self.metrics,
        )
        .await
    }
}

enum Resolve {
    Selected(Box<ResolveSelection>),
    NoEligible,
    Missing,
}

/// NG-07: resolved upstream plus affinity/lease metadata for the caller.
pub struct ResolveSelection {
    pub target: UpstreamTarget,
    /// `Set-Cookie` value to append on the client response (`cookie_affinity`).
    /// Never part of the upstream response — keep it out of cached headers.
    pub route_cookie: Option<String>,
    /// least_conn/P2C in-flight lease; hold until the exchange completes.
    pub lease: Option<InflightLease>,
}

// F3-R20: ProxyPassReverse-equivalent — response `Location`, `Content-Location`
// and `URI` header values whose prefix matches a rule's `from` get that prefix
// replaced by `to` (first matching rule wins).

/// Length of `scheme://authority` in `s` (authority ends at `/`, `?` or `#`).
fn url_authority_end(s: &str) -> Option<usize> {
    let i = s.find("://")? + 3;
    Some(i + s[i..].find(['/', '?', '#']).unwrap_or(s.len() - i))
}

/// Prefix-replace `value` when it matches `from` (scheme+authority compared
/// case-insensitively per RFC 3986 §3.1/§3.2; the remainder case-sensitively).
fn rewrite_prefix_match(value: &str, from: &str, to: &str) -> Option<String> {
    let f_auth = url_authority_end(from)?;
    let v_auth = url_authority_end(value)?;
    if !value[..v_auth].eq_ignore_ascii_case(&from[..f_auth]) {
        return None;
    }
    let (v_rest, f_rest) = (&value[v_auth..], &from[f_auth..]);
    v_rest
        .starts_with(f_rest)
        .then(|| format!("{to}{}", &v_rest[f_rest.len()..]))
}

fn rewrite_location_header_map(headers: &mut hyper::HeaderMap, rules: &[(String, String)]) {
    for key in [
        hyper::header::LOCATION,
        hyper::header::CONTENT_LOCATION,
        hyper::header::HeaderName::from_static("uri"),
    ] {
        let mut replaced: Vec<HeaderValue> = Vec::new();
        let mut any = false;
        for v in headers.get_all(&key).iter() {
            match v.to_str().ok().and_then(|s| {
                rules
                    .iter()
                    .find_map(|(f, t)| rewrite_prefix_match(s, f, t))
            }) {
                Some(s) => {
                    any = true;
                    replaced.push(HeaderValue::from_str(&s).unwrap_or_else(|_| v.clone()));
                }
                // Keep untouched values (incl. non-UTF8) byte-exact.
                None => replaced.push(v.clone()),
            }
        }
        if !any {
            continue;
        }
        headers.remove(&key);
        for v in replaced {
            headers.append(&key, v);
        }
    }
}

/// NG-07: attach the minted route cookie to a dispatch outcome (client-facing
/// headers only — upstream/cached headers never see it).
fn append_route_cookie(outcome: &mut ProxyDispatchOutcome, route_cookie: Option<&str>) {
    let Some(value) = route_cookie else { return };
    // NG-07: never pin the client on a failed upstream response.
    let headers = match outcome {
        ProxyDispatchOutcome::Materialized(m) if m.status < 500 => &mut m.headers,
        ProxyDispatchOutcome::Streaming {
            status, headers, ..
        } if *status < 500 => headers,
        _ => return,
    };
    headers.push(("set-cookie".to_string(), value.to_string()));
}

fn endpoint_transport_identity_from_target(t: &UpstreamTarget) -> Option<String> {
    let uri = t.uri_for("/");
    let scheme = uri.scheme_str().unwrap_or("http");
    let host = uri.host()?;
    let port = uri
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some(format!("{scheme}://{host}:{port}"))
}

fn build_cluster_binding(
    generation: u64,
    slot: &ProxyCompiledSlot,
    prior: &HashMap<String, UpstreamTarget>,
) -> ClusterBinding {
    if slot.single_endpoint_executable && !slot.target.is_empty() {
        // NG-08: nginx ignores max_fails/fail_timeout/slow_start when a group
        // has a single server ("such a server never becomes permanently
        // unavailable") — the sole-endpoint binding never bans.
        let peer = Arc::new(PassivePeer::disabled());
        // AP-01: recover the real member identity — slot.endpoints retains the
        // full table even for single-endpoint execution (SILENT_ENDPOINT_TRUNCATION=NO).
        // Fallback: the target URI is the member id (Apache BalancerMember form).
        let (endpoint_id, admin) = {
            let eligible: Vec<&exyonq_module_api::proxy_dispatch::ProxyCompiledEndpoint> = slot
                .endpoints
                .iter()
                .filter(|e| e.admin_enabled && e.weight > 0)
                .collect();
            match eligible.as_slice() {
                [ep] => (
                    ep.endpoint_id.clone(),
                    Arc::new(EndpointAdmin::new(ep.admin_enabled, ep.weight)),
                ),
                _ => (slot.target.clone(), Arc::new(EndpointAdmin::new(true, 1))),
            }
        };
        let key = endpoint_transport_identity(&slot.target);
        if let Some(key) = key.as_ref() {
            if let Some(existing) = prior.get(key) {
                let mut reused = existing.clone();
                reused.timeout = slot.timeout;
                reused.max_connect_retries = slot.max_connect_retries;
                reused.passive = peer;
                reused.admin = Arc::clone(&admin);
                return ClusterBinding::Single {
                    target: Box::new(reused),
                    admin,
                    endpoint_id,
                };
            }
        }
        let host = slot
            .target
            .parse::<Uri>()
            .ok()
            .and_then(|u| u.host().map(str::to_string));
        let desc = UpstreamDescriptor {
            cluster_id: slot.cluster_id,
            upstream_name: slot.upstream_name.clone(),
            target: slot.target.clone(),
            timeout: slot.timeout,
            host,
            max_connect_retries: slot.max_connect_retries,
        };
        return match UpstreamTarget::from_descriptor(&desc) {
            Ok(mut target) => {
                target.passive = peer;
                target.admin = Arc::clone(&admin);
                ClusterBinding::Single {
                    target: Box::new(target),
                    admin,
                    endpoint_id,
                }
            }
            Err(err) => {
                warn!(
                    %err,
                    cluster_id = slot.cluster_id,
                    target = %slot.target,
                    "compiled proxy slot failed to parse; binding NoEligible"
                );
                ClusterBinding::NoEligible
            }
        };
    }

    if slot.multi_endpoint_executable && !slot.endpoints.is_empty() {
        // NG-08: one PassivePeer per endpoint — the Arc is shared between the
        // selector spec (ban gate + slow_start ramp) and the runtime target
        // (transport paths record attempt outcomes into it).
        let peers: Vec<Arc<PassivePeer>> = slot
            .endpoints
            .iter()
            .map(|ep| {
                Arc::new(PassivePeer::new(
                    ep.max_fails,
                    ep.fail_timeout_ms,
                    ep.slow_start_ms,
                ))
            })
            .collect();
        let specs: Vec<EndpointSpec> = slot
            .endpoints
            .iter()
            .enumerate()
            .map(|(i, ep)| EndpointSpec {
                endpoint_id: ep.endpoint_id.clone(),
                http_uri: ep.http_uri.clone(),
                weight: ep.weight,
                priority: ep.priority,
                admin_enabled: ep.admin_enabled,
                passive: Arc::clone(&peers[i]),
                admin: Arc::new(EndpointAdmin::new(ep.admin_enabled, ep.weight)),
            })
            .collect();
        let failover = if slot.failover_priority_bands {
            FailoverMode::PriorityBands
        } else {
            FailoverMode::None
        };
        // Kept beside the moved specs so the N=1 Single collapse can reuse the
        // same admin identity the member table would have had.
        let spec_admins: Vec<(String, Arc<EndpointAdmin>)> = specs
            .iter()
            .map(|s| (s.endpoint_id.clone(), Arc::clone(&s.admin)))
            .collect();
        let selector = Arc::new(EndpointSelector::build_policy(
            generation,
            failover,
            specs,
            slot.selection_policy,
            slot.sticky_cookie.clone(),
            slot.sticky_header.clone(),
        ));
        let mut targets_by_index: Vec<Option<UpstreamTarget>> = vec![None; slot.endpoints.len()];
        let mut any = false;
        let mut sole: Option<(UpstreamTarget, usize)> = None;
        let mut eligible_count = 0usize;
        for (i, ep) in slot.endpoints.iter().enumerate() {
            let eligible = ep.admin_enabled && ep.weight > 0;
            let Some(key) = endpoint_transport_identity(&ep.http_uri) else {
                continue;
            };
            let mut target = if let Some(existing) = prior.get(&key) {
                let mut reused = existing.clone();
                reused.timeout = slot.timeout;
                reused.max_connect_retries = slot.max_connect_retries;
                reused
            } else {
                let host = ep
                    .http_uri
                    .parse::<Uri>()
                    .ok()
                    .and_then(|u| u.host().map(str::to_string));
                let desc = UpstreamDescriptor {
                    cluster_id: slot.cluster_id,
                    upstream_name: slot.upstream_name.clone(),
                    target: ep.http_uri.clone(),
                    timeout: slot.timeout,
                    host,
                    max_connect_retries: slot.max_connect_retries,
                };
                match UpstreamTarget::from_descriptor(&desc) {
                    Ok(t) => t,
                    Err(_) => continue,
                }
            };
            target.passive = Arc::clone(&peers[i]);
            target.admin = Arc::clone(&spec_admins[i].1);
            // AP-01: statically-ineligible members keep their target so
            // `member set` can enable them without a reload (BalancerMember).
            targets_by_index[i] = Some(target.clone());
            any = true;
            if eligible {
                eligible_count += 1;
                sole = Some((target, i));
            }
        }
        if !any {
            return ClusterBinding::NoEligible;
        }
        // OPEN-002 defense-in-depth: effective N=1 uses Single even if multi flag set.
        if eligible_count == 1 {
            // Structural: eligible_count==1 implies `sole` was set in the loop above.
            return match sole {
                Some((mut target, i)) => {
                    // NG-08: same nginx semantics as the single-server path —
                    // a lone eligible endpoint is attempted, never banned.
                    target.passive = Arc::new(PassivePeer::disabled());
                    let (endpoint_id, admin) = spec_admins[i].clone();
                    target.admin = Arc::clone(&admin);
                    ClusterBinding::Single {
                        target: Box::new(target),
                        admin,
                        endpoint_id,
                    }
                }
                None => ClusterBinding::NoEligible,
            };
        }
        return ClusterBinding::Multi {
            selector,
            targets_by_index: targets_by_index.into_boxed_slice(),
        };
    }

    // SILENT_ENDPOINT_TRUNCATION = NO / FIRST_ENDPOINT_FALLBACK = FORBIDDEN
    ClusterBinding::NoEligible
}

#[async_trait]
impl ProxyDispatchService for ProxyRuntime {
    async fn dispatch(&self, request: ProxyDispatchRequest) -> ProxyDispatchOutcome {
        // NG-07: affinity context — remote_addr for source, headers for
        // cookie/header pins, path+query for consistent_hash.
        let client_ip = client_ip_key(&request.remote_addr);
        let ctx = AffinityCtx {
            client_ip: (!client_ip.is_empty()).then_some(client_ip),
            headers: &request.headers,
            path_key: &request.path_and_query,
        };
        let first = match self.resolve(request.cluster_id, &ctx) {
            Resolve::Selected(s) => s,
            Resolve::NoEligible | Resolve::Missing => {
                self.responses_503.fetch_add(1, Ordering::Relaxed);
                emit_no_eligible(&RtFailCtx {
                    cluster_id: request.cluster_id,
                    path: &request.path_and_query,
                    peer: None,
                    host: request.host.as_deref(),
                    detail: None,
                });
                return ProxyDispatchOutcome::ServiceUnavailable;
            }
        };
        let total_timeout =
            upstream_timeout_for_path(first.target.timeout, &request.path_and_query);
        let deadline = shared_deadline(total_timeout);
        let max_retries = first.target.max_connect_retries;
        let mut retries_used: u8 = 0;
        let mut target = first.target;
        // NG-07: cookie_affinity mints once per resolved selection; refresh on retry.
        let mut route_cookie = first.route_cookie;
        // NG-07: least_conn/P2C lease — held until the outcome is produced
        // (streaming handoff documented: the lease covers connect+headers).
        let mut _lease = first.lease;

        loop {
            let budget = remaining_budget(deadline);
            // timeout_ms = 0 is already elapsed. Do not poll the upstream:
            // a zero tokio timeout can still observe a ready localhost future.
            if budget.is_zero() {
                return self.outcome_for_class(
                    UpstreamAttemptClass::TimedOut,
                    &request,
                    Some(target.peer_key.as_str()),
                    None,
                );
            }
            let attempt = match request.method {
                ProxyMethod::Get | ProxyMethod::Head => {
                    self.attempt_get_like(&target, &request, budget).await
                }
                ProxyMethod::Post
                | ProxyMethod::Put
                | ProxyMethod::Patch
                | ProxyMethod::Delete
                | ProxyMethod::Options
                | ProxyMethod::Other => self.attempt_post_like(&target, &request, budget).await,
            };
            match attempt {
                Ok(mut outcome) => {
                    append_route_cookie(&mut outcome, route_cookie.as_deref());
                    return outcome;
                }
                Err(class) => {
                    let remain = remaining_budget(deadline);
                    if may_retry_connect(max_retries, retries_used, class, remain) {
                        let failed_peer = target.peer_key.clone();
                        let next = match self.resolve_excluding(
                            request.cluster_id,
                            Some(failed_peer.as_str()),
                            &ctx,
                        ) {
                            Resolve::Selected(s) => s,
                            Resolve::NoEligible | Resolve::Missing => {
                                return self.outcome_for_class(
                                    class,
                                    &request,
                                    Some(failed_peer.as_str()),
                                    None,
                                );
                            }
                        };
                        target = next.target;
                        route_cookie = next.route_cookie;
                        _lease = next.lease;
                        note_connect_retry(&self.metrics);
                        retries_used = retries_used.saturating_add(1);
                        warn!(
                            retries_used,
                            peer = %failed_peer,
                            "cap021 connect-only retry"
                        );
                        continue;
                    }
                    return self.outcome_for_class(
                        class,
                        &request,
                        Some(target.peer_key.as_str()),
                        None,
                    );
                }
            }
        }
    }

    fn metrics(&self) -> ProxyMetricsSnapshot {
        ProxyMetricsSnapshot {
            responses_502: self.metrics.responses_502.load(Ordering::Relaxed),
            responses_503: self.responses_503.load(Ordering::Relaxed),
            responses_504: self.metrics.responses_504.load(Ordering::Relaxed),
            connect_retries: self.metrics.connect_retries.load(Ordering::Relaxed),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    fn bind_compiled_slots(&self, generation: u64, slots: &[ProxyCompiledSlot]) {
        ProxyRuntime::bind_compiled_slots(self, generation, slots);
    }
}

impl ProxyRuntime {
    /// Streaming GET for Plan 12 proxy cache load (returns live Hyper response).
    ///
    /// NG-07: resolves with affinity; returns the route cookie separately so it
    /// can reach the client response without ever entering cached headers.
    /// The least_conn/P2C lease lives inside the response body until drained.
    pub async fn forward_get_streaming_for_cache(
        &self,
        cluster_id: u32,
        path_and_query: &str,
        x_forwarded_for: Option<&HeaderValue>,
        request_headers: &[(String, String)],
    ) -> Option<(
        hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>>,
        Option<String>,
    )> {
        let ctx = AffinityCtx {
            client_ip: client_ip_from_xff(x_forwarded_for),
            headers: request_headers,
            path_key: path_and_query,
        };
        let sel = self.upstream_for_cluster_ctx(cluster_id, &ctx)?;
        let mut resp = forward_get_streaming(
            &sel.target,
            path_and_query,
            x_forwarded_for,
            &self.metrics,
            request_headers,
        )
        .await;
        if let Some(rules) = self.location_rewrite_rules(cluster_id) {
            rewrite_location_header_map(resp.headers_mut(), &rules);
        }
        if let Some(lease) = sel.lease {
            resp = resp.map(|b| GuardedBody::new(b, lease).boxed());
        }
        Some((resp, sel.route_cookie))
    }
}

/// Cache-eligible GET load by compiled cluster id (KD3.6 — no UpstreamTarget in core).
/// Returns the load plus an optional route cookie for the client response.
pub async fn load_get_for_cache_by_cluster(
    cluster_id: u32,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    max_object_bytes: usize,
    request_headers: &[(String, String)],
) -> (crate::proxy_cache::ProxyCacheLoad, Option<String>) {
    use crate::proxy_cache::ProxyCacheLoad;
    use exyonq_module_api::CacheRejection;

    let rt = crate::wire_conn::pinned_runtime();
    let Some((response, route_cookie)) = rt
        .forward_get_streaming_for_cache(
            cluster_id,
            path_and_query,
            x_forwarded_for,
            request_headers,
        )
        .await
    else {
        return (
            ProxyCacheLoad::Passthrough {
                response: crate::hyper_forward::service_unavailable(),
                rejection: CacheRejection::BodyNotMaterialized,
            },
            None,
        );
    };
    (
        crate::proxy_cache::prepare_proxy_cache_load(response, max_object_bytes, request_headers)
            .await,
        route_cookie,
    )
}

/// WebSocket upgrade by compiled cluster id (KD3.6 — core passes cluster_id only).
pub async fn forward_websocket_by_cluster(
    client: &ProxyClient,
    cluster_id: u32,
    req: hyper::Request<hyper::body::Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>> {
    let rt = crate::wire_conn::pinned_runtime();
    let req_headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter_map(|(n, v)| {
            v.to_str()
                .ok()
                .map(|v| (n.as_str().to_string(), v.to_string()))
        })
        .collect();
    let ctx = AffinityCtx {
        client_ip: client_ip_from_xff(x_forwarded_for),
        headers: &req_headers,
        path_key: req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or_else(|| req.uri().path()),
    };
    let Some(sel) = rt.upstream_for_cluster_ctx(cluster_id, &ctx) else {
        emit_no_eligible(&RtFailCtx {
            cluster_id,
            path: req.uri().path(),
            peer: None,
            host: None,
            detail: None,
        });
        return crate::hyper_forward::service_unavailable();
    };
    let mut resp = crate::websocket::forward_websocket(
        client,
        &sel.target,
        req,
        x_forwarded_for,
        rt.hyper_metrics().as_ref(),
    )
    .await;
    // NG-07: mint the route cookie on successful non-upgrade responses only.
    if let Some(cookie) = sel.route_cookie {
        let status = resp.status();
        if status != hyper::StatusCode::SWITCHING_PROTOCOLS && status.as_u16() < 500 {
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                resp.headers_mut().append(hyper::header::SET_COOKIE, v);
            }
        }
    }
    if let Some(lease) = sel.lease {
        resp = resp.map(|b| GuardedBody::new(b, lease).boxed());
    }
    resp
}

impl ProxyRuntime {
    pub async fn forward_get_for_wire(
        &self,
        cluster_id: u32,
        target: &UpstreamTarget,
        path_and_query: &str,
        x_forwarded_for: Option<&HeaderValue>,
        ctx: &AffinityCtx<'_>,
    ) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>> {
        // Cap021: wire GET shares connect-only retry + peer exclusion + deadline with dispatch.
        let total = upstream_timeout_for_path(target.timeout, path_and_query);
        let deadline = shared_deadline(total);
        let max_retries = target.max_connect_retries;
        let mut retries_used = 0u8;
        let location_rewrites = self.location_rewrite_rules(cluster_id);

        // Happy path: borrow `target` — no UpstreamTarget/HeaderValue clone until a connect retry.
        let budget = remaining_budget(deadline);
        match attempt_forward_empty_body(
            target,
            path_and_query,
            Method::GET,
            x_forwarded_for,
            None,
            &self.metrics,
            budget,
        )
        .await
        {
            Ok(mut resp) => {
                if let Some(rules) = &location_rewrites {
                    rewrite_location_header_map(resp.headers_mut(), rules);
                }
                resp
            }
            Err(class) => {
                let remain = remaining_budget(deadline);
                if !may_retry_connect(max_retries, retries_used, class, remain) {
                    let ctx = RtFailCtx {
                        cluster_id,
                        path: path_and_query,
                        peer: Some(target.peer_key.as_str()),
                        host: None,
                        detail: None,
                    };
                    return response_for_attempt_class_ctx(class, &self.metrics, Some(&ctx));
                }
                let failed_peer = target.peer_key.clone();
                let sel = match self.resolve_excluding(cluster_id, Some(failed_peer.as_str()), ctx)
                {
                    Resolve::Selected(s) => s,
                    Resolve::NoEligible | Resolve::Missing => {
                        let ctx = RtFailCtx {
                            cluster_id,
                            path: path_and_query,
                            peer: Some(failed_peer.as_str()),
                            host: None,
                            detail: None,
                        };
                        return response_for_attempt_class_ctx(class, &self.metrics, Some(&ctx));
                    }
                };
                let mut current = sel.target;
                let mut route_cookie = sel.route_cookie;
                let mut _retry_lease = sel.lease;
                note_connect_retry(&self.metrics);
                retries_used = retries_used.saturating_add(1);
                warn!(
                    retries_used,
                    peer = %failed_peer,
                    "cap021 connect-only retry"
                );

                loop {
                    let budget = remaining_budget(deadline);
                    match attempt_forward_empty_body(
                        &current,
                        path_and_query,
                        Method::GET,
                        x_forwarded_for,
                        None,
                        &self.metrics,
                        budget,
                    )
                    .await
                    {
                        Ok(mut resp) => {
                            // NG-07: retry mints the new endpoint's token only
                            // on a successful upstream response.
                            if resp.status().as_u16() < 500 {
                                if let Some(cookie) = route_cookie.take() {
                                    if let Ok(v) = HeaderValue::from_str(&cookie) {
                                        resp.headers_mut().append(hyper::header::SET_COOKIE, v);
                                    }
                                }
                            }
                            if let Some(rules) = &location_rewrites {
                                rewrite_location_header_map(resp.headers_mut(), rules);
                            }
                            return resp;
                        }
                        Err(class) => {
                            let remain = remaining_budget(deadline);
                            if may_retry_connect(max_retries, retries_used, class, remain) {
                                let failed_peer = current.peer_key.clone();
                                let sel = match self.resolve_excluding(
                                    cluster_id,
                                    Some(failed_peer.as_str()),
                                    ctx,
                                ) {
                                    Resolve::Selected(s) => s,
                                    Resolve::NoEligible | Resolve::Missing => {
                                        let ctx = RtFailCtx {
                                            cluster_id,
                                            path: path_and_query,
                                            peer: Some(failed_peer.as_str()),
                                            host: None,
                                            detail: None,
                                        };
                                        return response_for_attempt_class_ctx(
                                            class,
                                            &self.metrics,
                                            Some(&ctx),
                                        );
                                    }
                                };
                                current = sel.target;
                                route_cookie = sel.route_cookie;
                                _retry_lease = sel.lease;
                                note_connect_retry(&self.metrics);
                                retries_used = retries_used.saturating_add(1);
                                warn!(
                                    retries_used,
                                    peer = %failed_peer,
                                    "cap021 connect-only retry"
                                );
                                continue;
                            }
                            let ctx = RtFailCtx {
                                cluster_id,
                                path: path_and_query,
                                peer: Some(current.peer_key.as_str()),
                                host: None,
                                detail: None,
                            };
                            return response_for_attempt_class_ctx(
                                class,
                                &self.metrics,
                                Some(&ctx),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// NG-07: response body wrapper holding a least_conn/P2C in-flight lease until
/// the body is fully streamed or dropped — upstream work stays counted for the
/// real exchange duration on client-facing paths.
struct GuardedBody<B> {
    inner: B,
    _lease: Option<InflightLease>,
}

impl<B> GuardedBody<B> {
    fn new(inner: B, lease: InflightLease) -> Self {
        Self {
            inner,
            _lease: Some(lease),
        }
    }
}

impl<B> http_body::Body for GuardedBody<B>
where
    B: http_body::Body<Data = bytes::Bytes> + Unpin,
    B::Error: Into<hyper::Error>,
{
    type Data = bytes::Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        std::pin::Pin::new(&mut self.inner)
            .poll_frame(cx)
            .map(|f| f.map(|r| r.map_err(Into::into)))
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod location_rewrite_tests {
    use super::{rewrite_location_header_map, rewrite_prefix_match};
    use hyper::header::HeaderValue;
    use hyper::HeaderMap;

    #[test]
    fn prefix_match_rewrites_scheme_authority_case_insensitive() {
        let r = rewrite_prefix_match("HTTP://BE.Local/x?y=1", "http://be.local/", "/app/");
        assert_eq!(r.as_deref(), Some("/app/x?y=1"));
    }

    #[test]
    fn prefix_match_no_authority_boundary_confusion() {
        // `http://be` must not rewrite `http://be.local/` or `http://be:8443/`.
        assert_eq!(
            rewrite_prefix_match("http://be.local/x", "http://be", "/p"),
            None
        );
        assert_eq!(
            rewrite_prefix_match("http://be:8443/x", "http://be", "/p"),
            None
        );
        assert_eq!(
            rewrite_prefix_match("http://be/x", "http://be", "/p").as_deref(),
            Some("/p/x")
        );
    }

    #[test]
    fn prefix_match_path_is_case_sensitive() {
        assert_eq!(
            rewrite_prefix_match("http://be/Admin", "http://be/admin", "/a/"),
            None
        );
    }

    #[test]
    fn header_map_rewrites_and_preserves_others() {
        let mut h = HeaderMap::new();
        h.insert(
            hyper::header::LOCATION,
            HeaderValue::from_static("http://be/x"),
        );
        h.insert(
            hyper::header::CONTENT_LOCATION,
            HeaderValue::from_static("http://be/y"),
        );
        h.insert("x-other", HeaderValue::from_static("keep"));
        let rules = vec![("http://be/".to_string(), "/public/".to_string())];
        rewrite_location_header_map(&mut h, &rules);
        assert_eq!(h["location"], "/public/x");
        assert_eq!(h["content-location"], "/public/y");
        assert_eq!(h["x-other"], "keep");
    }

    #[test]
    fn header_map_unmatched_values_stay() {
        let mut h = HeaderMap::new();
        h.insert(
            hyper::header::LOCATION,
            HeaderValue::from_static("http://other/x"),
        );
        let rules = vec![("http://be/".to_string(), "/public/".to_string())];
        rewrite_location_header_map(&mut h, &rules);
        assert_eq!(h["location"], "http://other/x");
    }
}
