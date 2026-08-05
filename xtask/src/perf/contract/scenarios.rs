use super::schema::{Fingerprint, ProfileSpec, ToolInfo};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const CONTRACT_SCENARIO_IDS: &[&str] = &["P1", "P2", "P4", "P7"];

#[derive(Debug, Clone)]
pub struct ContractScenario {
    pub id: String,
    pub label: String,
    pub path: String,
    pub connections: u32,
    pub duration: String,
    pub scheme: String,
    pub features: Vec<String>,
    pub payload_label: String,
}

pub fn contract_scenario_ids() -> &'static [&'static str] {
    CONTRACT_SCENARIO_IDS
}

pub fn load_contract_scenarios(repo_root: &Path) -> anyhow::Result<Vec<ContractScenario>> {
    let toml_path = repo_root.join("benchmarks/scenarios/perf/scenarios.toml");
    let raw = std::fs::read_to_string(&toml_path)?;
    let table: toml::Table = toml::from_str(&raw)?;
    let mut out = Vec::new();
    for id in CONTRACT_SCENARIO_IDS {
        let key = id.to_ascii_lowercase();
        let Some(entry) = table.get(&key) else {
            anyhow::bail!("missing scenario {key} in scenarios.toml");
        };
        let entry = entry.as_table().context("scenario table")?;
        let label = entry
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or(id)
            .to_string();
        let path = entry
            .get("path")
            .and_then(|v| v.as_str())
            .context("path")?
            .to_string();
        let connections = entry
            .get("connections")
            .and_then(|v| v.as_integer())
            .unwrap_or(100) as u32;
        let duration = entry
            .get("duration")
            .and_then(|v| v.as_str())
            .unwrap_or("30s")
            .to_string();
        let scheme = entry
            .get("scheme")
            .and_then(|v| v.as_str())
            .unwrap_or("http")
            .to_string();
        let mut features = Vec::new();
        if let Some(exyonq) = entry.get("exyonq").and_then(|v| v.as_str()) {
            features.push(format!("exyonq={exyonq}"));
        }
        let payload_label = match *id {
            "P1" => "1KiB".to_string(),
            "P2" => "64KiB".to_string(),
            "P4" => "proxy-1KiB".to_string(),
            "P7" => "many-routes".to_string(),
            _ => "unknown".to_string(),
        };
        out.push(ContractScenario {
            id: id.to_string(),
            label,
            path,
            connections,
            duration,
            scheme,
            features,
            payload_label,
        });
    }
    Ok(out)
}

use anyhow::Context;

pub fn quick_profile() -> ProfileSpec {
    ProfileSpec {
        repetitions: 3,
        duration_warmup: "3s".into(),
        duration_measure: "5s".into(),
        connections: 100,
        threads: 2,
        load_mode: "ceiling".into(),
        tool: ToolInfo {
            name: "rewrk".into(),
            version: "0.3.2".into(),
        },
    }
}

pub fn full_profile() -> ProfileSpec {
    ProfileSpec {
        repetitions: 5,
        duration_warmup: "20s".into(),
        duration_measure: "30s".into(),
        connections: 100,
        threads: 2,
        load_mode: "ceiling".into(),
        tool: ToolInfo {
            name: "rewrk".into(),
            version: "0.3.2".into(),
        },
    }
}

pub fn fingerprint_for_scenario(
    scenario: &ContractScenario,
    profile: &ProfileSpec,
    platform: &str,
    arch: &str,
    cpu_count: u32,
) -> Fingerprint {
    let tls = scenario.scheme.eq_ignore_ascii_case("https");
    let epoll = std::env::var("EXYONQ_EPOLL_STATIC")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let config_fingerprint = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        scenario.id,
        scenario.path,
        scenario.connections,
        profile.duration_measure,
        profile.load_mode,
        tls,
        epoll
    );
    Fingerprint {
        schema_version: super::schema::CONTRACT_SCHEMA_VERSION,
        scenario: scenario.id.clone(),
        platform: platform.to_string(),
        arch: arch.to_string(),
        kernel: None,
        cpu_count,
        tool: profile.tool.clone(),
        connections: scenario.connections,
        threads: profile.threads,
        duration_warmup: profile.duration_warmup.clone(),
        duration_measure: profile.duration_measure.clone(),
        load_mode: profile.load_mode.clone(),
        server_target: "exyonq".into(),
        tls,
        sendfile: false,
        epoll_static: epoll,
        path: scenario.path.clone(),
        payload_label: scenario.payload_label.clone(),
        features: scenario.features.clone(),
        config_fingerprint,
    }
}

#[derive(Debug, Deserialize)]
struct SummaryJson {
    #[serde(default)]
    scenario_id: Option<String>,
    #[serde(default)]
    rps: Option<f64>,
    #[serde(default)]
    p50_ms: Option<f64>,
    #[serde(default)]
    p95_ms: Option<f64>,
    #[serde(default)]
    p99_ms: Option<f64>,
    #[serde(default)]
    success_rate: Option<f64>,
    #[serde(default)]
    valid: Option<bool>,
    #[serde(default)]
    ram_peak_mib: Option<f64>,
}

pub fn metrics_from_summary_json(
    value: &serde_json::Value,
) -> anyhow::Result<super::schema::MetricsSnapshot> {
    let s: SummaryJson = serde_json::from_value(value.clone())?;
    let success = s.success_rate.unwrap_or(100.0);
    let error_rate = (100.0 - success).max(0.0);
    Ok(super::schema::MetricsSnapshot {
        requests_per_second: s.rps.unwrap_or(0.0),
        p50_ms: s.p50_ms.unwrap_or(0.0),
        p95_ms: s.p95_ms.unwrap_or(0.0),
        p99_ms: s.p99_ms.unwrap_or(0.0),
        error_count: if error_rate > 0.0 { 1 } else { 0 },
        error_rate,
        unexpected_5xx: 0,
        process_rss_bytes: s.ram_peak_mib.map(|m| (m * 1024.0 * 1024.0) as u64),
    })
}

pub fn parse_scenarios_toml_keys(repo_root: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    let toml_path = repo_root.join("benchmarks/scenarios/perf/scenarios.toml");
    let raw = std::fs::read_to_string(&toml_path)?;
    let table: toml::Table = toml::from_str(&raw)?;
    let mut out = BTreeMap::new();
    for (k, v) in table {
        if v.as_table().and_then(|t| t.get("label")).is_some() {
            out.insert(
                k,
                v.as_table().unwrap()["label"].as_str().unwrap().to_string(),
            );
        }
    }
    Ok(out)
}
