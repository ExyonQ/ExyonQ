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
//! K8S-P2B productive multi-endpoint selection (smooth WRR + priority bands).
//!
//! ```text
//! WRR_ALGORITHM_SELECTED = SMOOTH_WEIGHTED_ROUND_ROBIN
//! PRIORITY_ORDERING = LOWER_NUMERIC_PRIORITY_IS_PREFERRED
//! WEIGHT_ZERO_SEMANTICS = NOT_SELECTABLE
//! P2B_PRIORITY_FAILOVER = BASED_ON_CONFIGURED_ELIGIBILITY_ONLY
//! ```

use exyonq_module_api::proxy_dispatch::ProxySelectionPolicy;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// NG-08: passive health state for one endpoint — nginx `max_fails` /
/// `fail_timeout` / `slow_start` semantics.
///
/// Failure accounting matches nginx: `max_fails` failed attempts counted
/// **inside** `fail_timeout` ban the peer for `fail_timeout` (a gap longer
/// than the window since the last failure restarts the counter). A success
/// resets it. When the ban expires and `slow_start` > 0, the effective
/// weight ramps linearly back to the configured value.
///
/// Timestamps are relative to this peer's own [`Instant`] epoch — peers never
/// compare deadlines across objects.
#[derive(Debug)]
pub struct PassivePeer {
    max_fails: u32,
    fail_timeout_ms: u64,
    slow_start_ms: u64,
    epoch: Instant,
    /// Consecutive failed attempts; reset on success and when the ban arms.
    fails: AtomicU64,
    /// Ban end in ms since `epoch`; `0` = never banned.
    banned_until_ms: AtomicU64,
    /// Timestamp (ms since `epoch`) of the last recorded failure; `u64::MAX`
    /// = none yet. nginx counts fails *inside* `fail_timeout` — a gap longer
    /// than the window restarts the counter.
    last_fail_ms: AtomicU64,
}

impl PassivePeer {
    pub fn new(max_fails: u32, fail_timeout_ms: u64, slow_start_ms: u64) -> Self {
        Self {
            max_fails,
            fail_timeout_ms,
            slow_start_ms,
            epoch: Instant::now(),
            fails: AtomicU64::new(0),
            banned_until_ms: AtomicU64::new(0),
            last_fail_ms: AtomicU64::new(u64::MAX),
        }
    }

    /// Failure accounting disabled (`max_fails = 0` semantics).
    pub fn disabled() -> Self {
        Self::new(0, 0, 0)
    }

    /// Whether this peer is banned right now.
    pub fn is_banned(&self) -> bool {
        let until = self.banned_until_ms.load(Ordering::Relaxed);
        until != 0 && self.now_ms() < until
    }

    /// Eligible for selection (inverse of the ban window).
    pub fn selectable(&self) -> bool {
        !self.is_banned()
    }

    /// Consecutive-failure counter (observability/tests).
    pub fn fails(&self) -> u64 {
        self.fails.load(Ordering::Relaxed)
    }

    /// NG-08 outcome hook — `ok = true` for any received upstream response
    /// (nginx default `proxy_next_upstream error timeout` counts only
    /// transport failures, not HTTP 5xx); `ok = false` on connect/transport
    /// error or attempt timeout.
    pub fn record(&self, ok: bool) {
        if ok {
            self.fails.store(0, Ordering::Relaxed);
            return;
        }
        if self.max_fails == 0 {
            return;
        }
        // Attempts cannot reach a banned peer through selection; a late
        // in-flight record during the ban must not extend it silently — the
        // ban is already armed and fails was reset when it armed.
        if self.is_banned() {
            return;
        }
        // nginx: fails are counted inside `fail_timeout` — a gap longer than
        // the window since the last failure restarts the counter.
        let now = self.now_ms();
        let last = self.last_fail_ms.swap(now, Ordering::Relaxed);
        if last != u64::MAX && now.saturating_sub(last) > self.fail_timeout_ms {
            self.fails.store(0, Ordering::Relaxed);
        }
        let n = self.fails.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= u64::from(self.max_fails) {
            self.banned_until_ms.store(
                self.now_ms().saturating_add(self.fail_timeout_ms),
                Ordering::Relaxed,
            );
            self.fails.store(0, Ordering::Relaxed);
        }
    }

