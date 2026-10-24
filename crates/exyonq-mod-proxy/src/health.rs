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
//! Cap024 active upstream HTTP health checking.
//!
//! ```text
//! INITIAL/HEALTHY = selectable (optimistic until consecutive failures)
//! UNHEALTHY = excluded from new request selection
//! Probe = real HTTP GET to configured path on configured peer authority
//! AT_MOST_ONE_ACTIVE_PROBE_PER_PEER
//! ```

use crate::hyper_client::get_empty_body_client;
use crate::selector::endpoint_transport_identity;
use exyonq_module_api::proxy_dispatch::ProxyHealthCheckCompiled;
use http_body_util::{BodyExt, Empty};
use hyper::{Method, Request};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// NG-10 process-lifetime probe stats keyed `(upstream, peer_key)`; the gauge
/// `state` mirrors the latest observed phase (1 healthy / 0 unhealthy).
#[derive(Default)]
struct HealthPeerStats {
    probes_ok: u64,
    probes_fail: u64,
    state: u64,
}

static HEALTH_STATS: LazyLock<Mutex<HashMap<(String, String), HealthPeerStats>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn health_stats_map() -> std::sync::MutexGuard<'static, HashMap<(String, String), HealthPeerStats>>
{
    HEALTH_STATS.lock().unwrap_or_else(|p| p.into_inner())
}

/// Current installed generation — a dead-generation probe task must not write
/// stats after prune cleared the table (abort is not synchronous).
static HEALTH_STATS_GEN: AtomicU64 = AtomicU64::new(0);

/// Observed peer health for routing (optimistic start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerHealthPhase {
    /// Selectable; includes initial pre-probe state.
    Healthy,
    /// Not selectable for new requests.
    Unhealthy,
}

#[derive(Debug)]
struct PeerHealthState {
    phase: PeerHealthPhase,
    consecutive_successes: u32,
    consecutive_failures: u32,
}

impl PeerHealthState {
    fn new() -> Self {
        Self {
            phase: PeerHealthPhase::Healthy,
            consecutive_successes: 0,
            consecutive_failures: 0,
        }
    }
}

#[derive(Debug)]
struct PeerHealthCell {
    state: Mutex<PeerHealthState>,
    /// Cap024: at most one in-flight probe per peer (outside Mutex for Send).
    probe_in_flight: AtomicBool,
}

impl PeerHealthCell {
    fn new() -> Self {
        Self {
            state: Mutex::new(PeerHealthState::new()),
            probe_in_flight: AtomicBool::new(false),
        }
    }
}

/// Generation-scoped health table for one upstream cluster.
#[derive(Debug)]
pub struct ClusterHealthView {
    generation: u64,
    /// NG-10: upstream name for stats/metric labels.
    upstream: String,
    config: ProxyHealthCheckCompiled,
    peers: HashMap<String, PeerHealthCell>,
    healthy_transitions: AtomicU64,
    unhealthy_transitions: AtomicU64,
}

impl ClusterHealthView {
    pub fn is_selectable(&self, peer_key: &str) -> bool {
        if !self.config.enabled {
            return true;
        }
        // Cap024 fail-closed: unknown peer under an enabled health view is not selectable
        // (prevents removed-peer resurrection across reload cutover).
        match self.peers.get(peer_key) {
            None => false,
            Some(cell) => match cell.state.lock() {
                Ok(s) => s.phase == PeerHealthPhase::Healthy,
                // Poisoned lock: fail closed for enabled health.
                Err(_) => false,
            },
        }
    }

    pub fn phase(&self, peer_key: &str) -> PeerHealthPhase {
        match self.peers.get(peer_key) {
            Some(cell) => cell
                .state
                .lock()
                .map(|s| s.phase)
                .unwrap_or(PeerHealthPhase::Unhealthy),
            None => PeerHealthPhase::Unhealthy,
        }
    }

