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
//! Q-10: WAF Prometheus counters — decisions by action (block ratio),
//! per-rule hits (top rules), and inspection latency histogram.
//!
//! Decisions and violations are cold-path (emit only on non-Allow or rule
//! hits); inspection timing is hot-path but only atomics. Rule labels are
//! bounded (`MAX_TRACKED_RULES`) like the Q-15 64-series cap: overflow folds
//! into `exyonq_waf_violations_labeled_dropped_total`.

use exyonq_waf_api::{WafAction, WafDecision, WafIpList, WafPhase};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const MAX_TRACKED_RULES: usize = 512;
/// Cap on labeled `[waf.ip]` entry series (same rationale as `MAX_TRACKED_RULES`).
const MAX_TRACKED_IP_ENTRIES: usize = 512;

const PHASE_COUNT: usize = 6;
const ACTION_COUNT: usize = 5;

static INSPECTIONS_BY_PHASE: [AtomicU64; PHASE_COUNT] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static DECISIONS_BY_ACTION: [AtomicU64; ACTION_COUNT] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// Inspection latency buckets in microseconds (0.1ms … 1s).
const LATENCY_BUCKETS_US: [u64; 9] = [
    100, 500, 1_000, 5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000,
];
static LATENCY_BUCKETS: [AtomicU64; 9] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static LATENCY_SUM_US: AtomicU64 = AtomicU64::new(0);
static LATENCY_COUNT: AtomicU64 = AtomicU64::new(0);

static RULE_HITS: Mutex<Option<BTreeMap<String, u64>>> = Mutex::new(None);
static RULE_HITS_DROPPED: AtomicU64 = AtomicU64::new(0);

/// One configured `[waf.ip]` entry: list, entry text, expiry.
type IpEntryKey = (WafIpList, String, Option<SystemTime>);

/// Configured `[waf.ip]` entries of the compiled generation.
static IP_ENTRIES: Mutex<Option<Vec<IpEntryKey>>> = Mutex::new(None);
/// Matched `[waf.ip]` entries keyed by `(list, entry)`.
static IP_ENTRY_HITS: Mutex<Option<BTreeMap<(String, String), u64>>> = Mutex::new(None);
static IP_ENTRY_HITS_DROPPED: AtomicU64 = AtomicU64::new(0);

/// Record the client-IP filter entries of the compiled WAF generation.
///
/// Called by the composition root on install/reload; replaces the previous set
/// so a reload with fewer entries does not leave stale series behind.
pub fn register_ip_entries(entries: &[IpEntryKey]) {
    if let Ok(mut guard) = IP_ENTRIES.lock() {
        *guard = Some(entries.to_vec());
    }
}