    /// Effective weight for selection: `0` while banned (defensive — callers
    /// gate on [`Self::selectable`]), linear ramp `0..=w` while inside
    /// `slow_start` after ban expiry (nginx: the recovering server's weight
    /// grows linearly from 0), `w` otherwise.
    fn ramped_weight(&self, w: i64) -> i64 {
        if self.slow_start_ms == 0 || w <= 0 {
            return w;
        }
        let banned_end = self.banned_until_ms.load(Ordering::Relaxed);
        if banned_end == 0 {
            return w;
        }
        let now = self.now_ms();
        if now < banned_end {
            return 0;
        }
        let elapsed = now - banned_end;
        if elapsed >= self.slow_start_ms {
            return w;
        }
        let scaled =
            (w as u128).saturating_mul(u128::from(elapsed)) / u128::from(self.slow_start_ms);
        scaled.min(w as u128) as i64
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
}

/// Config-identity equality for `EndpointSpec` (runtime state never compares).
impl PartialEq for PassivePeer {
    fn eq(&self, other: &Self) -> bool {
        self.max_fails == other.max_fails
            && self.fail_timeout_ms == other.fail_timeout_ms
            && self.slow_start_ms == other.slow_start_ms
    }
}
impl Eq for PassivePeer {}

/// Desired endpoint row used to build a selector.
///
/// `passive` (NG-08) is shared with the runtime's [`UpstreamTarget`] for the
/// same endpoint — the selector gates/ramps on it while transport paths
/// record attempt outcomes into the same object.
/// AP-01: hot-mutable member admin state (exyonqctl `server set`).
/// `weight=0` or `enabled=false` makes the member ineligible immediately —
/// same effective gate as a configured `weight = 0` at build.
#[derive(Debug)]
pub struct EndpointAdmin {
    enabled: AtomicBool,
    weight: AtomicU64,
}

impl EndpointAdmin {
    pub fn new(enabled: bool, weight: u32) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            weight: AtomicU64::new(u64::from(weight)),
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
    pub fn set_enabled(&self, v: bool) {
        self.enabled.store(v, Ordering::Relaxed);
    }
    pub fn weight(&self) -> u32 {
        u32::try_from(self.weight.load(Ordering::Relaxed)).unwrap_or(u32::MAX)
    }
    pub fn set_weight(&self, v: u32) {
        self.weight.store(u64::from(v), Ordering::Relaxed);
    }
}

impl PartialEq for EndpointAdmin {
    fn eq(&self, other: &Self) -> bool {
        self.enabled() == other.enabled() && self.weight() == other.weight()
    }
}
impl Eq for EndpointAdmin {}

#[derive(Debug, Clone)]
pub struct EndpointSpec {
    pub endpoint_id: String,
    pub http_uri: String,
    /// Configured weight baseline (admin overlay may differ at runtime).
    pub weight: u32,
    pub priority: u32,
    /// Configured admin baseline (admin overlay may differ at runtime).
    pub admin_enabled: bool,
    /// NG-08: passive health peer state (shared Arc with the runtime target).
    pub passive: Arc<PassivePeer>,
    /// AP-01: hot admin overlay shared with the control surface.
    pub admin: Arc<EndpointAdmin>,
}

impl PartialEq for EndpointSpec {
    fn eq(&self, other: &Self) -> bool {
        self.endpoint_id == other.endpoint_id
            && self.http_uri == other.http_uri
            && self.weight == other.weight
            && self.priority == other.priority
            && self.admin_enabled == other.admin_enabled
            && self.passive == other.passive
            && self.admin == other.admin
    }
}
impl Eq for EndpointSpec {}

impl EndpointSpec {
    /// AP-01: hot eligibility — the admin overlay (enable/weight) is evaluated
    /// per selection so `exyonqctl server set` takes effect without a rebuild.
    fn admin_selectable(&self) -> bool {
        self.admin.enabled() && self.admin.weight() > 0
    }
}

/// NG-07: in-flight request lease for `LeastConn`/`PowerOfTwo` accounting.
/// Decrements the endpoint's counter on drop.
#[derive(Debug)]
pub struct InflightLease {
    counter: Arc<AtomicU64>,
}

impl Drop for InflightLease {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailoverMode {
    /// All eligible endpoints share one WRR band (priority ignored for banding).
    None,
    /// Lower numeric priority = preferred band; failover only when preferred band empty.
    PriorityBands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOutcome {
    /// Index into the selector's full endpoint table (not band-local).
    Selected {
        endpoint_index: usize,
        priority: u32,
    },
    NoEligibleEndpoint,
}

/// AP-01: member row exposed to the control surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStatus {
    pub endpoint_id: String,
    pub enabled: bool,
    pub weight: u32,
    /// Composite selectability now (admin gate + passive/health), informational.
    pub selectable: bool,
}

#[derive(Debug)]
struct Band {
    priority: u32,
    /// Indices into `endpoints` of the parent selector.
    members: Box<[usize]>,
    current: Mutex<Box<[i64]>>,
}

/// NG-07: cap on virtual hash points per endpoint (`weight` clamp) so a
/// pathological weight cannot turn affinity selection into a hot-loop DoS.
/// shortcut: weights above 64 are treated as 64; upgrade to a ketama ring if
/// exact large-weight fidelity is ever required.
const MAX_HASH_VPOINTS: u32 = 64;

/// Compiled per-cluster selector. Rebuild off hot path; select under per-band mutex only.
#[derive(Debug)]
pub struct EndpointSelector {
    pub generation: u64,
    endpoints: Box<[EndpointSpec]>,
    bands: Box<[Band]>,
    /// NG-07: selection/affinity policy compiled from config.
    policy: ProxySelectionPolicy,
    /// NG-07: sticky cookie name for `CookieAffinity` (normalized upstream).
    sticky_cookie: Option<Box<str>>,
    /// NG-07: header name for `HeaderAffinity`.
    sticky_header: Option<Box<str>>,
    /// NG-07: parallel to `endpoints` — in-flight counts for least_conn / P2C.
    inflight: Box<[Arc<AtomicU64>]>,
    /// NG-07: scramble seed for P2C sampling (not cryptographic).
    p2c_seed: AtomicU64,
}

impl EndpointSelector {
    /// Build selector from desired specs. Ineligible endpoints are omitted from bands.
    pub fn build(generation: u64, failover: FailoverMode, specs: Vec<EndpointSpec>) -> Self {
        Self::build_policy(
            generation,
            failover,
            specs,
            ProxySelectionPolicy::default(),
            None,
            None,
        )
    }

    /// NG-07: build with a compiled selection policy and sticky metadata.
    pub fn build_policy(
        generation: u64,
        failover: FailoverMode,
        specs: Vec<EndpointSpec>,
        policy: ProxySelectionPolicy,
        sticky_cookie: Option<String>,
        sticky_header: Option<String>,
    ) -> Self {
        let endpoints: Box<[EndpointSpec]> = specs.into_boxed_slice();
        // AP-01: all members stay in the band table so a hot `enabled=true`
        // rejoins without a rebuild; eligibility is gated per selection on the
        // shared admin overlay (enabled + weight > 0).
        let eligible: Vec<usize> = (0..endpoints.len()).collect();

        let bands = match failover {
            FailoverMode::None => {
                if eligible.is_empty() {
                    Vec::new().into_boxed_slice()
                } else {
                    vec![make_band(0, &eligible, &endpoints)].into_boxed_slice()
                }
            }
            FailoverMode::PriorityBands => {
                let mut by_prio: Vec<(u32, Vec<usize>)> = Vec::new();
                for idx in eligible {
                    let p = endpoints[idx].priority;
                    match by_prio.binary_search_by_key(&p, |(k, _)| *k) {
                        Ok(pos) => by_prio[pos].1.push(idx),
                        Err(pos) => by_prio.insert(pos, (p, vec![idx])),
                    }
                }
                // Stable within band: endpoint_id ascending (Contract 2).
                for (_, members) in by_prio.iter_mut() {
                    members.sort_by(|&a, &b| {
                        endpoints[a]
                            .endpoint_id
                            .cmp(&endpoints[b].endpoint_id)
                            .then_with(|| a.cmp(&b))
                    });
                }
                by_prio
                    .into_iter()
                    .map(|(p, members)| make_band(p, &members, &endpoints))
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            }
        };

        let inflight = endpoints
            .iter()
            .map(|_| Arc::new(AtomicU64::new(0)))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            generation,
            endpoints,
            bands,
            policy,
            sticky_cookie: sticky_cookie.map(String::into_boxed_str),
            sticky_header: sticky_header.map(String::into_boxed_str),
            inflight,
            p2c_seed: AtomicU64::new(0x9e37_79b9_7f4a_7c15 ^ generation),
        }
    }

    pub fn endpoint_count(&self) -> usize {
        self.endpoints.len()
    }

    /// NG-07: compiled selection policy for this cluster.
    pub fn policy(&self) -> ProxySelectionPolicy {
        self.policy
    }

    /// NG-07: sticky cookie name (`CookieAffinity`).
    pub fn sticky_cookie(&self) -> Option<&str> {
        self.sticky_cookie.as_deref()
    }

    /// NG-07: sticky header name (`HeaderAffinity`).
    pub fn sticky_header(&self) -> Option<&str> {
        self.sticky_header.as_deref()
    }

    pub fn band_count(&self) -> usize {
        self.bands.len()
    }

    pub fn endpoints(&self) -> &[EndpointSpec] {
        &self.endpoints
    }

    /// AP-01: member rows for the control surface (`exyonqctl server list`).
    pub fn member_list(&self) -> Vec<MemberStatus> {
        self.endpoints
            .iter()
            .map(|ep| MemberStatus {
                endpoint_id: ep.endpoint_id.clone(),
                enabled: ep.admin.enabled(),
                weight: ep.admin.weight(),
                selectable: ep.admin_selectable() && ep.passive.selectable(),
            })
            .collect()
    }

