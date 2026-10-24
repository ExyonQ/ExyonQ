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
//! NG-09: periodic DNS re-resolve for hostname upstream endpoints.
//!
//! Dial-time resolution (`tokio::net::lookup_host` / the hyper connector)
//! already picks up address changes for new connections. This supervisor adds
//! the scheduled re-resolution nginx `resolver valid=` provides, plus the
//! `exyonq_upstream_dns_*` observability surface: per-endpoint resolution and
//! failure counters, current address count, and a timestamp of the last
//! address-set change (the observable "backend IP changed without a reload"
//! event).

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;
use tracing::{info, warn};

#[derive(Default)]
struct DnsPeerStats {
    resolutions_total: u64,
    failures_total: u64,
    last_change_unix_s: u64,
    last_addrs: Vec<IpAddr>,
}

static DNS_STATS: LazyLock<Mutex<HashMap<(String, String), DnsPeerStats>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Monotonic supervisor epoch — a dead-generation task must not write stats
/// after prune cleared the table (abort is not synchronous).
static DNS_STATS_EPOCH: AtomicU64 = AtomicU64::new(0);

fn stats_map() -> std::sync::MutexGuard<'static, HashMap<(String, String), DnsPeerStats>> {
    DNS_STATS.lock().unwrap_or_else(|p| p.into_inner())
}

/// One hostname endpoint tracked by a refresh task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsPeer {
    /// `(upstream_name, "host:port")` label pair.
    pub upstream: String,
    /// Hostname (never a literal IP — literal IPs are filtered upstream).
    pub host: String,
    pub port: u16,
}

impl DnsPeer {
    fn label(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Build a tracked peer from a compiled `http://host:port` URI; `None` when
/// the authority is a literal IP (re-resolving a literal is meaningless).
pub fn dns_peer_from_http_uri(upstream: &str, uri: &str) -> Option<DnsPeer> {
    let authority = uri.parse::<http::Uri>().ok()?.authority()?.clone();
    // `Authority::host()` keeps RFC-3986 brackets for IPv6 literals.
    let host = authority
        .host()
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or_else(|| authority.host());
    if host.is_empty() || host.parse::<IpAddr>().is_ok() {
        return None;
    }
    Some(DnsPeer {
        upstream: upstream.to_string(),
        host: host.to_string(),
        port: authority.port_u16().unwrap_or(80),
    })
}

/// `(cluster_id, upstream_name, refresh_interval, peers)` — generation batch.
pub type DnsClusterSpec = (u32, String, Duration, Vec<DnsPeer>);

/// Owns periodic DNS re-resolve tasks for the proxy runtime (generation-scoped,
/// mirrors [`crate::health::HealthSupervisor`]).
pub struct DnsRefreshSupervisor {
    inner: Mutex<Vec<JoinHandle<()>>>,
}

impl Default for DnsRefreshSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl DnsRefreshSupervisor {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    /// Abort prior tasks, then spawn one task per upstream that configured
    /// `dns_refresh_ms` and still has hostname endpoints.
    pub fn replace_generation(&self, clusters: &[DnsClusterSpec]) {
        let mut tasks = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        for t in tasks.drain(..) {
            t.abort();
        }
        let epoch = DNS_STATS_EPOCH.fetch_add(1, Ordering::Relaxed) + 1;
        // Keep stats/gauges only for peers still tracked — a reload that drops
        // an endpoint must not leave phantom `dns_addresses` series forever.
        // Counters for surviving peers stay (process-lifetime by convention).
        let active: std::collections::HashSet<(String, String)> = clusters
            .iter()
            .flat_map(|(_, upstream, _, peers)| peers.iter().map(|p| (upstream.clone(), p.label())))
            .collect();
        stats_map().retain(|key, _| active.contains(key));
        for (_, upstream, interval, peers) in clusters {
            if peers.is_empty() {
                continue;
            }
            // One lookup per distinct host:port per tick.
            let mut peers = peers.clone();
            peers.sort_by_key(|p| p.label());
            peers.dedup_by(|a, b| a.label() == b.label());
            let upstream = upstream.clone();
            let interval = *interval;
            tasks.push(tokio::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    ticker.tick().await;
                    for peer in &peers {
                        refresh_peer(&upstream, peer, Some(epoch)).await;
                    }
                }
            }));
        }
    }

    /// Abort all tasks (process shutdown / test teardown). Stats persist —
    /// Prometheus counters are process-lifetime by convention.
    pub fn shutdown(&self) {
        // Invalidate stats writes from tasks that are still finishing.
        DNS_STATS_EPOCH.fetch_add(1, Ordering::Relaxed);
        let mut tasks = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        for t in tasks.drain(..) {
            t.abort();
        }
    }
}