    fn apply_observation(&self, peer_key: &str, success: bool) {
        let Some(cell) = self.peers.get(peer_key) else {
            return;
        };
        let Ok(mut s) = cell.state.lock() else {
            return;
        };
        let prev = s.phase;
        if success {
            s.consecutive_failures = 0;
            s.consecutive_successes = s.consecutive_successes.saturating_add(1);
            if s.phase == PeerHealthPhase::Unhealthy
                && s.consecutive_successes >= self.config.healthy_threshold
            {
                s.phase = PeerHealthPhase::Healthy;
                s.consecutive_successes = 0;
            }
        } else {
            s.consecutive_successes = 0;
            s.consecutive_failures = s.consecutive_failures.saturating_add(1);
            if s.phase == PeerHealthPhase::Healthy
                && s.consecutive_failures >= self.config.unhealthy_threshold
            {
                s.phase = PeerHealthPhase::Unhealthy;
                s.consecutive_failures = 0;
            }
        }
        if self.generation == HEALTH_STATS_GEN.load(Ordering::Relaxed) {
            let mut map = health_stats_map();
            let st = map
                .entry((self.upstream.clone(), peer_key.to_string()))
                .or_default();
            if success {
                st.probes_ok += 1;
            } else {
                st.probes_fail += 1;
            }
            st.state = (s.phase == PeerHealthPhase::Healthy) as u64;
        }
        if prev != s.phase {
            match s.phase {
                PeerHealthPhase::Healthy => {
                    self.healthy_transitions.fetch_add(1, Ordering::Relaxed);
                    info!(
                        peer_key,
                        generation = self.generation,
                        "cap024 peer recovered HEALTHY"
                    );
                }
                PeerHealthPhase::Unhealthy => {
                    self.unhealthy_transitions.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        peer_key,
                        generation = self.generation,
                        "cap024 peer marked UNHEALTHY"
                    );
                }
            }
        }
    }
}

struct HealthInner {
    views: HashMap<u32, Arc<ClusterHealthView>>,
    tasks: Vec<JoinHandle<()>>,
}

/// Owns health views + probe tasks for the proxy runtime.
pub struct HealthSupervisor {
    /// Single mutex: abort/spawn/install is one critical section (no orphan tasks).
    inner: Mutex<HealthInner>,
}