    /// AP-01: hot member mutation. `None` = field untouched; `weight = 0`
    /// drains the member without removing it (BalancerMember semantics).
    pub fn member_set(
        &self,
        endpoint_id: &str,
        enabled: Option<bool>,
        weight: Option<u32>,
    ) -> Option<MemberStatus> {
        let ep = self
            .endpoints
            .iter()
            .find(|e| e.endpoint_id == endpoint_id)?;
        if let Some(v) = enabled {
            ep.admin.set_enabled(v);
        }
        if let Some(v) = weight {
            ep.admin.set_weight(v);
        }
        Some(MemberStatus {
            endpoint_id: ep.endpoint_id.clone(),
            enabled: ep.admin.enabled(),
            weight: ep.admin.weight(),
            selectable: ep.admin_selectable() && ep.passive.selectable(),
        })
    }

    /// Smooth WRR within the first non-empty eligible band (bands pre-filtered).
    pub fn select(&self) -> SelectionOutcome {
        self.select_filtered(|_| true)
    }

    /// Cap045 / LA-CAP045-001: smooth WRR among members that pass `live`.
    ///
    /// Cap024 unhealthy and Cap021 request-local exclusion must not burn WRR
    /// credits on skipped peers or fall through to first-match scan. Non-live
    /// band members keep `current` cleared so recovery does not inherit a
    /// stale accumulator spike.
    pub fn select_filtered<F>(&self, mut live: F) -> SelectionOutcome
    where
        F: FnMut(usize) -> bool,
    {
        for band in self.bands.iter() {
            if band.members.is_empty() {
                continue;
            }
            let mut current = band.current.lock().expect("selector band poisoned");
            let mut live_local: Vec<usize> = Vec::new();
            // NG-08: slow_start ramp — the effective weight of a recovering
            // peer is scaled by ramp progress. AP-01: the base weight is the
            // live admin weight, so `server set ... weight=N` applies now.
            let mut raw: Vec<u32> = Vec::new();
            for (i, &endpoint_index) in band.members.iter().enumerate() {
                let ep = &self.endpoints[endpoint_index];
                if ep.admin_selectable() && ep.passive.selectable() && live(endpoint_index) {
                    live_local.push(i);
                    raw.push(ep.admin.weight());
                } else {
                    // Drop stale credit while peer is not selectable this request.
                    current[i] = 0;
                }
            }
            let norm = normalize_weights(&raw);
            let mut ramped: Vec<i64> = Vec::with_capacity(norm.len());
            let mut live_total: i64 = 0;
            for (k, &i) in live_local.iter().enumerate() {
                let w = self.endpoints[band.members[i]]
                    .passive
                    .ramped_weight(norm[k]);
                ramped.push(w);
                live_total = live_total.saturating_add(w);
            }
            if live_local.is_empty() || live_total <= 0 {
                continue;
            }
            let mut best = live_local[0];
            let mut best_val = i64::MIN;
            for (k, &i) in live_local.iter().enumerate() {
                current[i] = current[i].saturating_add(ramped[k]);
                if current[i] > best_val {
                    best_val = current[i];
                    best = i;
                }
            }
            current[best] = current[best].saturating_sub(live_total);
            drop(current);
            let endpoint_index = band.members[best];
            let priority = self.endpoints[endpoint_index].priority;
            return SelectionOutcome::Selected {
                endpoint_index,
                priority,
            };
        }
        SelectionOutcome::NoEligibleEndpoint
    }

    /// Active preferred priority when bands exist (lowest numeric) — first
    /// band that still holds an admin-selectable member (AP-01: bands keep
    /// hot-disabled members in the table).
    pub fn preferred_priority(&self) -> Option<u32> {
        self.bands
            .iter()
            .find(|b| {
                b.members
                    .iter()
                    .any(|&i| self.endpoints[i].admin_selectable())
            })
            .map(|b| b.priority)
    }

    /// NG-07: weighted rendezvous (HRW) selection of `key` inside the first band
    /// with live members. The same key maps to the same endpoint while the live
    /// set is stable; when an endpoint leaves, only its keys remap
    /// (consistent-hash semantics for `consistent_hash` / `source_affinity` /
    /// `header_affinity`).
    pub fn select_key_filtered<F>(&self, key: &str, mut live: F) -> SelectionOutcome
    where
        F: FnMut(usize) -> bool,
    {
        for band in self.bands.iter() {
            let mut best: Option<usize> = None;
            let mut best_score = 0u64;
            for &endpoint_index in band.members.iter() {
                let ep = &self.endpoints[endpoint_index];
                if !ep.admin_selectable() || !ep.passive.selectable() || !live(endpoint_index) {
                    continue;
                }
                // NG-08: slow_start scales the endpoint's hash points too; a
                // peer ramped to 0 weight cannot win the rendezvous.
                let ramped = ep.passive.ramped_weight(i64::from(ep.admin.weight()));
                if ramped <= 0 {
                    continue;
                }
                let vpoints = u32::try_from(ramped)
                    .unwrap_or(u32::MAX)
                    .clamp(1, MAX_HASH_VPOINTS);
                let mut top = 0u64;
                for v in 0..vpoints {
                    let h = hrw_hash(key.as_bytes(), ep.endpoint_id.as_bytes(), v);
                    if h > top {
                        top = h;
                    }
                }
                if top > best_score {
                    best = Some(endpoint_index);
                    best_score = top;
                }
            }
            if let Some(endpoint_index) = best {
                let priority = self.endpoints[endpoint_index].priority;
                return SelectionOutcome::Selected {
                    endpoint_index,
                    priority,
                };
            }
        }
        SelectionOutcome::NoEligibleEndpoint
    }

    /// NG-07 `cookie_affinity`: pin to the endpoint named by the route token.
    /// Band order does not gate an explicit pin; eligibility + `live` still do.
    pub fn select_by_id_filtered<F>(&self, endpoint_id: &str, mut live: F) -> SelectionOutcome
    where
        F: FnMut(usize) -> bool,
    {
        if let Some((endpoint_index, ep)) = self
            .endpoints
            .iter()
            .enumerate()
            .find(|(_, e)| e.endpoint_id == endpoint_id)
        {
            if ep.admin_selectable() && ep.passive.selectable() && live(endpoint_index) {
                return SelectionOutcome::Selected {
                    endpoint_index,
                    priority: ep.priority,
                };
            }
        }
        SelectionOutcome::NoEligibleEndpoint
    }

    /// NG-07 `least_conn`: least in-flight per unit of effective weight among
    /// live members of the first band with candidates (nginx compares
    /// conns/weight — and NG-08 slow_start feeds the ramped weight, so a
    /// recovering peer does not absorb full preference during the ramp).
    pub fn select_least_conn<F>(
        self: &Arc<Self>,
        mut live: F,
    ) -> (SelectionOutcome, Option<InflightLease>)
    where
        F: FnMut(usize) -> bool,
    {
        for band in self.bands.iter() {
            let mut best: Option<usize> = None;
            let mut best_load = 0u64;
            let mut best_ramped = 1i64;
            for &endpoint_index in band.members.iter() {
                let ep = &self.endpoints[endpoint_index];
                if !ep.admin_selectable() || !ep.passive.selectable() || !live(endpoint_index) {
                    continue;
                }
                let ramped = ep.passive.ramped_weight(i64::from(ep.admin.weight()));
                if ramped <= 0 {
                    continue;
                }
                let load = self.inflight[endpoint_index].load(Ordering::Relaxed);
                // Minimize load/ramped via cross-multiplication (no division).
                if best.is_none()
                    || u128::from(load) * u128::try_from(best_ramped).unwrap_or(1)
                        < u128::from(best_load) * u128::try_from(ramped).unwrap_or(1)
                {
                    best = Some(endpoint_index);
                    best_load = load;
                    best_ramped = ramped;
                }
            }
            if let Some(endpoint_index) = best {
                let priority = self.endpoints[endpoint_index].priority;
                return (
                    SelectionOutcome::Selected {
                        endpoint_index,
                        priority,
                    },
                    self.mint_lease(endpoint_index),
                );
            }
        }
        (SelectionOutcome::NoEligibleEndpoint, None)
    }