impl Drop for DnsRefreshSupervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn refresh_peer(upstream: &str, peer: &DnsPeer, epoch: Option<u64>) {
    let label = peer.label();
    let result = tokio::net::lookup_host((peer.host.as_str(), peer.port)).await;
    // Dead-generation tasks skip stats writes (abort is not synchronous).
    if epoch.is_some_and(|e| e != DNS_STATS_EPOCH.load(Ordering::Relaxed)) {
        return;
    }
    let mut map = stats_map();
    let st = map
        .entry((upstream.to_string(), label.clone()))
        .or_default();
    match result {
        Ok(addrs) => {
            st.resolutions_total += 1;
            let mut addrs: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
            addrs.sort();
            addrs.dedup();
            if st.last_addrs.is_empty() {
                // First observation is the baseline, not a change event.
                st.last_addrs = addrs;
            } else if addrs != st.last_addrs {
                st.last_change_unix_s = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                info!(
                    upstream,
                    endpoint = %label,
                    old = ?st.last_addrs,
                    new = ?addrs,
                    "upstream DNS address set changed"
                );
                st.last_addrs = addrs;
            }
        }
        Err(err) => {
            st.failures_total += 1;
            warn!(upstream, endpoint = %label, %err, "upstream DNS re-resolve failed");
        }
    }
}