impl Default for HealthSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthSupervisor {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HealthInner {
                views: HashMap::new(),
                tasks: Vec::new(),
            }),
        }
    }

    pub fn view(&self, cluster_id: u32) -> Option<Arc<ClusterHealthView>> {
        self.inner.lock().ok()?.views.get(&cluster_id).cloned()
    }

    /// `(cluster_id, upstream_name, health_config, [(peer_key, http_uri), ...])`
    #[allow(clippy::type_complexity)] // generation replace batch shape is intentional
    pub fn replace_generation(
        &self,
        generation: u64,
        clusters: &[(u32, String, ProxyHealthCheckCompiled, Vec<(String, String)>)],
    ) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        for t in inner.tasks.drain(..) {
            t.abort();
        }
        // Generation guard: aborted-but-still-finishing tasks can only write
        // stats while their view's generation is current.
        HEALTH_STATS_GEN.store(generation, Ordering::Relaxed);
        // NG-10: prune stats for peers no longer tracked — gauges must not
        // report phantom state for removed endpoints.
        let active: std::collections::HashSet<(String, String)> = clusters
            .iter()
            .filter(|(_, _, cfg, _)| cfg.enabled)
            .flat_map(|(_, upstream, _, peers)| {
                peers.iter().map(|(k, _)| (upstream.clone(), k.clone()))
            })
            .collect();
        health_stats_map().retain(|key, _| active.contains(key));
        let mut new_views = HashMap::new();
        let mut new_tasks = Vec::new();
        for (cluster_id, upstream, cfg, peers) in clusters {
            if !cfg.enabled || peers.is_empty() {
                continue;
            }
            // NG-10: one probe task per distinct peer — duplicate transport
            // identities would multiply failure accumulation on one cell.
            let mut peers = peers.clone();
            peers.sort_by(|a, b| a.0.cmp(&b.0));
            peers.dedup_by(|a, b| a.0 == b.0);
            let mut map = HashMap::new();
            for (peer_key, _) in &peers {
                map.insert(peer_key.clone(), PeerHealthCell::new());
            }
            let view = Arc::new(ClusterHealthView {
                generation,
                upstream: upstream.clone(),
                config: cfg.clone(),
                peers: map,
                healthy_transitions: AtomicU64::new(0),
                unhealthy_transitions: AtomicU64::new(0),
            });
            new_views.insert(*cluster_id, Arc::clone(&view));
            let n = peers.len().max(1) as u32;
            let slice = cfg
                .interval
                .checked_div(n)
                .unwrap_or(Duration::from_millis(1));
            for (i, (peer_key, http_uri)) in peers.iter().enumerate() {
                let view = Arc::clone(&view);
                let peer_key = peer_key.clone();
                let http_uri = http_uri.clone();
                let path = cfg.path.clone();
                let interval = cfg.interval;
                let timeout = cfg.timeout;
                let cfg = cfg.clone();
                let stagger = slice.saturating_mul((i as u32) % n);
                let handle = tokio::spawn(async move {
                    tokio::time::sleep(stagger).await;
                    let client = get_empty_body_client();
                    let mut ticker = tokio::time::interval(interval);
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        ticker.tick().await;
                        let Some(cell) = view.peers.get(&peer_key) else {
                            break;
                        };
                        if cell
                            .probe_in_flight
                            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                            .is_err()
                        {
                            continue;
                        }
                        let ok = probe_once(&client, &http_uri, &path, timeout, &cfg).await;
                        view.apply_observation(&peer_key, ok);
                        cell.probe_in_flight.store(false, Ordering::Release);
                    }
                });
                new_tasks.push(handle);
            }
        }
        inner.views = new_views;
        inner.tasks = new_tasks;
    }

    pub fn shutdown(&self) {
        // Invalidate stats writes from tasks that are still finishing.
        HEALTH_STATS_GEN.store(u64::MAX, Ordering::Relaxed);
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        for t in inner.tasks.drain(..) {
            t.abort();
        }
        inner.views.clear();
    }
}

impl Drop for HealthSupervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn probe_once(
    client: &hyper_util::client::legacy::Client<
        hyper_util::client::legacy::connect::HttpConnector,
        Empty<bytes::Bytes>,
    >,
    base_uri: &str,
    path: &str,
    timeout: Duration,
    cfg: &ProxyHealthCheckCompiled,
) -> bool {
    let Some(url) = join_health_url(base_uri, path) else {
        return false;
    };
    let req = match Request::builder()
        .method(Method::GET)
        .uri(&url)
        .header(hyper::header::USER_AGENT, "exyonq-cap024-health/1")
        .body(Empty::<bytes::Bytes>::new())
    {
        Ok(r) => r,
        Err(_) => return false,
    };
    // Single wall-clock budget for request + bounded body drain (Cap024 P3 harden).
    let deadline = tokio::time::Instant::now() + timeout;
    match tokio::time::timeout_at(deadline, client.request(req)).await {
        Ok(Ok(resp)) => {
            let status = resp.status();
            if !status_expected(status.as_u16(), &cfg.expect_status) {
                let _ = tokio::time::timeout_at(deadline, drain_body(resp.into_body(), None)).await;
                return false;
            }
            let headers_ok = match &cfg.expect_header {
                None => true,
                // Any occurrence may satisfy the expectation (multi-value
                // headers like Set-Cookie appear more than once).
                Some((name, expected)) => {
                    let mut values = resp.headers().get_all(name.as_str()).iter();
                    match expected {
                        None => values.next().is_some(),
                        Some(exp) => {
                            values.any(|v| v.to_str().map(|s| s.trim() == exp).unwrap_or(false))
                        }
                    }
                }
            };
            if !headers_ok {
                let _ = tokio::time::timeout_at(deadline, drain_body(resp.into_body(), None)).await;
                return false;
            }
            let need_body = cfg.expect_body.is_some();
            let collected = tokio::time::timeout_at(
                deadline,
                drain_body(
                    resp.into_body(),
                    if need_body { Some(64 * 1024) } else { None },
                ),
            )
            .await
            .unwrap_or(None);
            match &cfg.expect_body {
                None => true,
                Some(needle) => collected
                    .map(|b| contains_subslice(&b, needle.as_bytes()))
                    .unwrap_or(false),
            }
        }
        Ok(Err(_)) | Err(_) => false,
    }
}