    /// NG-07 `power_of_two` (P2C): sample two live members, pick the less loaded.
    pub fn select_power_of_two<F>(
        self: &Arc<Self>,
        mut live: F,
    ) -> (SelectionOutcome, Option<InflightLease>)
    where
        F: FnMut(usize) -> bool,
    {
        for band in self.bands.iter() {
            let mut live_members: Vec<usize> = Vec::with_capacity(band.members.len());
            for &endpoint_index in band.members.iter() {
                let ep = &self.endpoints[endpoint_index];
                // NG-08: a peer ramped to effective weight 0 during slow_start
                // is not a candidate, same as in the weighted paths.
                if ep.admin_selectable()
                    && ep.passive.selectable()
                    && ep.passive.ramped_weight(i64::from(ep.admin.weight())) > 0
                    && live(endpoint_index)
                {
                    live_members.push(endpoint_index);
                }
            }
            if live_members.is_empty() {
                continue;
            }
            let pick = if live_members.len() == 1 {
                live_members[0]
            } else {
                let a = self.p2c_sample(live_members.len());
                let mut b = self.p2c_sample(live_members.len());
                if b == a {
                    b = (a + 1) % live_members.len();
                }
                let (ia, ib) = (live_members[a], live_members[b]);
                // NG-08: compare load per unit of ramped weight (conns/weight),
                // consistent with least_conn.
                let ramped = |i: usize| {
                    u128::try_from(
                        self.endpoints[i]
                            .passive
                            .ramped_weight(i64::from(self.endpoints[i].admin.weight())),
                    )
                    .unwrap_or(1)
                    .max(1)
                };
                if u128::from(self.inflight[ia].load(Ordering::Relaxed)) * ramped(ib)
                    <= u128::from(self.inflight[ib].load(Ordering::Relaxed)) * ramped(ia)
                {
                    ia
                } else {
                    ib
                }
            };
            let priority = self.endpoints[pick].priority;
            return (
                SelectionOutcome::Selected {
                    endpoint_index: pick,
                    priority,
                },
                self.mint_lease(pick),
            );
        }
        (SelectionOutcome::NoEligibleEndpoint, None)
    }

    /// In-flight count for `endpoint_index` (tests/observability).
    pub fn inflight_of(&self, endpoint_index: usize) -> u64 {
        self.inflight
            .get(endpoint_index)
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    fn mint_lease(self: &Arc<Self>, endpoint_index: usize) -> Option<InflightLease> {
        let counter = Arc::clone(self.inflight.get(endpoint_index)?);
        counter.fetch_add(1, Ordering::Relaxed);
        Some(InflightLease { counter })
    }

    /// xorshift64 scramble over an atomic counter — uniform enough for LB sampling.
    fn p2c_sample(&self, bound: usize) -> usize {
        let mut x = self
            .p2c_seed
            .fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)
            | 1;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        ((x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32) as usize) % bound
    }
}

/// FNV-1a over key ‖ endpoint_id ‖ vpoint — deterministic HRW score.
fn hrw_hash(key: &[u8], id: &[u8], vpoint: u32) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in key
        .iter()
        .chain(id.iter())
        .chain(vpoint.to_le_bytes().iter())
    {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn make_band(priority: u32, members: &[usize], _endpoints: &[EndpointSpec]) -> Band {
    let current = vec![0i64; members.len()].into_boxed_slice();
    Band {
        priority,
        members: members.to_vec().into_boxed_slice(),
        current: Mutex::new(current),
    }
}

/// Scale weights into i64 with non-zero preservation and overflow control.
fn normalize_weights(weights: &[u32]) -> Vec<i64> {
    if weights.is_empty() {
        return Vec::new();
    }
    let sum: u128 = weights.iter().map(|&w| u128::from(w)).sum();
    if sum == 0 {
        return weights.iter().map(|_| 0i64).collect();
    }
    const MAX_TOTAL: u128 = (i64::MAX / 4) as u128;
    if sum <= MAX_TOTAL {
        return weights.iter().map(|&w| i64::from(w)).collect();
    }
    let scale = (sum / MAX_TOTAL).max(1);
    weights
        .iter()
        .map(|&w| {
            let scaled = (u128::from(w) / scale).max(1);
            i64::try_from(scaled).unwrap_or(i64::MAX / 4)
        })
        .collect()
}

/// Transport pool identity: scheme + host + port from http URI.
pub fn endpoint_transport_identity(http_uri: &str) -> Option<String> {
    let uri: http::Uri = http_uri.parse().ok()?;
    let scheme = uri.scheme_str().unwrap_or("http");
    let host = uri.host()?;
    let port = uri
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some(format!("{scheme}://{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use std::thread;

    fn ep(id: &str, uri: &str, weight: u32, priority: u32, enabled: bool) -> EndpointSpec {
        EndpointSpec {
            endpoint_id: id.into(),
            http_uri: uri.into(),
            weight,
            priority,
            admin_enabled: enabled,
            passive: Arc::new(PassivePeer::disabled()),
            admin: Arc::new(EndpointAdmin::new(enabled, weight)),
        }
    }

    fn ep_passive(
        id: &str,
        uri: &str,
        weight: u32,
        priority: u32,
        max_fails: u32,
        fail_timeout_ms: u64,
        slow_start_ms: u64,
    ) -> EndpointSpec {
        EndpointSpec {
            endpoint_id: id.into(),
            http_uri: uri.into(),
            weight,
            priority,
            admin_enabled: true,
            passive: Arc::new(PassivePeer::new(max_fails, fail_timeout_ms, slow_start_ms)),
            admin: Arc::new(EndpointAdmin::new(true, weight)),
        }
    }

    #[test]
    fn one_endpoint_always_selected() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![ep("a", "http://10.0.0.1:80", 1, 0, true)],
        );
        for _ in 0..100 {
            match s.select() {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "a");
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn equal_weight_approximately_balanced() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        let n = 10_000;
        let mut counts = HashMap::new();
        for _ in 0..n {
            match s.select() {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    let id = s.endpoints()[endpoint_index].endpoint_id.clone();
                    *counts.entry(id).or_insert(0usize) += 1;
                }
                SelectionOutcome::NoEligibleEndpoint => panic!("expected selection"),
            }
        }
        let a = counts["a"] as f64;
        let b = counts["b"] as f64;
        let ratio = (a / b - 1.0).abs();
        assert!(ratio < 0.05, "a={a} b={b} ratio_err={ratio}");
    }

    #[test]
    fn filtered_skips_heavy_peer_preserves_equal_wrr() {
        // LA-CAP045-001: heavy peer filtered → remaining equal weights stay ~1:1.
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("heavy", "http://10.0.0.1:80", 100, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
                ep("c", "http://10.0.0.3:80", 1, 0, true),
            ],
        );
        let n = 200;
        let mut counts = HashMap::new();
        for _ in 0..n {
            match s.select_filtered(|idx| s.endpoints()[idx].endpoint_id != "heavy") {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    let id = s.endpoints()[endpoint_index].endpoint_id.clone();
                    *counts.entry(id).or_insert(0usize) += 1;
                }
                SelectionOutcome::NoEligibleEndpoint => panic!("expected selection"),
            }
        }
        assert_eq!(counts.get("heavy").copied().unwrap_or(0), 0);
        let b = counts["b"];
        let c = counts["c"];
        assert_eq!(b + c, n);
        // Exact SWWR equal-weight cycle → 100:100 over 200.
        assert_eq!(b, 100, "b={b} c={c}");
        assert_eq!(c, 100, "b={b} c={c}");
    }

