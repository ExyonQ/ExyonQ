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

//! Coordinated parallelism budget for Competitive Frontier H1 shards.
//!
//! Does **not** change Cap067 / Tokio defaults. Only computes a reservation
//! for the separate-process dataplane so it does not independently consume
//! all `available_parallelism()`.

/// Inclusive minimum competitive H1 shard count.
pub const MIN_SHARDS: usize = 1;
/// Inclusive maximum competitive H1 shard count (architecture ceiling).
pub const MAX_SHARDS: usize = 128;

/// Conceptual product parallelism budget (Phase 1: H1 shard reservation authority).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExyonqParallelismBudget {
    pub competitive_h1_shards: usize,
    /// Documented reservation for Tokio/control (not applied to Cap067 here).
    pub control_budget: usize,
    /// Documented reservation for Cap067/static (semantics unchanged).
    pub static_budget: usize,
    pub fastcgi_budget: usize,
    pub blocking_budget: usize,
    pub detected_parallelism: usize,
}

impl ExyonqParallelismBudget {
    /// Derive a budget from detected CPUs without taking the whole machine for H1.
    ///
    /// Policy: leave at least half of detected CPUs for control+static headroom
    /// when `detected >= 2`; clamp to `[MIN_SHARDS, MAX_SHARDS]`.
    pub fn derive(detected: usize) -> Self {
        let detected = detected.max(1);
        let control_budget = detected.min(16);
        let static_budget = detected.min(16);
        let fastcgi_budget = 4;
        let blocking_budget = 4;
        let headroom = control_budget
            .saturating_add(static_budget)
            .saturating_add(fastcgi_budget)
            .saturating_add(blocking_budget);
        // Prefer not to oversubscribe: shards ≈ max(1, detected - half) style.
        let reserved = (detected / 2).max(1);
        let shards = detected.saturating_sub(reserved).max(MIN_SHARDS);
        // Also never exceed detected, and keep below MAX.
        let competitive_h1_shards = shards.min(detected).clamp(MIN_SHARDS, MAX_SHARDS);
        let _ = headroom; // documented fields only in Phase 1
        Self {
            competitive_h1_shards,
            control_budget,
            static_budget,
            fastcgi_budget,
            blocking_budget,
            detected_parallelism: detected,
        }
    }
}

/// Resolve shard count: explicit override → else budget derivation from `available_parallelism`.
pub fn resolve_competitive_h1_shards(explicit: Option<usize>) -> usize {
    if let Some(n) = explicit {
        return n.clamp(MIN_SHARDS, MAX_SHARDS);
    }
    let detected = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(MIN_SHARDS);
    ExyonqParallelismBudget::derive(detected).competitive_h1_shards
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_hardcodes_four_as_only_default() {
        let b1 = ExyonqParallelismBudget::derive(1);
        assert_eq!(b1.competitive_h1_shards, 1);
        let b8 = ExyonqParallelismBudget::derive(8);
        assert!(b8.competitive_h1_shards >= 1);
        assert!(b8.competitive_h1_shards <= 8);
        // 8 CPU → reserved 4 → shards 4 (may equal 4 incidentally, not hardcoded sole path)
        let b16 = ExyonqParallelismBudget::derive(16);
        assert!(b16.competitive_h1_shards <= 16);
        let b128 = ExyonqParallelismBudget::derive(128);
        assert!(b128.competitive_h1_shards <= MAX_SHARDS);
    }

    #[test]
    fn explicit_override_clamped() {
        assert_eq!(resolve_competitive_h1_shards(Some(0)), 1);
        assert_eq!(resolve_competitive_h1_shards(Some(999)), MAX_SHARDS);
    }
}