fn note_ip_entry_hits(decision: &WafDecision) {
    if !decision
        .violations
        .iter()
        .any(|v| v.evidence.ip_entry.is_some())
    {
        return;
    }
    if let Ok(mut guard) = IP_ENTRY_HITS.lock() {
        let map = guard.get_or_insert_with(BTreeMap::new);
        for entry in decision
            .violations
            .iter()
            .filter_map(|v| v.evidence.ip_entry.as_ref())
        {
            let key = (entry.list.as_str().to_string(), entry.entry.clone());
            if let Some(n) = map.get_mut(&key) {
                *n += 1;
            } else if map.len() < MAX_TRACKED_IP_ENTRIES {
                map.insert(key, 1);
            } else {
                IP_ENTRY_HITS_DROPPED.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn phase_idx(phase: WafPhase) -> usize {
    match phase {
        WafPhase::Connection => 0,
        WafPhase::RequestHeaders => 1,
        WafPhase::RequestBodyChunk => 2,
        WafPhase::RequestBodyComplete => 3,
        WafPhase::ResponseHeaders => 4,
        WafPhase::ResponseBodyChunk => 5,
    }
}

fn action_idx(action: WafAction) -> usize {
    match action {
        WafAction::Allow => 0,
        WafAction::Log => 1,
        WafAction::Block => 2,
        WafAction::Challenge => 3,
        WafAction::RateLimit => 4,
    }
}

const PHASE_NAMES: [&str; PHASE_COUNT] = [
    "connection",
    "request_headers",
    "request_body_chunk",
    "request_body_complete",
    "response_headers",
    "response_body_chunk",
];
const ACTION_NAMES: [&str; ACTION_COUNT] = ["allow", "log", "block", "challenge", "rate_limit"];

/// Hot path: one engine `inspect*` call completed.
pub(crate) fn note_inspection(phase: WafPhase, elapsed: Duration) {
    INSPECTIONS_BY_PHASE[phase_idx(phase)].fetch_add(1, Ordering::Relaxed);
    let micros = elapsed.as_micros() as u64;
    LATENCY_SUM_US.fetch_add(micros, Ordering::Relaxed);
    LATENCY_COUNT.fetch_add(1, Ordering::Relaxed);
    for (i, bound) in LATENCY_BUCKETS_US.iter().enumerate() {
        if micros <= *bound {
            LATENCY_BUCKETS[i].fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Cold path: a decision was emitted (called for every `finish`, Allow included).
pub(crate) fn note_decision(decision: &WafDecision) {
    DECISIONS_BY_ACTION[action_idx(decision.action)].fetch_add(1, Ordering::Relaxed);
    note_ip_entry_hits(decision);
    if decision.violations.is_empty() {
        return;
    }
    if let Ok(mut guard) = RULE_HITS.lock() {
        let map = guard.get_or_insert_with(BTreeMap::new);
        for v in &decision.violations {
            let key = v.rule_id.as_str();
            if let Some(n) = map.get_mut(key) {
                *n += 1;
            } else if map.len() < MAX_TRACKED_RULES {
                map.insert(key.to_string(), 1);
            } else {
                RULE_HITS_DROPPED.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Prometheus text for `exyonq_waf_*` series. Always emitted when registered.
pub fn append_waf_prometheus(out: &mut String) {
    out.push_str(
        "# HELP exyonq_waf_inspections_total WAF engine inspections by phase (process lifetime).\n",
    );
    out.push_str("# TYPE exyonq_waf_inspections_total counter\n");
    for (i, name) in PHASE_NAMES.iter().enumerate() {
        let n = INSPECTIONS_BY_PHASE[i].load(Ordering::Relaxed);
        if n > 0 {
            out.push_str(&format!(
                "exyonq_waf_inspections_total{{phase=\"{name}\"}} {n}\n"
            ));
        }
    }
    out.push_str("# HELP exyonq_waf_decisions_total WAF decisions by action (block ratio = block|challenge|rate_limit over all; process lifetime).\n");
    out.push_str("# TYPE exyonq_waf_decisions_total counter\n");
    for (i, name) in ACTION_NAMES.iter().enumerate() {
        let n = DECISIONS_BY_ACTION[i].load(Ordering::Relaxed);
        if n > 0 {
            out.push_str(&format!(
                "exyonq_waf_decisions_total{{action=\"{name}\"}} {n}\n"
            ));
        }
    }
    if let Ok(guard) = RULE_HITS.lock() {
        if let Some(map) = guard.as_ref() {
            out.push_str("# HELP exyonq_waf_violations_total WAF rule hits by rule_id (top rules; process lifetime).\n");
            out.push_str("# TYPE exyonq_waf_violations_total counter\n");
            for (rule, n) in map {
                out.push_str(&format!(
                    "exyonq_waf_violations_total{{rule_id=\"{}\"}} {n}\n",
                    rule.replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\n', "\\n")
                ));
            }
        }
    }
    if let Ok(guard) = IP_ENTRIES.lock() {
        if let Some(entries) = guard.as_ref() {
            if !entries.is_empty() {
                let now = SystemTime::now();
                let (mut wl_active, mut wl_expired) = (0u64, 0u64);
                let (mut bl_active, mut bl_expired) = (0u64, 0u64);
                for (list, _, until) in entries {
                    let expired = until.is_some_and(|until| now >= until);
                    match (list, expired) {
                        (WafIpList::Whitelist, false) => wl_active += 1,
                        (WafIpList::Whitelist, true) => wl_expired += 1,
                        (WafIpList::Blacklist, false) => bl_active += 1,
                        (WafIpList::Blacklist, true) => bl_expired += 1,
                    }
                }
                out.push_str("# HELP exyonq_waf_ip_filter_entries Configured client-IP list entries; active = not expired at scrape (process lifetime config).\n");
                out.push_str("# TYPE exyonq_waf_ip_filter_entries gauge\n");
                if wl_active + wl_expired > 0 {
                    out.push_str(&format!(
                        "exyonq_waf_ip_filter_entries{{list=\"whitelist\",state=\"active\"}} {wl_active}\n"
                    ));
                    out.push_str(&format!(
                        "exyonq_waf_ip_filter_entries{{list=\"whitelist\",state=\"expired\"}} {wl_expired}\n"
                    ));
                }
                if bl_active + bl_expired > 0 {
                    out.push_str(&format!(
                        "exyonq_waf_ip_filter_entries{{list=\"blacklist\",state=\"active\"}} {bl_active}\n"
                    ));
                    out.push_str(&format!(
                        "exyonq_waf_ip_filter_entries{{list=\"blacklist\",state=\"expired\"}} {bl_expired}\n"
                    ));
                }
            }
        }
    }
    if let Ok(guard) = IP_ENTRY_HITS.lock() {
        if let Some(map) = guard.as_ref() {
            if !map.is_empty() {
                out.push_str("# HELP exyonq_waf_ip_entry_hits_total Client-IP list entry matches (process lifetime).\n");
                out.push_str("# TYPE exyonq_waf_ip_entry_hits_total counter\n");
                for ((list, entry), n) in map {
                    out.push_str(&format!(
                        "exyonq_waf_ip_entry_hits_total{{list=\"{list}\",entry=\"{}\"}} {n}\n",
                        entry.replace('\\', "\\\\").replace('"', "\\\"")
                    ));
                }
            }
        }
    }
    let ip_dropped = IP_ENTRY_HITS_DROPPED.load(Ordering::Relaxed);
    if ip_dropped > 0 {
        out.push_str("# HELP exyonq_waf_ip_entry_hits_labeled_dropped_total IP-entry hits dropped after the series cap (process lifetime).\n");
        out.push_str("# TYPE exyonq_waf_ip_entry_hits_labeled_dropped_total counter\n");
        out.push_str(&format!(
            "exyonq_waf_ip_entry_hits_labeled_dropped_total {ip_dropped}\n"
        ));
    }
    let dropped = RULE_HITS_DROPPED.load(Ordering::Relaxed);
    if dropped > 0 {
        out.push_str("# HELP exyonq_waf_violations_labeled_dropped_total Rule hits dropped after the series cap (process lifetime).\n");
        out.push_str("# TYPE exyonq_waf_violations_labeled_dropped_total counter\n");
        out.push_str(&format!(
            "exyonq_waf_violations_labeled_dropped_total {dropped}\n"
        ));
    }
    out.push_str("# HELP exyonq_waf_inspection_seconds WAF engine inspection latency in seconds (process lifetime).\n");
    out.push_str("# TYPE exyonq_waf_inspection_seconds histogram\n");
    for (i, bound_us) in LATENCY_BUCKETS_US.iter().enumerate() {
        let count = LATENCY_BUCKETS[i].load(Ordering::Relaxed);
        let le = *bound_us as f64 / 1_000_000.0;
        out.push_str(&format!(
            "exyonq_waf_inspection_seconds_bucket{{le=\"{le}\"}} {count}\n"
        ));
    }
    let count = LATENCY_COUNT.load(Ordering::Relaxed);
    out.push_str(&format!(
        "exyonq_waf_inspection_seconds_bucket{{le=\"+Inf\"}} {count}\n"
    ));
    let sum = LATENCY_SUM_US.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    out.push_str(&format!("exyonq_waf_inspection_seconds_sum {sum}\n"));
    out.push_str(&format!("exyonq_waf_inspection_seconds_count {count}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_waf_api::{WafEvidence, WafIpEntryEvidence, WafRuleId, WafViolation};

    fn violation(rule: &str) -> WafViolation {
        WafViolation {
            rule_id: WafRuleId::new(rule),
            phase: WafPhase::RequestHeaders,
            message: "hit".into(),
            evidence: WafEvidence::default(),
        }
    }

    #[test]
    fn decisions_and_violations_render_prometheus() {
        note_inspection(WafPhase::RequestHeaders, Duration::from_micros(300));
        note_decision(&WafDecision::allow());
        note_decision(&WafDecision {
            action: WafAction::Block,
            violations: vec![violation("q10test-942100"), violation("q10test-942100")],
            inspection_truncated: false,
            retry_after_secs: None,
        });
        let mut out = String::new();
        append_waf_prometheus(&mut out);
        assert!(out.contains("exyonq_waf_inspections_total{phase=\"request_headers\"}"));
        assert!(out.contains("exyonq_waf_decisions_total{action=\"allow\"}"));
        assert!(out.contains("exyonq_waf_decisions_total{action=\"block\"}"));
        assert!(out.contains("exyonq_waf_violations_total{rule_id=\"q10test-942100\"} 2"));
        assert!(out.contains("exyonq_waf_inspection_seconds_bucket{le=\"0.0005\"}"));
        assert!(out.contains("exyonq_waf_inspection_seconds_count"));
    }

    #[test]
    fn ip_filter_entries_and_entry_hits_render_prometheus() {
        register_ip_entries(&[
            (
                WafIpList::Blacklist,
                "203.0.113.0/24".to_string(),
                Some(SystemTime::now() + Duration::from_secs(3600)),
            ),
            (
                WafIpList::Blacklist,
                "198.51.100.7".to_string(),
                Some(SystemTime::UNIX_EPOCH),
            ),
            (WafIpList::Whitelist, "10.0.0.0/8".to_string(), None),
        ]);
        let mut hit = violation("EXY-IP-1001");
        hit.evidence.ip_entry = Some(WafIpEntryEvidence {
            list: WafIpList::Blacklist,
            entry: "203.0.113.0/24".into(),
            reason: Some("repeated 404 scanning".into()),
        });
        note_decision(&WafDecision {
            action: WafAction::Block,
            violations: vec![hit],
            inspection_truncated: false,
            retry_after_secs: None,
        });
        let mut out = String::new();
        append_waf_prometheus(&mut out);
        assert!(out.contains("exyonq_waf_ip_filter_entries{list=\"blacklist\",state=\"active\"} 1"));
        assert!(
            out.contains("exyonq_waf_ip_filter_entries{list=\"blacklist\",state=\"expired\"} 1")
        );
        assert!(out.contains("exyonq_waf_ip_filter_entries{list=\"whitelist\",state=\"active\"} 1"));
        assert!(out.contains(
            "exyonq_waf_ip_entry_hits_total{list=\"blacklist\",entry=\"203.0.113.0/24\"} 1"
        ));
    }
}
