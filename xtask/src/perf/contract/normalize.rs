use super::scenarios::{fingerprint_for_scenario, ContractScenario};
use super::schema::ProfileSpec;
use super::schema::{MetricsSnapshot, RunDocument};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct NormalizedRun {
    pub doc: RunDocument,
}

pub fn normalize_from_summary_json(
    summary: &Value,
    scenario: &ContractScenario,
    profile: &ProfileSpec,
    platform: &str,
    arch: &str,
    cpu_count: u32,
    commit: Option<String>,
) -> anyhow::Result<NormalizedRun> {
    let valid_flag = summary
        .get("valid")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let success_rate = summary
        .get("success_rate")
        .and_then(|v| v.as_f64())
        .unwrap_or(100.0);
    let loadgen_saturated = summary
        .get("loadgen_saturated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let rps = summary.get("rps").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let mut invalid_reason = None;
    let mut valid = valid_flag && rps > 0.0 && success_rate >= 99.9;
    if loadgen_saturated {
        valid = false;
        invalid_reason = Some("loadgen_saturated".into());
    }
    if rps <= 0.0 {
        valid = false;
        invalid_reason.get_or_insert_with(|| "missing_rps".into());
    }
    if success_rate < 99.9 {
        valid = false;
        invalid_reason.get_or_insert_with(|| "success_rate_below_threshold".into());
    }

    let p50_ms = summary
        .get("p50_ms")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let p95_ms = summary
        .get("p95_ms")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let p99_ms = summary
        .get("p99_ms")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if valid && (p50_ms <= 0.0 || p99_ms <= 0.0) {
        valid = false;
        invalid_reason = Some("incomplete_latency_percentiles".into());
    }

    let ram_peak_mib = summary.get("ram_peak_mib").and_then(|v| v.as_f64());
    let error_rate = (100.0 - success_rate).max(0.0);

    let metrics = MetricsSnapshot {
        requests_per_second: rps,
        p50_ms,
        p95_ms,
        p99_ms,
        error_count: if error_rate > 0.0 { 1 } else { 0 },
        error_rate,
        unexpected_5xx: 0,
        process_rss_bytes: ram_peak_mib.map(|m| (m * 1024.0 * 1024.0) as u64),
    };

    let fingerprint = fingerprint_for_scenario(scenario, profile, platform, arch, cpu_count);
    let doc = RunDocument {
        schema_version: super::schema::CONTRACT_SCHEMA_VERSION,
        contract_id: super::schema::CONTRACT_ID.into(),
        commit,
        captured_at: chrono_like_now(),
        valid,
        invalid_reason,
        fingerprint,
        metrics,
    };
    Ok(NormalizedRun { doc })
}

pub fn normalize_raw_rewrk_json(
    raw: &Value,
    scenario: &ContractScenario,
    profile: &ProfileSpec,
    platform: &str,
    arch: &str,
    cpu_count: u32,
    commit: Option<String>,
) -> anyhow::Result<NormalizedRun> {
    let summary = raw.get("summary").cloned().unwrap_or_else(|| raw.clone());
    let lat = raw
        .get("latencyPercentiles")
        .or_else(|| raw.get("latency_percentiles"));
    let mut merged = serde_json::json!({});
    if let Some(s) = summary.as_object() {
        for (k, v) in s {
            merged[k] = v.clone();
        }
    }
    if let Some(lat) = lat {
        let p50 = lat.get("p50").and_then(|v| v.as_f64()).unwrap_or(0.0) * 1000.0;
        let p95 = lat.get("p95").and_then(|v| v.as_f64()).unwrap_or(0.0) * 1000.0;
        let p99 = lat.get("p99").and_then(|v| v.as_f64()).unwrap_or(0.0) * 1000.0;
        merged["p50_ms"] = serde_json::json!(p50);
        merged["p95_ms"] = serde_json::json!(p95);
        merged["p99_ms"] = serde_json::json!(p99);
    }
    if let Some(rps) = summary.get("requestsPerSec").and_then(|v| v.as_f64()) {
        merged["rps"] = serde_json::json!(rps);
    }
    if let Some(sr) = summary.get("successRate").and_then(|v| v.as_f64()) {
        merged["success_rate"] = serde_json::json!(sr * 100.0);
    }
    merged["valid"] = serde_json::json!(true);
    normalize_from_summary_json(
        &merged, scenario, profile, platform, arch, cpu_count, commit,
    )
}

pub fn read_summary_file(path: &Path) -> anyhow::Result<Value> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::contract::scenarios::quick_profile;

    fn p1_scenario() -> ContractScenario {
        ContractScenario {
            id: "P1".into(),
            label: "Static 1 KiB".into(),
            path: "/site/1k.bin".into(),
            connections: 100,
            duration: "30s".into(),
            scheme: "http".into(),
            features: vec![],
            payload_label: "1KiB".into(),
        }
    }

    #[test]
    fn normalizes_summary_json() {
        let summary = serde_json::json!({
            "rps": 1000.0,
            "p50_ms": 1.0,
            "p95_ms": 2.0,
            "p99_ms": 3.0,
            "success_rate": 100.0,
            "valid": true
        });
        let run = normalize_from_summary_json(
            &summary,
            &p1_scenario(),
            &quick_profile(),
            "linux",
            "aarch64",
            4,
            None,
        )
        .expect("normalize");
        assert!(run.doc.valid);
        assert_eq!(run.doc.metrics.requests_per_second, 1000.0);
    }
}