fn escape_label(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Cold-path Prometheus appender (`exyonq_upstream_dns_*` series).
pub fn append_dns_prometheus(out: &mut String) {
    let map = stats_map();
    if map.is_empty() {
        return;
    }
    let rows: BTreeMap<(String, String), (u64, u64, u64, u64)> = map
        .iter()
        .map(|(k, s)| {
            (
                k.clone(),
                (
                    s.resolutions_total,
                    s.failures_total,
                    s.last_change_unix_s,
                    s.last_addrs.len() as u64,
                ),
            )
        })
        .collect();
    out.push_str(
        "# HELP exyonq_upstream_dns_resolutions_total Successful periodic re-resolutions (process lifetime).\n",
    );
    out.push_str("# TYPE exyonq_upstream_dns_resolutions_total counter\n");
    for ((upstream, endpoint), (ok, _, _, _)) in &rows {
        out.push_str(&format!(
            "exyonq_upstream_dns_resolutions_total{{upstream=\"{}\",endpoint=\"{}\"}} {ok}\n",
            escape_label(upstream),
            escape_label(endpoint),
        ));
    }
    out.push_str(
        "# HELP exyonq_upstream_dns_failures_total Failed periodic re-resolutions (process lifetime).\n",
    );
    out.push_str("# TYPE exyonq_upstream_dns_failures_total counter\n");
    for ((upstream, endpoint), (_, fail, _, _)) in &rows {
        out.push_str(&format!(
            "exyonq_upstream_dns_failures_total{{upstream=\"{}\",endpoint=\"{}\"}} {fail}\n",
            escape_label(upstream),
            escape_label(endpoint),
        ));
    }
    out.push_str(
        "# HELP exyonq_upstream_dns_addresses Current DNS address count per hostname endpoint.\n",
    );
    out.push_str("# TYPE exyonq_upstream_dns_addresses gauge\n");
    for ((upstream, endpoint), (_, _, _, n)) in &rows {
        out.push_str(&format!(
            "exyonq_upstream_dns_addresses{{upstream=\"{}\",endpoint=\"{}\"}} {n}\n",
            escape_label(upstream),
            escape_label(endpoint),
        ));
    }
    out.push_str(
        "# HELP exyonq_upstream_dns_last_change_timestamp_seconds Unix time of last observed address-set change (0 = never).\n",
    );
    out.push_str("# TYPE exyonq_upstream_dns_last_change_timestamp_seconds gauge\n");
    for ((upstream, endpoint), (_, _, change, _)) in &rows {
        out.push_str(&format!(
            "exyonq_upstream_dns_last_change_timestamp_seconds{{upstream=\"{}\",endpoint=\"{}\"}} {change}\n",
            escape_label(upstream),
            escape_label(endpoint),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(host: &str, port: u16) -> DnsPeer {
        DnsPeer {
            upstream: "u".into(),
            host: host.into(),
            port,
        }
    }

    #[tokio::test]
    async fn refresh_peer_records_resolution_and_addr_count() {
        // localhost always resolves; baseline observation must not count as change.
        let p = peer("localhost", 80);
        refresh_peer("u-ng09", &p, None).await;
        let map = stats_map();
        let st = &map[&("u-ng09".to_string(), "localhost:80".to_string())];
        assert_eq!(st.resolutions_total, 1);
        assert_eq!(st.failures_total, 0);
        assert!(!st.last_addrs.is_empty());
        assert_eq!(st.last_change_unix_s, 0);
    }

    #[tokio::test]
    async fn refresh_peer_records_failure() {
        let p = peer("nonexistent.invalid.example", 80);
        refresh_peer("u-ng09", &p, None).await;
        let map = stats_map();
        let st = &map[&(
            "u-ng09".to_string(),
            "nonexistent.invalid.example:80".to_string(),
        )];
        assert_eq!(st.failures_total, 1);
        assert_eq!(st.resolutions_total, 0);
    }

    #[test]
    fn literal_ips_are_not_tracked() {
        // IPv4 literal, IPv6 literal (bracketed authority), and hostname.
        assert!(dns_peer_from_http_uri("u", "http://127.0.0.1:8001").is_none());
        assert!(dns_peer_from_http_uri("u", "http://[::1]:8001").is_none());
        assert!(dns_peer_from_http_uri("u", "not a uri").is_none());
        let p = dns_peer_from_http_uri("u", "http://a.internal:8001").unwrap();
        assert_eq!((p.host.as_str(), p.port), ("a.internal", 8001));
        // No explicit port → default 80.
        let p = dns_peer_from_http_uri("u", "http://b.internal").unwrap();
        assert_eq!(p.port, 80);
    }

    #[tokio::test]
    async fn replace_generation_dedups_and_prunes_stats() {
        let sup = DnsRefreshSupervisor::new();
        let peers = vec![
            peer("dup.test", 80),
            peer("dup.test", 80),
            peer("gone.test", 81),
        ];
        sup.replace_generation(&[(1u32, "u-dedup".into(), Duration::from_secs(3600), peers)]);
        {
            let tasks = sup.inner.lock().unwrap_or_else(|p| p.into_inner());
            assert_eq!(tasks.len(), 1);
        }
        // Seed a stale stat, then a generation without `gone.test` must prune it.
        stats_map()
            .entry(("u-dedup".into(), "gone.test:81".into()))
            .or_default()
            .resolutions_total = 9;
        let peers = vec![peer("dup.test", 80)];
        sup.replace_generation(&[(1u32, "u-dedup".into(), Duration::from_secs(3600), peers)]);
        assert!(!stats_map().contains_key(&("u-dedup".into(), "gone.test:81".into())));
        sup.shutdown();
    }

    #[tokio::test]
    async fn stale_epoch_observation_does_not_write_stats() {
        // A dead-generation task finishing after prune must not recreate
        // phantom series (stats write is gated on the current epoch).
        let p = peer("stale-epoch.test", 80);
        let stale = DNS_STATS_EPOCH.load(Ordering::Relaxed) + 10_000;
        refresh_peer("u-stale", &p, Some(stale)).await;
        assert!(!stats_map().contains_key(&("u-stale".into(), "stale-epoch.test:80".into())));
    }

    #[test]
    fn prometheus_render_emits_all_series() {
        {
            let mut map = stats_map();
            map.entry(("u-render".into(), "h.test:443".into()))
                .or_default()
                .resolutions_total = 3;
        }
        let mut out = String::new();
        append_dns_prometheus(&mut out);
        for series in [
            "exyonq_upstream_dns_resolutions_total",
            "exyonq_upstream_dns_failures_total",
            "exyonq_upstream_dns_addresses",
            "exyonq_upstream_dns_last_change_timestamp_seconds",
        ] {
            assert!(out.contains(series), "{series} missing");
        }
        assert!(out.contains("upstream=\"u-render\""));
    }
}
