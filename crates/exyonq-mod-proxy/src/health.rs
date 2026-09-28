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
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{info, warn};

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

    /// `(cluster_id, health_config, [(peer_key, http_uri), ...])`
    #[allow(clippy::type_complexity)] // generation replace batch shape is intentional
    pub fn replace_generation(
        &self,
        generation: u64,
        clusters: &[(u32, ProxyHealthCheckCompiled, Vec<(String, String)>)],
    ) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        for t in inner.tasks.drain(..) {
            t.abort();
        }
        let mut new_views = HashMap::new();
        let mut new_tasks = Vec::new();
        for (cluster_id, cfg, peers) in clusters {
            if !cfg.enabled || peers.is_empty() {
                continue;
            }
            let mut map = HashMap::new();
            for (peer_key, _) in peers {
                map.insert(peer_key.clone(), PeerHealthCell::new());
            }
            let view = Arc::new(ClusterHealthView {
                generation,
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
                        let ok = probe_once(client, &http_uri, &path, timeout).await;
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
            let _ = tokio::time::timeout_at(deadline, async {
                let mut body = resp.into_body();
                let mut taken = 0usize;
                const MAX_DRAIN: usize = 64 * 1024;
                while taken < MAX_DRAIN {
                    match body.frame().await {
                        Some(Ok(frame)) => {
                            if let Some(data) = frame.data_ref() {
                                taken = taken.saturating_add(data.len());
                            }
                        }
                        _ => break,
                    }
                }
            })
            .await;
            status.is_success()
        }
        Ok(Err(_)) | Err(_) => false,
    }
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
            config: ProxyHealthCheckCompiled {
                enabled: true,
                interval: Duration::from_millis(100),
                timeout: Duration::from_millis(50),
                path: "/health".into(),
                healthy_threshold: 2,
                unhealthy_threshold: 3,
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
            config: ProxyHealthCheckCompiled {
                enabled: true,
                interval: Duration::from_millis(100),
                timeout: Duration::from_millis(50),
                path: "/health".into(),
                healthy_threshold: 2,
                unhealthy_threshold: 2,
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
}