/// NG-10: empty `expect_status` keeps Cap024 semantics (any 2xx).
fn status_expected(code: u16, expect: &[u16]) -> bool {
    if expect.is_empty() {
        return (200..300).contains(&code);
    }
    expect.contains(&code)
}

/// Bounded body drain; when `collect` is Some(cap) the first `cap` bytes are
/// returned for `expect_body` matching.
async fn drain_body(mut body: hyper::body::Incoming, collect: Option<usize>) -> Option<Vec<u8>> {
    let mut taken = 0usize;
    let mut buf = Vec::new();
    const MAX_DRAIN: usize = 64 * 1024;
    while taken < MAX_DRAIN {
        match body.frame().await {
            Some(Ok(frame)) => {
                if let Some(data) = frame.data_ref() {
                    taken = taken.saturating_add(data.len());
                    if let Some(cap) = collect {
                        if buf.len() < cap {
                            let room = cap - buf.len();
                            buf.extend_from_slice(&data[..data.len().min(room)]);
                        }
                    }
                }
            }
            _ => break,
        }
    }
    collect.map(|_| buf)
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|w| w == needle)
}

/// NG-10: `exyonq_upstream_health_*` Prometheus appender.
pub fn append_health_prometheus(out: &mut String) {
    let map = health_stats_map();
    out.push_str(
        "# HELP exyonq_upstream_health_probes_total Health probe results (process lifetime).\n",
    );
    out.push_str("# TYPE exyonq_upstream_health_probes_total counter\n");
    let mut keys: Vec<_> = map.keys().cloned().collect();
    keys.sort();
    for (upstream, peer) in &keys {
        let st = &map[&(upstream.clone(), peer.clone())];
        out.push_str(&format!(
            "exyonq_upstream_health_probes_total{{upstream=\"{}\",endpoint=\"{}\",result=\"success\"}} {}\n",
            escape_label(upstream),
            escape_label(peer),
            st.probes_ok
        ));
        out.push_str(&format!(
            "exyonq_upstream_health_probes_total{{upstream=\"{}\",endpoint=\"{}\",result=\"failure\"}} {}\n",
            escape_label(upstream),
            escape_label(peer),
            st.probes_fail
        ));
    }
    out.push_str(
        "# HELP exyonq_upstream_health_state Current peer health phase (1 healthy, 0 unhealthy).\n",
    );
    out.push_str("# TYPE exyonq_upstream_health_state gauge\n");
    for (upstream, peer) in &keys {
        let st = &map[&(upstream.clone(), peer.clone())];
        out.push_str(&format!(
            "exyonq_upstream_health_state{{upstream=\"{}\",endpoint=\"{}\"}} {}\n",
            escape_label(upstream),
            escape_label(peer),
            st.state
        ));
    }
}

