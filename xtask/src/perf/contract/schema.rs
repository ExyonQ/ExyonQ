use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CONTRACT_SCHEMA_VERSION: u32 = 1;
pub const CONTRACT_ID: &str = "performance-contract-v1";

pub fn default_baseline_path(repo_root: &Path) -> PathBuf {
    repo_root.join("benchmarks/baselines/performance-contract-v1.json")
}

/// Registry-backed proposed baselines (PS2-CA1). Does not replace `default_baseline_path` (linux-aarch64).
pub fn performance_contract_registry_path(repo_root: &Path) -> PathBuf {
    repo_root.join("benchmarks/baselines/performance-contract-registry.json")
}

pub fn proposed_baseline_path(repo_root: &Path, worker_mode: &str) -> PathBuf {
    repo_root.join(format!(
        "benchmarks/baselines/proposed/linux-x86_64-{worker_mode}-performance-contract-v1.json"
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fingerprint {
    pub schema_version: u32,
    pub scenario: String,
    pub platform: String,
    pub arch: String,
    pub kernel: Option<String>,
    pub cpu_count: u32,
    pub tool: ToolInfo,
    pub connections: u32,
    pub threads: u32,
    pub duration_warmup: String,
    pub duration_measure: String,
    pub load_mode: String,
    pub server_target: String,
    pub tls: bool,
    pub sendfile: bool,
    pub epoll_static: bool,
    pub path: String,
    pub payload_label: String,
    pub features: Vec<String>,
    pub config_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MetricsSnapshot {
    pub requests_per_second: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub error_count: u64,
    pub error_rate: f64,
    pub unexpected_5xx: u64,
    pub process_rss_bytes: Option<u64>,
}

impl MetricsSnapshot {
    pub fn is_complete(&self) -> bool {
        self.requests_per_second > 0.0
            && self.p50_ms >= 0.0
            && self.p95_ms >= 0.0
            && self.p99_ms >= 0.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunDocument {
    pub schema_version: u32,
    pub contract_id: String,
    pub commit: Option<String>,
    pub captured_at: String,
    pub valid: bool,
    pub invalid_reason: Option<String>,
    pub fingerprint: Fingerprint,
    pub metrics: MetricsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileSpec {
    pub repetitions: u32,
    pub duration_warmup: String,
    pub duration_measure: String,
    pub connections: u32,
    pub threads: u32,
    pub load_mode: String,
    pub tool: ToolInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineScenarioEntry {
    pub fingerprint: Fingerprint,
    pub metrics: MetricsSnapshot,
    pub repetitions: u32,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToleranceBands {
    pub rps_warn_pct: f64,
    pub rps_fail_pct: f64,
    pub p99_warn_pct: f64,
    pub p99_fail_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObservedCvPct {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p99: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScenarioGatePolicy {
    pub reference_quality: String,
    pub hard_gate: bool,
    #[serde(default)]
    pub failure_threshold_pending: bool,
    pub minimum_repetitions: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_cv_pct: Option<ObservedCvPct>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_threshold: Option<ToleranceBands>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_threshold: Option<ToleranceBands>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extreme_degradation_threshold: Option<ToleranceBands>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineDocument {
    pub schema_version: u32,
    pub contract_id: String,
    pub status: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub host_class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_profile: Option<ProfileSpec>,
    pub full_profile: ProfileSpec,
    pub scenarios: BTreeMap<String, BaselineScenarioEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_scenarios: Option<BTreeMap<String, BaselineScenarioEntry>>,
    /// `false` for `--accept-unvalidated-seed`; omitted when official accepted baseline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub official: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_class_netcup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_gates: Option<BTreeMap<String, ScenarioGatePolicy>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScenarioCompareLine {
    pub scenario: String,
    pub verdict: String,
    pub rps_delta_pct: Option<f64>,
    pub p99_delta_pct: Option<f64>,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompareReport {
    pub schema_version: u32,
    pub contract_id: String,
    pub overall_verdict: String,
    pub scenarios: Vec<ScenarioCompareLine>,
    /// `compare` (default) or `collect_only` when first-seed full collection skips competitive gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Present when `mode=collect_only` and samples validated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_status: Option<String>,
}

pub fn read_baseline(path: &Path) -> anyhow::Result<BaselineDocument> {
    let raw = std::fs::read_to_string(path)?;
    let doc: BaselineDocument = serde_json::from_str(&raw)?;
    if doc.schema_version != CONTRACT_SCHEMA_VERSION {
        anyhow::bail!(
            "baseline schema_version {} != supported {}",
            doc.schema_version,
            CONTRACT_SCHEMA_VERSION
        );
    }
    Ok(doc)
}

pub fn write_baseline(path: &Path, doc: &BaselineDocument) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(doc)?;
    std::fs::write(path, format!("{json}\n"))?;
    Ok(())
}

pub fn read_run(path: &Path) -> anyhow::Result<RunDocument> {
    let raw = std::fs::read_to_string(path)?;
    let doc: RunDocument = serde_json::from_str(&raw)?;
    if doc.schema_version != CONTRACT_SCHEMA_VERSION {
        anyhow::bail!(
            "run schema_version {} != supported {}",
            doc.schema_version,
            CONTRACT_SCHEMA_VERSION
        );
    }
    Ok(doc)
}

pub fn write_run(path: &Path, doc: &RunDocument) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(doc)?;
    std::fs::write(path, format!("{json}\n"))?;
    Ok(())
}
