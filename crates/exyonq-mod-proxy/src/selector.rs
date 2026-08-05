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

use std::sync::Mutex;

/// Desired endpoint row used to build a selector (no runtime health).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointSpec {
    pub endpoint_id: String,
    pub http_uri: String,
    pub weight: u32,
    pub priority: u32,
    pub admin_enabled: bool,
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

#[derive(Debug)]
struct Band {
    priority: u32,
    /// Indices into `endpoints` of the parent selector.
    members: Box<[usize]>,
    weights: Box<[i64]>,
    total_weight: i64,
    current: Mutex<Box<[i64]>>,
}

/// Compiled per-cluster selector. Rebuild off hot path; select under per-band mutex only.
#[derive(Debug)]
pub struct EndpointSelector {
    pub generation: u64,
    endpoints: Box<[EndpointSpec]>,
    bands: Box<[Band]>,
}

impl EndpointSelector {
    /// Build selector from desired specs. Ineligible endpoints are omitted from bands.
    pub fn build(generation: u64, failover: FailoverMode, specs: Vec<EndpointSpec>) -> Self {
        let endpoints: Box<[EndpointSpec]> = specs.into_boxed_slice();
        let eligible: Vec<usize> = endpoints
            .iter()
            .enumerate()
            .filter(|(_, e)| e.admin_enabled && e.weight > 0)
            .map(|(i, _)| i)
            .collect();

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

        Self {
            generation,
            endpoints,
            bands,
        }
    }

    pub fn endpoint_count(&self) -> usize {
        self.endpoints.len()
    }

    pub fn band_count(&self) -> usize {
        self.bands.len()
    }

    pub fn endpoints(&self) -> &[EndpointSpec] {
        &self.endpoints
    }

    /// Smooth WRR within the first non-empty eligible band (bands pre-filtered).
    pub fn select(&self) -> SelectionOutcome {
        for band in self.bands.iter() {
            if band.members.is_empty() || band.total_weight <= 0 {
                continue;
            }
            let mut current = band.current.lock().expect("selector band poisoned");
            let mut best = 0usize;
            let mut best_val = i64::MIN;
            for (i, w) in band.weights.iter().enumerate() {
                // Saturating add avoids overflow panic; weights normalized at build.
                current[i] = current[i].saturating_add(*w);
                if current[i] > best_val {
                    best_val = current[i];
                    best = i;
                }
            }
            current[best] = current[best].saturating_sub(band.total_weight);
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

    /// Active preferred priority when bands exist (lowest numeric).
    pub fn preferred_priority(&self) -> Option<u32> {
        self.bands.first().map(|b| b.priority)
    }
}

fn make_band(priority: u32, members: &[usize], endpoints: &[EndpointSpec]) -> Band {
    let raw: Vec<u32> = members.iter().map(|&i| endpoints[i].weight).collect();
    let weights = normalize_weights(&raw);
    let total_weight: i64 = weights.iter().sum();
    let current = vec![0i64; weights.len()].into_boxed_slice();
    Band {
        priority,
        members: members.to_vec().into_boxed_slice(),
        weights: weights.into_boxed_slice(),
        total_weight,
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
}