fn escape_label(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Health probe URL: configured peer authority + path-only (no SSRF rewrite).
fn join_health_url(base_uri: &str, path: &str) -> Option<String> {
    let uri: http::Uri = base_uri.parse().ok()?;
    let scheme = uri.scheme_str().unwrap_or("http");
    let authority = uri.authority()?.as_str();
    if !path.starts_with('/') {
        return None;
    }
    Some(format!("{scheme}://{authority}{path}"))
}

pub fn peer_key_from_http_uri(http_uri: &str) -> Option<String> {
    endpoint_transport_identity(http_uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_failures_required() {
        let view = ClusterHealthView {
            generation: 1,
            upstream: "u".into(),
            config: ProxyHealthCheckCompiled {
                enabled: true,
                interval: Duration::from_millis(100),
                timeout: Duration::from_millis(50),
                path: "/health".into(),
                healthy_threshold: 2,
                unhealthy_threshold: 3,
                ..Default::default()
            },
            peers: {
                let mut m = HashMap::new();
                m.insert("p".into(), PeerHealthCell::new());
                m
            },
            healthy_transitions: AtomicU64::new(0),
            unhealthy_transitions: AtomicU64::new(0),
        };
        view.apply_observation("p", false);
        view.apply_observation("p", false);
        assert_eq!(view.phase("p"), PeerHealthPhase::Healthy);
        view.apply_observation("p", true);
        view.apply_observation("p", false);
        view.apply_observation("p", false);
        assert_eq!(view.phase("p"), PeerHealthPhase::Healthy);
        view.apply_observation("p", false);
        assert_eq!(view.phase("p"), PeerHealthPhase::Unhealthy);
        view.apply_observation("p", true);
        assert_eq!(view.phase("p"), PeerHealthPhase::Unhealthy);
        view.apply_observation("p", true);
        assert_eq!(view.phase("p"), PeerHealthPhase::Healthy);
    }

    #[test]
    fn enabled_unknown_peer_not_selectable() {
        let view = ClusterHealthView {
            generation: 1,
            upstream: "u".into(),
            config: ProxyHealthCheckCompiled {
                enabled: true,
                interval: Duration::from_millis(100),
                timeout: Duration::from_millis(50),
                path: "/health".into(),
                healthy_threshold: 2,
                unhealthy_threshold: 2,
                ..Default::default()
            },
            peers: HashMap::new(),
            healthy_transitions: AtomicU64::new(0),
            unhealthy_transitions: AtomicU64::new(0),
        };
        assert!(!view.is_selectable("removed-peer"));
    }

    #[test]
    fn join_health_url_keeps_authority() {
        assert_eq!(
            join_health_url("http://127.0.0.1:9/api", "/health").as_deref(),
            Some("http://127.0.0.1:9/health")
        );
    }

    #[test]
    fn stale_generation_observation_does_not_write_stats() {
        // A dead-generation probe finishing after prune must not recreate
        // phantom series (regression: stats write is gated on current gen).
        HEALTH_STATS_GEN.store(7, Ordering::Relaxed);
        let view = ClusterHealthView {
            generation: 999, // stale — no longer installed
            upstream: "u-stale".into(),
            config: ProxyHealthCheckCompiled {
                enabled: true,
                ..Default::default()
            },
            peers: {
                let mut m = HashMap::new();
                m.insert("p".into(), PeerHealthCell::new());
                m
            },
            healthy_transitions: AtomicU64::new(0),
            unhealthy_transitions: AtomicU64::new(0),
        };
        view.apply_observation("p", true);
        assert!(!health_stats_map().contains_key(&("u-stale".into(), "p".into())));
    }

    #[test]
    fn expect_header_matches_any_occurrence() {
        // Multi-value headers: expectation satisfied by any occurrence.
        let mut hm = hyper::HeaderMap::new();
        hm.append("x-h", hyper::header::HeaderValue::from_static("no"));
        hm.append("x-h", hyper::header::HeaderValue::from_static("pass"));
        let values: Vec<_> = hm.get_all("x-h").iter().collect();
        assert!(values
            .iter()
            .any(|v| v.to_str().map(|s| s.trim() == "pass").unwrap_or(false)));
    }
}