    #[test]
    fn weighted_1_to_3_distribution() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 3, 0, true),
            ],
        );
        let n = 10_000;
        let mut counts = HashMap::new();
        for _ in 0..n {
            if let SelectionOutcome::Selected { endpoint_index, .. } = s.select() {
                let id = s.endpoints()[endpoint_index].endpoint_id.clone();
                *counts.entry(id).or_insert(0usize) += 1;
            }
        }
        let ratio = counts["b"] as f64 / counts["a"] as f64;
        assert!(
            (ratio - 3.0).abs() < 0.15,
            "expected ~3.0 got {ratio} counts={counts:?}"
        );
    }

    #[test]
    fn weighted_1_to_10_distribution() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 10, 0, true),
            ],
        );
        let n = 10_000;
        let mut counts = HashMap::new();
        for _ in 0..n {
            if let SelectionOutcome::Selected { endpoint_index, .. } = s.select() {
                let id = s.endpoints()[endpoint_index].endpoint_id.clone();
                *counts.entry(id).or_insert(0usize) += 1;
            }
        }
        let ratio = counts["b"] as f64 / counts["a"] as f64;
        assert!(
            (ratio - 10.0).abs() < 0.5,
            "expected ~10 got {ratio} counts={counts:?}"
        );
    }

    #[test]
    fn zero_weight_never_selected() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 0, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        for _ in 0..200 {
            match s.select() {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "b");
                }
                SelectionOutcome::NoEligibleEndpoint => panic!("b should be eligible"),
            }
        }
    }

    #[test]
    fn disabled_never_selected() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 10, 0, false),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        for _ in 0..100 {
            match s.select() {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "b");
                }
                SelectionOutcome::NoEligibleEndpoint => panic!("expected b"),
            }
        }
    }

    #[test]
    fn priority_band_isolation() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("low", "http://10.0.0.1:80", 100, 10, true),
                ep("high", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        for _ in 0..200 {
            match s.select() {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "high");
                }
                SelectionOutcome::NoEligibleEndpoint => panic!("expected high"),
            }
        }
    }

    #[test]
    fn failover_to_next_eligible_band() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("pref", "http://10.0.0.1:80", 1, 0, false),
                ep("backup", "http://10.0.0.2:80", 1, 5, true),
            ],
        );
        match s.select() {
            SelectionOutcome::Selected { endpoint_index, .. } => {
                assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "backup");
            }
            SelectionOutcome::NoEligibleEndpoint => panic!("expected backup"),
        }
    }

    #[test]
    fn recovery_to_preferred_band_on_rebuild() {
        let down = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("pref", "http://10.0.0.1:80", 1, 0, false),
                ep("backup", "http://10.0.0.2:80", 1, 5, true),
            ],
        );
        assert_eq!(down.preferred_priority(), Some(5));
        let up = EndpointSelector::build(
            2,
            FailoverMode::PriorityBands,
            vec![
                ep("pref", "http://10.0.0.1:80", 1, 0, true),
                ep("backup", "http://10.0.0.2:80", 1, 5, true),
            ],
        );
        assert_eq!(up.preferred_priority(), Some(0));
        match up.select() {
            SelectionOutcome::Selected { endpoint_index, .. } => {
                assert_eq!(up.endpoints()[endpoint_index].endpoint_id, "pref");
            }
            SelectionOutcome::NoEligibleEndpoint => panic!("expected pref"),
        }
    }

    #[test]
    fn no_eligible_and_empty() {
        let empty = EndpointSelector::build(1, FailoverMode::PriorityBands, vec![]);
        assert!(matches!(
            empty.select(),
            SelectionOutcome::NoEligibleEndpoint
        ));
        let all_disabled = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![ep("a", "http://10.0.0.1:80", 1, 0, false)],
        );
        assert!(matches!(
            all_disabled.select(),
            SelectionOutcome::NoEligibleEndpoint
        ));
    }

    #[test]
    fn removed_endpoint_never_selected_after_rebuild() {
        let s = EndpointSelector::build(
            2,
            FailoverMode::None,
            vec![ep("b", "http://10.0.0.2:80", 1, 0, true)],
        );
        for _ in 0..50 {
            match s.select() {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "b");
                }
                SelectionOutcome::NoEligibleEndpoint => panic!("expected b"),
            }
        }
    }

    #[test]
    fn overflow_max_weight_no_panic() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", u32::MAX, 0, true),
                ep("b", "http://10.0.0.2:80", u32::MAX, 0, true),
            ],
        );
        for _ in 0..1000 {
            let _ = s.select();
        }
    }

    #[test]
    fn concurrent_selection_no_panic() {
        let s = Arc::new(EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
                ep("c", "http://10.0.0.3:80", 1, 0, true),
            ],
        ));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = Arc::clone(&s);
            handles.push(thread::spawn(move || {
                for _ in 0..2_000 {
                    match s.select() {
                        SelectionOutcome::Selected { .. } => {}
                        SelectionOutcome::NoEligibleEndpoint => panic!("unexpected"),
                    }
                }
            }));
        }
        for h in handles {
            h.join().expect("thread");
        }
    }

    #[test]
    fn pool_key_differs_per_endpoint_authority() {
        let a = endpoint_transport_identity("http://10.0.0.1:8080").unwrap();
        let b = endpoint_transport_identity("http://10.0.0.2:8080").unwrap();
        assert_ne!(a, b);
        assert_eq!(
            endpoint_transport_identity("http://10.0.0.1:8080").unwrap(),
            a
        );
        // Future TLS identity is not collapsed into UserBackendId — scheme differs.
        let https = endpoint_transport_identity("https://10.0.0.1:8443").unwrap();
        assert_ne!(a, https);
        assert!(https.starts_with("https://"));
    }

    #[test]
    fn distinct_selectors_have_independent_band_locks() {
        // CROSS_BACKEND_LOCK_CONTENTION = NO_BY_DESIGN_AND_TEST
        // Two selectors = two backends; concurrent select cannot share a band mutex.
        let a = Arc::new(EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a1", "http://10.0.0.1:80", 1, 0, true),
                ep("a2", "http://10.0.0.2:80", 1, 0, true),
            ],
        ));
        let b = Arc::new(EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("b1", "http://10.0.1.1:80", 1, 0, true),
                ep("b2", "http://10.0.1.2:80", 1, 0, true),
            ],
        ));
        let mut handles = Vec::new();
        for sel in [a, b] {
            for _ in 0..4 {
                let sel = Arc::clone(&sel);
                handles.push(thread::spawn(move || {
                    for _ in 0..2_000 {
                        assert!(matches!(sel.select(), SelectionOutcome::Selected { .. }));
                    }
                }));
            }
        }
        for h in handles {
            h.join().expect("thread");
        }
    }

    #[test]
    fn weight_and_priority_change_reflected_on_rebuild() {
        let s1 = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        let mut counts = HashMap::new();
        for _ in 0..200 {
            if let SelectionOutcome::Selected { endpoint_index, .. } = s1.select() {
                *counts
                    .entry(s1.endpoints()[endpoint_index].endpoint_id.clone())
                    .or_insert(0usize) += 1;
            }
        }
        assert!(counts["a"] > 0 && counts["b"] > 0);

        // Weight change: b becomes 10×.
        let s2 = EndpointSelector::build(
            2,
            FailoverMode::PriorityBands,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 10, 0, true),
            ],
        );
        let mut counts2 = HashMap::new();
        for _ in 0..2_000 {
            if let SelectionOutcome::Selected { endpoint_index, .. } = s2.select() {
                *counts2
                    .entry(s2.endpoints()[endpoint_index].endpoint_id.clone())
                    .or_insert(0usize) += 1;
            }
        }
        assert!(counts2["b"] > counts2["a"] * 5);

        // Priority change: a demoted → only b selected.
        let s3 = EndpointSelector::build(
            3,
            FailoverMode::PriorityBands,
            vec![
                ep("a", "http://10.0.0.1:80", 100, 5, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        for _ in 0..50 {
            match s3.select() {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    assert_eq!(s3.endpoints()[endpoint_index].endpoint_id, "b");
                }
                SelectionOutcome::NoEligibleEndpoint => panic!("expected b"),
            }
        }
    }

    #[test]
    fn newly_added_endpoint_becomes_selectable() {
        let s1 = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![ep("a", "http://10.0.0.1:80", 1, 0, true)],
        );
        match s1.select() {
            SelectionOutcome::Selected { endpoint_index, .. } => {
                assert_eq!(s1.endpoints()[endpoint_index].endpoint_id, "a");
            }
            SelectionOutcome::NoEligibleEndpoint => panic!("expected a"),
        }
        let s2 = EndpointSelector::build(
            2,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("c", "http://10.0.0.3:80", 1, 0, true),
            ],
        );
        let mut seen = HashSet::new();
        for _ in 0..40 {
            if let SelectionOutcome::Selected { endpoint_index, .. } = s2.select() {
                seen.insert(s2.endpoints()[endpoint_index].endpoint_id.clone());
            }
        }
        assert!(seen.contains("a") && seen.contains("c"));
    }

    #[test]
    fn weight_normalization_preserves_relative_ratios() {
        // WRR_WEIGHT_NORMALIZATION_TESTS = PASS
        // Large equal weights still alternate ~1:1 after scale-down.
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", u32::MAX / 2, 0, true),
                ep("b", "http://10.0.0.2:80", u32::MAX / 2, 0, true),
            ],
        );
        let mut counts = HashMap::new();
        for _ in 0..10_000 {
            if let SelectionOutcome::Selected { endpoint_index, .. } = s.select() {
                *counts
                    .entry(s.endpoints()[endpoint_index].endpoint_id.clone())
                    .or_insert(0usize) += 1;
            }
        }
        let ratio = (counts["a"] as f64 / counts["b"] as f64 - 1.0).abs();
        assert!(ratio < 0.05, "normalized equal weights drifted: {counts:?}");
    }

    #[test]
    fn deterministic_construction_band_order() {
        let s1 = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("z", "http://10.0.0.3:80", 1, 1, true),
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("m", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        let s2 = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("m", "http://10.0.0.2:80", 1, 0, true),
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("z", "http://10.0.0.3:80", 1, 0, true),
            ],
        );
        assert_eq!(s1.preferred_priority(), Some(0));
        assert_eq!(s2.preferred_priority(), Some(0));
        // Within preferred band, endpoint_id order a then m.
        let mut seen = Vec::new();
        for _ in 0..4 {
            if let SelectionOutcome::Selected { endpoint_index, .. } = s1.select() {
                seen.push(s1.endpoints()[endpoint_index].endpoint_id.clone());
            }
        }
        assert!(seen.iter().all(|id| id == "a" || id == "m"));
        assert!(!seen.iter().any(|id| id == "z"));
    }

    // ---------- NG-07: keyed selection, least_conn, P2C, leases ----------

    fn two_live() -> EndpointSelector {
        EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
            ],
        )
    }

    fn picked_id(s: &EndpointSelector, out: SelectionOutcome) -> String {
        match out {
            SelectionOutcome::Selected { endpoint_index, .. } => {
                s.endpoints()[endpoint_index].endpoint_id.clone()
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn hrw_same_key_same_endpoint() {
        let s = two_live();
        let first = picked_id(&s, s.select_key_filtered("/api/x", |_| true));
        for _ in 0..50 {
            assert_eq!(
                picked_id(&s, s.select_key_filtered("/api/x", |_| true)),
                first
            );
        }
    }

    #[test]
    fn hrw_remaps_only_departed_endpoint_keys() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
                ep("c", "http://10.0.0.3:80", 1, 0, true),
            ],
        );
        // Baseline key→endpoint map with all three live.
        let keys: Vec<String> = (0..300).map(|i| format!("key-{i}")).collect();
        let baseline: HashMap<String, String> = keys
            .iter()
            .map(|k| (k.clone(), picked_id(&s, s.select_key_filtered(k, |_| true))))
            .collect();
        // "a" dead: its keys must remap to b/c; keys owned by b/c must NOT move.
        let dead = s
            .endpoints()
            .iter()
            .position(|e| e.endpoint_id == "a")
            .expect("a");
        for (key, owner) in &baseline {
            let now = picked_id(&s, s.select_key_filtered(key, |i| i != dead));
            assert_ne!(now, "a", "key {key} still routed to dead endpoint");
            if owner != "a" {
                assert_eq!(&now, owner, "key {key} remapped without cause");
            }
        }
    }

    #[test]
    fn hrw_weight_biases_distribution() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("light", "http://10.0.0.1:80", 1, 0, true),
                ep("heavy", "http://10.0.0.2:80", 4, 0, true),
            ],
        );
        let mut heavy_wins = 0usize;
        for i in 0..400 {
            if picked_id(&s, s.select_key_filtered(&format!("k{i}"), |_| true)) == "heavy" {
                heavy_wins += 1;
            }
        }
        assert!(
            heavy_wins > 200,
            "weight=4 endpoint should dominate distribution, got {heavy_wins}/400"
        );
    }

    #[test]
    fn hrw_respects_priority_band() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("hot", "http://10.0.0.1:80", 1, 0, true),
                ep("spare", "http://10.0.0.2:80", 1, 1, true),
            ],
        );
        for i in 0..50 {
            assert_eq!(
                picked_id(&s, s.select_key_filtered(&format!("k{i}"), |_| true)),
                "hot",
                "lower band must not win while preferred band is live"
            );
        }
    }

    #[test]
    fn select_by_id_pins_and_respects_live() {
        let s = two_live();
        assert_eq!(picked_id(&s, s.select_by_id_filtered("b", |_| true)), "b");
        let b = s
            .endpoints()
            .iter()
            .position(|e| e.endpoint_id == "b")
            .expect("b");
        assert!(matches!(
            s.select_by_id_filtered("b", |i| i != b),
            SelectionOutcome::NoEligibleEndpoint
        ));
        assert!(matches!(
            s.select_by_id_filtered("nonexistent", |_| true),
            SelectionOutcome::NoEligibleEndpoint
        ));
    }

    #[test]
    fn select_by_id_skips_disabled_endpoint() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, false),
            ],
        );
        assert!(matches!(
            s.select_by_id_filtered("b", |_| true),
            SelectionOutcome::NoEligibleEndpoint
        ));
    }

    #[test]
    fn least_conn_prefers_idle_and_lease_counts() {
        let s = Arc::new(two_live());
        let (out, lease) = s.select_least_conn(|_| true);
        let first = picked_id(&s, out);
        let first_idx = s
            .endpoints()
            .iter()
            .position(|e| e.endpoint_id == first)
            .expect("idx");
        assert_eq!(s.inflight_of(first_idx), 1);
        // While the lease is held, the other endpoint has fewer in-flight.
        let (out2, lease2) = s.select_least_conn(|_| true);
        let second = picked_id(&s, out2);
        assert_ne!(first, second);
        let second_idx = s
            .endpoints()
            .iter()
            .position(|e| e.endpoint_id == second)
            .expect("idx2");
        assert_eq!(s.inflight_of(second_idx), 1);
        drop(lease);
        drop(lease2);
        assert_eq!(s.inflight_of(first_idx), 0);
        assert_eq!(s.inflight_of(second_idx), 0);
    }

    #[test]
    fn least_conn_no_eligible_mints_no_lease() {
        let s = Arc::new(two_live());
        let (out, lease) = s.select_least_conn(|_| false);
        assert!(matches!(out, SelectionOutcome::NoEligibleEndpoint));
        assert!(lease.is_none());
    }

    #[test]
    fn p2c_prefers_less_loaded() {
        let s = Arc::new(two_live());
        let (out, lease) = s.select_power_of_two(|_| true);
        let first = picked_id(&s, out);
        // With one endpoint loaded, P2C between two must pick the idle one.
        for _ in 0..20 {
            let (out2, lease2) = s.select_power_of_two(|_| true);
            assert_ne!(picked_id(&s, out2), first);
            drop(lease2);
        }
        drop(lease);
    }

    #[test]
    fn p2c_single_live_member_picks_it() {
        let s = Arc::new(two_live());
        let a = s
            .endpoints()
            .iter()
            .position(|e| e.endpoint_id == "a")
            .expect("a");
        let (out, _lease) = s.select_power_of_two(|i| i == a);
        assert_eq!(picked_id(&s, out), "a");
    }

    // ---------- NG-08: passive health (max_fails / fail_timeout / slow_start) ----------

    #[test]
    fn passive_max_fails_threshold_bans() {
        // max_fails=2: first failure counts, second arms the ban.
        let p = PassivePeer::new(2, 10_000, 0);
        p.record(false);
        assert!(!p.is_banned(), "one fail below threshold must not ban");
        assert_eq!(p.fails(), 1);
        p.record(false);
        assert!(p.is_banned(), "reaching max_fails must ban");
        assert_eq!(p.fails(), 0, "counter resets when the ban arms");
    }

    #[test]
    fn passive_success_resets_consecutive_fails() {
        let p = PassivePeer::new(2, 10_000, 0);
        p.record(false);
        p.record(true);
        assert_eq!(p.fails(), 0, "success clears the consecutive counter");
        p.record(false);
        assert!(!p.is_banned(), "counter restarted — still below threshold");
    }

    #[test]
    fn passive_max_fails_zero_disables() {
        let p = PassivePeer::disabled();
        for _ in 0..10 {
            p.record(false);
        }
        assert!(!p.is_banned() && p.selectable());
    }

    #[test]
    fn passive_ban_expires_after_fail_timeout() {
        let p = PassivePeer::new(1, 30, 0);
        p.record(false);
        assert!(p.is_banned());
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(p.selectable(), "ban must expire after fail_timeout");
    }

    #[test]
    fn passive_record_during_ban_does_not_extend() {
        let p = PassivePeer::new(1, 60, 0);
        p.record(false);
        assert!(p.is_banned());
        // Late in-flight failure while already banned must not re-arm the
        // window — ban still ends at the original deadline.
        p.record(false);
        std::thread::sleep(std::time::Duration::from_millis(80));
        assert!(p.selectable());
    }

    #[test]
    fn passive_slow_start_ramps_weight() {
        // 30ms ban, 400ms ramp: after recovery weight < full and rises.
        let p = PassivePeer::new(1, 30, 400);
        p.record(false);
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(p.selectable());
        let early = p.ramped_weight(100);
        assert!((1..100).contains(&early), "ramp start weight, got {early}");
        std::thread::sleep(std::time::Duration::from_millis(200));
        let later = p.ramped_weight(100);
        assert!(later > early, "ramp must be monotonic: {early} → {later}");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(p.ramped_weight(100), 100, "ramp completes to full weight");
    }

    #[test]
    fn passive_fails_outside_window_restart_count() {
        // nginx: fails are counted inside fail_timeout — a gap longer than the
        // window restarts the counter instead of accumulating stale fails.
        let p = PassivePeer::new(2, 40, 0);
        p.record(false);
        assert_eq!(p.fails(), 1);
        std::thread::sleep(std::time::Duration::from_millis(80));
        p.record(false);
        assert_eq!(p.fails(), 1, "stale fail outside window must not count");
        assert!(p.selectable(), "sparse fails must not ban");
    }

    #[test]
    fn passive_fails_inside_window_accumulate_and_ban() {
        let p = PassivePeer::new(2, 60_000, 0);
        p.record(false);
        p.record(false);
        assert!(p.is_banned(), "fails inside the window must ban");
    }

    #[test]
    fn passive_no_slow_start_full_weight_after_ban() {
        let p = PassivePeer::new(1, 30, 0);
        p.record(false);
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert_eq!(p.ramped_weight(100), 100);
    }

    #[test]
    fn passive_never_banned_full_weight() {
        let p = PassivePeer::new(1, 10_000, 5_000);
        assert_eq!(p.ramped_weight(7), 7, "ramp only applies after a ban");
    }

    #[test]
    fn banned_peer_excluded_from_selection() {
        let dead = ep_passive("dead", "http://10.0.0.9:80", 1, 0, 1, 60_000, 0);
        let dead_peer = Arc::clone(&dead.passive);
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![dead, ep("live", "http://10.0.0.2:80", 1, 0, true)],
        );
        dead_peer.record(false);
        for _ in 0..50 {
            match s.select() {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "live");
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn banned_sole_peer_yields_no_eligible() {
        let dead = ep_passive("dead", "http://10.0.0.9:80", 1, 0, 1, 60_000, 0);
        dead.passive.record(false);
        let s = EndpointSelector::build(1, FailoverMode::None, vec![dead]);
        assert_eq!(s.select(), SelectionOutcome::NoEligibleEndpoint);
    }

    #[test]
    fn banned_peer_rejoins_after_fail_timeout() {
        let flappy = ep_passive("flap", "http://10.0.0.9:80", 1, 0, 1, 30, 0);
        flappy.passive.record(false);
        let s = EndpointSelector::build(1, FailoverMode::None, vec![flappy]);
        assert_eq!(s.select(), SelectionOutcome::NoEligibleEndpoint);
        std::thread::sleep(std::time::Duration::from_millis(60));
        match s.select() {
            SelectionOutcome::Selected { endpoint_index, .. } => {
                assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "flap");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn banned_peer_excluded_in_priority_band() {
        // Banned preferred-band member → traffic falls to the backup band.
        let dead = ep_passive("dead", "http://10.0.0.9:80", 1, 0, 1, 60_000, 0);
        dead.passive.record(false);
        let s = EndpointSelector::build(
            1,
            FailoverMode::PriorityBands,
            vec![dead, ep("backup", "http://10.0.0.2:80", 1, 1, true)],
        );
        match s.select() {
            SelectionOutcome::Selected {
                endpoint_index,
                priority,
            } => {
                assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "backup");
                assert_eq!(priority, 1);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn pinned_banned_peer_falls_back() {
        // cookie_affinity pin to a banned endpoint → NoEligible for the pin
        // (caller falls back to band selection).
        let dead = ep_passive("dead", "http://10.0.0.9:80", 1, 0, 1, 60_000, 0);
        dead.passive.record(false);
        let s = EndpointSelector::build(1, FailoverMode::None, vec![dead]);
        assert_eq!(
            s.select_by_id_filtered("dead", |_| true),
            SelectionOutcome::NoEligibleEndpoint
        );
    }

    #[test]
    fn least_conn_skips_banned_peer() {
        let dead = ep_passive("dead", "http://10.0.0.9:80", 1, 0, 1, 60_000, 0);
        dead.passive.record(false);
        let s = Arc::new(EndpointSelector::build_policy(
            1,
            FailoverMode::None,
            vec![dead, ep("live", "http://10.0.0.2:80", 1, 0, true)],
            exyonq_module_api::proxy_dispatch::ProxySelectionPolicy::LeastConn,
            None,
            None,
        ));
        for _ in 0..20 {
            let (out, _lease) = s.select_least_conn(|_| true);
            match out {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "live");
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    /// NG-08: nginx weighs conns/effective-weight — a just-recovered peer mid
    /// slow_start must not absorb full least_conn preference while ramped to 0.
    #[test]
    fn least_conn_deprioritizes_ramping_peer() {
        let flappy = ep_passive("flap", "http://10.0.0.9:80", 1, 0, 1, 30, 30_000);
        flappy.passive.record(false);
        std::thread::sleep(std::time::Duration::from_millis(60));
        let s = Arc::new(EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![flappy, ep("live", "http://10.0.0.2:80", 1, 0, true)],
        ));
        for _ in 0..20 {
            let (out, _lease) = s.select_least_conn(|_| true);
            match out {
                SelectionOutcome::Selected { endpoint_index, .. } => {
                    assert_eq!(s.endpoints()[endpoint_index].endpoint_id, "live");
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn ramping_peer_gets_reduced_share() {
        // After ban expiry with slow_start, the recovering peer must receive
        // strictly less than its equal-weight share.
        let flappy = ep_passive("flap", "http://10.0.0.9:80", 1, 0, 1, 30, 30_000);
        flappy.passive.record(false);
        std::thread::sleep(std::time::Duration::from_millis(60));
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![flappy, ep("live", "http://10.0.0.2:80", 1, 0, true)],
        );
        let mut flap_count = 0usize;
        for _ in 0..200 {
            if let SelectionOutcome::Selected { endpoint_index, .. } = s.select() {
                if s.endpoints()[endpoint_index].endpoint_id == "flap" {
                    flap_count += 1;
                }
            }
        }
        assert!(
            flap_count < 60,
            "recovering peer must stay well below 50% share, got {flap_count}/200"
        );
    }

    fn selected_id(s: &EndpointSelector) -> Option<String> {
        match s.select() {
            SelectionOutcome::Selected { endpoint_index, .. } => {
                Some(s.endpoints()[endpoint_index].endpoint_id.clone())
            }
            SelectionOutcome::NoEligibleEndpoint => None,
        }
    }

    #[test]
    fn ap01_member_set_disables_without_rebuild() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        let m = s.member_set("b", Some(false), None).expect("member exists");
        assert!(!m.enabled);
        for _ in 0..20 {
            assert_eq!(selected_id(&s).as_deref(), Some("a"));
        }
        // Re-enable hot — the member rejoins the same selector.
        s.member_set("b", Some(true), None);
        let mut saw_b = false;
        for _ in 0..50 {
            saw_b |= selected_id(&s).as_deref() == Some("b");
        }
        assert!(saw_b, "re-enabled member must be selectable again");
    }

    #[test]
    fn ap01_member_weight_zero_drains() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        s.member_set("b", None, Some(0));
        for _ in 0..20 {
            assert_eq!(selected_id(&s).as_deref(), Some("a"));
        }
    }

    #[test]
    fn ap01_disabled_member_blocks_affinity_pin() {
        // select_by_id honors the same live gate — a pinned route token to a
        // disabled member cannot resurrect it.
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, true),
            ],
        );
        s.member_set("b", Some(false), None);
        assert!(matches!(
            s.select_by_id_filtered("b", |_| true),
            SelectionOutcome::NoEligibleEndpoint
        ));
    }

    #[test]
    fn ap01_member_list_reflects_admin() {
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 2, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, false),
            ],
        );
        let list = s.member_list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].endpoint_id, "a");
        assert!(list[0].enabled && list[0].weight == 2);
        assert_eq!(list[1].endpoint_id, "b");
        assert!(!list[1].enabled && list[1].weight == 1);
        assert_eq!(s.member_set("ghost", Some(true), None), None);
    }

    #[test]
    fn ap01_build_disabled_member_rejoins_on_enable() {
        // Config-disabled member is in the band table — hot enable makes it
        // selectable without a rebuild.
        let s = EndpointSelector::build(
            1,
            FailoverMode::None,
            vec![
                ep("a", "http://10.0.0.1:80", 1, 0, true),
                ep("b", "http://10.0.0.2:80", 1, 0, false),
            ],
        );
        for _ in 0..10 {
            assert_eq!(selected_id(&s).as_deref(), Some("a"));
        }
        s.member_set("b", Some(true), None);
        let mut saw_b = false;
        for _ in 0..50 {
            saw_b |= selected_id(&s).as_deref() == Some("b");
        }
        assert!(saw_b, "build-disabled member must rejoin on enable");
    }
}
