use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Health {
    pub status: String,
    pub generation: u64,
    pub revision: u64,
    pub draining: bool,
    pub reload_in_progress: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Stats {
    pub generation: u64,
    pub revision: u64,
    pub uptime_s: u64,
    pub active_connections: u64,
    pub draining: bool,
    pub reload_in_progress: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Revision {
    pub revision: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilitySet {
    pub api_version: String,
    pub features: BTreeMap<String, bool>,
}

impl Default for CapabilitySet {
    fn default() -> Self {
        Self {
            api_version: "v1".to_owned(),
            features: BTreeMap::from([
                ("health".to_owned(), true),
                ("stats".to_owned(), true),
                ("config.read".to_owned(), true),
                ("vhosts.create".to_owned(), true),
                ("vhosts.delete".to_owned(), true),
            ]),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VhostUpstream {
    pub target: String,
    #[serde(default = "default_upstream_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_upstream_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VhostCreate {
    pub domain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<VhostUpstream>,
}

fn default_path() -> String {
    "/".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vhost {
    pub domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<VhostUpstream>,
    pub revision: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub revision: u64,
    pub generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub vhosts: Vec<Vhost>,
    pub capabilities: CapabilitySet,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}
