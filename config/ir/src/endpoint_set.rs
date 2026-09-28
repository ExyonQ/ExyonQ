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
//! Native EndpointSet foundation (K8S-P2A).
//!
//! Kubernetes-agnostic identity and endpoint models for Config IR.
//! Dense plan `BackendId` (u32 table index) lives in `exyonq-runtime-plan` and
//! must not be conflated with [`UserBackendId`].

use crate::ConfigError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

/// Provisional hard max endpoints per backend (DoS / config abuse).
pub const MAX_ENDPOINTS_PER_BACKEND_PROVISIONAL: usize = 256;
/// Provisional hard max backends in one config.
pub const MAX_BACKENDS_PROVISIONAL: usize = 4096;
/// Provisional max length for BackendId / EndpointId strings.
pub const MAX_ID_LENGTH_PROVISIONAL: usize = 128;

/// User-facing stable backend identity (IR). Not the dense RuntimePlan BackendId.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct UserBackendId(String);

impl<'de> Deserialize<'de> for UserBackendId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

impl UserBackendId {
    pub fn new(raw: impl Into<String>) -> Result<Self, ConfigError> {
        let s = raw.into();
        validate_id_token("backend_id", &s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for UserBackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable endpoint identity within a backend EndpointSet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct EndpointId(String);

impl<'de> Deserialize<'de> for EndpointId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

impl EndpointId {
    pub fn new(raw: impl Into<String>) -> Result<Self, ConfigError> {
        let s = raw.into();
        validate_id_token("endpoint_id", &s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EndpointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn validate_id_token(kind: &'static str, s: &str) -> Result<(), ConfigError> {
    if s.is_empty() {
        return Err(ConfigError::InvalidEndpointIdentity {
            kind,
            detail: "empty".into(),
        });
    }
    if s.len() > MAX_ID_LENGTH_PROVISIONAL {
        return Err(ConfigError::InvalidEndpointIdentity {
            kind,
            detail: format!(
                "length {} exceeds MAX_ID_LENGTH_PROVISIONAL={MAX_ID_LENGTH_PROVISIONAL}",
                s.len()
            ),
        });
    }
    // Conservative charset: no path separators, whitespace, or non-ASCII (DoS / future path misuse).
    if !s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(ConfigError::InvalidEndpointIdentity {
            kind,
            detail: "must match [A-Za-z0-9._-]".into(),
        });
    }
    Ok(())
}

/// Desired admin state in Config IR (no observed health).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdminEndpointState {
    #[default]
    Enabled,
    Disabled,
    DrainRequested,
}

/// Address form: IPv4, IPv6, or hostname (runtime resolution deferred to existing URI path).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointAddress {
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
    Hostname(String),
}

impl EndpointAddress {
    pub fn parse(raw: &str) -> Result<Self, ConfigError> {
        let s = raw.trim();
        if s.is_empty() {
            return Err(ConfigError::InvalidEndpointAddress {
                value: raw.to_string(),
                detail: "empty".into(),
            });
        }
        if let Ok(v4) = s.parse::<Ipv4Addr>() {
            return Ok(Self::Ipv4(v4));
        }
        // Strip brackets for IPv6 literals like [::1]
        let bare = s
            .strip_prefix('[')
            .and_then(|r| r.strip_suffix(']'))
            .unwrap_or(s);
        if let Ok(v6) = bare.parse::<Ipv6Addr>() {
            return Ok(Self::Ipv6(v6));
        }
        validate_hostname(s)?;
        Ok(Self::Hostname(s.to_ascii_lowercase()))
    }

    pub fn as_host_str(&self) -> String {
        match self {
            Self::Ipv4(a) => a.to_string(),
            Self::Ipv6(a) => format!("[{a}]"),
            Self::Hostname(h) => h.clone(),
        }
    }
}

fn validate_hostname(s: &str) -> Result<(), ConfigError> {
    if s.len() > 253 {
        return Err(ConfigError::InvalidEndpointAddress {
            value: s.to_string(),
            detail: "hostname too long".into(),
        });
    }
    if s.starts_with('.') || s.ends_with('.') || s.contains("..") {
        return Err(ConfigError::InvalidEndpointAddress {
            value: s.to_string(),
            detail: "invalid hostname form".into(),
        });
    }
    for label in s.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(ConfigError::InvalidEndpointAddress {
                value: s.to_string(),
                detail: "invalid DNS label".into(),
            });
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(ConfigError::InvalidEndpointAddress {
                value: s.to_string(),
                detail: "hostname has illegal characters".into(),
            });
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(ConfigError::InvalidEndpointAddress {
                value: s.to_string(),
                detail: "DNS label cannot start/end with '-'".into(),
            });
        }
    }
    Ok(())
}

/// Selection policy (separate from failover).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EndpointSelectionPolicy {
    #[default]
    WeightedRoundRobin,
}

/// Failover policy (separate from selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EndpointFailoverPolicy {
    #[default]
    PriorityBands,
    None,
}

/// Single desired endpoint in an EndpointSet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Endpoint {
    pub endpoint_id: EndpointId,
    pub address: EndpointAddress,
    pub port: u16,
    pub weight: u32,
    pub priority: u32,
    pub admin_state: AdminEndpointState,
}

impl Endpoint {
    pub const DEFAULT_WEIGHT: u32 = 1;
    pub const DEFAULT_PRIORITY: u32 = 0;

    /// Build http:// URI for single-endpoint legacy/execution (hostname preserved; no DNS at compile).
    pub fn to_http_uri_string(&self) -> String {
        format!("http://{}:{}", self.address.as_host_str(), self.port)
    }
}

/// Explicit endpoint set for one logical backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct EndpointSet {
    endpoints: Vec<Endpoint>,
}

impl EndpointSet {
    pub fn empty() -> Self {
        Self {
            endpoints: Vec::new(),
        }
    }

    pub fn from_endpoints(mut endpoints: Vec<Endpoint>) -> Result<Self, ConfigError> {
        if endpoints.len() > MAX_ENDPOINTS_PER_BACKEND_PROVISIONAL {
            return Err(ConfigError::EndpointSetLimitExceeded {
                limit: MAX_ENDPOINTS_PER_BACKEND_PROVISIONAL,
                found: endpoints.len(),
            });
        }
        // Deterministic order: by endpoint_id, then address/port.
        endpoints.sort_by(|a, b| {
            a.endpoint_id
                .cmp(&b.endpoint_id)
                .then_with(|| a.address.as_host_str().cmp(&b.address.as_host_str()))
                .then_with(|| a.port.cmp(&b.port))
        });
        let mut seen = BTreeSet::new();
        for ep in &endpoints {
            if !seen.insert(ep.endpoint_id.as_str().to_string()) {
                return Err(ConfigError::DuplicateEndpointId {
                    endpoint_id: ep.endpoint_id.as_str().to_string(),
                });
            }
            if ep.port == 0 {
                return Err(ConfigError::InvalidEndpointPort { port: 0 });
            }
            // weight may be 0 (never selected); no upper hard max beyond u32.
        }
        Ok(Self { endpoints })
    }

    pub fn endpoints(&self) -> &[Endpoint] {
        &self.endpoints
    }

    pub fn len(&self) -> usize {
        self.endpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }

    /// Sole executable URI when exactly one endpoint is present.
    pub fn sole_http_uri(&self) -> Option<String> {
        if self.endpoints.len() == 1 {
            Some(self.endpoints[0].to_http_uri_string())
        } else {
            None
        }
    }

    /// Desired-config fingerprint material (no runtime/observed fields).
    pub fn desired_fingerprint_parts(&self) -> Vec<String> {
        self.endpoints
            .iter()
            .map(|ep| {
                format!(
                    "{}|{}|{}|{}|{}|{:?}",
                    ep.endpoint_id.as_str(),
                    ep.address.as_host_str(),
                    ep.port,
                    ep.weight,
                    ep.priority,
                    ep.admin_state
                )
            })
            .collect()
    }
}

/// Raw TOML/JSON endpoint row before normalize.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawEndpointConfig {
    #[serde(default)]
    pub id: Option<String>,
    pub address: String,
    pub port: u16,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub admin_state: AdminEndpointState,
}

fn default_weight() -> u32 {
    Endpoint::DEFAULT_WEIGHT
}

/// Deterministic endpoint id from desired address/port/weight/priority (no runtime).
pub fn deterministic_endpoint_id(
    address: &EndpointAddress,
    port: u16,
    weight: u32,
    priority: u32,
) -> EndpointId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    address.hash(&mut hasher);
    port.hash(&mut hasher);
    weight.hash(&mut hasher);
    priority.hash(&mut hasher);
    let digest = hasher.finish();
    // Fixed-width hex — always valid length/charset for EndpointId.
    EndpointId(format!("ep-{digest:016x}"))
}

pub fn endpoint_from_raw(raw: RawEndpointConfig) -> Result<Endpoint, ConfigError> {
    let address = EndpointAddress::parse(&raw.address)?;
    let endpoint_id = match raw.id {
        Some(id) => EndpointId::new(id)?,
        None => deterministic_endpoint_id(&address, raw.port, raw.weight, raw.priority),
    };
    Ok(Endpoint {
        endpoint_id,
        address,
        port: raw.port,
        weight: raw.weight,
        priority: raw.priority,
        admin_state: raw.admin_state,
    })
}

/// Parse legacy `http://host:port` (or host-only → port 80) into one Endpoint.
pub fn endpoint_from_http_target(target: &str) -> Result<Endpoint, ConfigError> {
    let uri: hyper::Uri = target
        .parse()
        .map_err(|_| ConfigError::InvalidUpstreamTarget {
            value: target.to_string(),
        })?;
    if uri.scheme_str() != Some("http") {
        return Err(ConfigError::InvalidUpstreamTarget {
            value: target.to_string(),
        });
    }
    let host = uri
        .host()
        .ok_or_else(|| ConfigError::InvalidUpstreamTarget {
            value: target.to_string(),
        })?;
    let port = uri.port_u16().unwrap_or(80);
    let address = EndpointAddress::parse(host)?;
    let weight = Endpoint::DEFAULT_WEIGHT;
    let priority = Endpoint::DEFAULT_PRIORITY;
    let endpoint_id = deterministic_endpoint_id(&address, port, weight, priority);
    Ok(Endpoint {
        endpoint_id,
        address,
        port,
        weight,
        priority,
        admin_state: AdminEndpointState::Enabled,
    })
}

/// Normalize discovery host:port (or host) strings into http targets then Endpoint.
pub fn endpoint_from_discovery_hostport(endpoint: &str) -> Result<Endpoint, ConfigError> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::InvalidEndpointAddress {
            value: endpoint.to_string(),
            detail: "empty discovery endpoint".into(),
        });
    }
    if trimmed.contains("://") {
        return endpoint_from_http_target(trimmed);
    }
    endpoint_from_http_target(&format!("http://{trimmed}"))
}

impl FromStr for EndpointAddress {
    type Err = ConfigError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_id_rejects_empty_and_long() {
        assert!(UserBackendId::new("").is_err());
        assert!(UserBackendId::new("a".repeat(MAX_ID_LENGTH_PROVISIONAL + 1)).is_err());
        assert!(UserBackendId::new("api").is_ok());
    }

    #[test]
    fn endpoint_id_deterministic_stable() {
        let a = EndpointAddress::parse("127.0.0.1").unwrap();
        let id1 = deterministic_endpoint_id(&a, 8080, 1, 0);
        let id2 = deterministic_endpoint_id(&a, 8080, 1, 0);
        assert_eq!(id1, id2);
        let id3 = deterministic_endpoint_id(&a, 8081, 1, 0);
        assert_ne!(id1, id3);
    }

    #[test]
    fn address_ipv4_ipv6_hostname() {
        assert!(matches!(
            EndpointAddress::parse("10.0.0.1").unwrap(),
            EndpointAddress::Ipv4(_)
        ));
        assert!(matches!(
            EndpointAddress::parse("::1").unwrap(),
            EndpointAddress::Ipv6(_)
        ));
        assert!(matches!(
            EndpointAddress::parse("[2001:db8::1]").unwrap(),
            EndpointAddress::Ipv6(_)
        ));
        assert!(matches!(
            EndpointAddress::parse("api.example.com").unwrap(),
            EndpointAddress::Hostname(_)
        ));
        assert!(EndpointAddress::parse("bad host").is_err());
    }

    #[test]
    fn endpoint_set_orders_and_rejects_dup_ids() {
        let a = endpoint_from_http_target("http://10.0.0.2:80").unwrap();
        let mut b = endpoint_from_http_target("http://10.0.0.1:80").unwrap();
        b.endpoint_id = a.endpoint_id.clone();
        assert!(EndpointSet::from_endpoints(vec![a, b]).is_err());
    }

    #[test]
    fn endpoint_id_collision_rejects_duplicate() {
        // ENDPOINT_ID_COLLISION_BEHAVIOR = REJECT_DUPLICATE_OR_COLLISION
        let a = EndpointAddress::parse("10.0.0.1").unwrap();
        let id = deterministic_endpoint_id(&a, 80, 1, 0);
        let ep1 = Endpoint {
            endpoint_id: id.clone(),
            address: a.clone(),
            port: 80,
            weight: 1,
            priority: 0,
            admin_state: AdminEndpointState::Enabled,
        };
        let ep2 = Endpoint {
            endpoint_id: id,
            address: EndpointAddress::parse("10.0.0.2").unwrap(),
            port: 81,
            weight: 2,
            priority: 1,
            admin_state: AdminEndpointState::Disabled,
        };
        let err = EndpointSet::from_endpoints(vec![ep1, ep2]).unwrap_err();
        assert!(matches!(err, ConfigError::DuplicateEndpointId { .. }));
    }

    #[test]
    fn deterministic_id_fields_exclude_runtime_volatile() {
        // ENDPOINT_ID_DERIVATION_FIELDS = address, port, weight, priority
        // ENDPOINT_ID_RUNTIME_VOLATILE_FIELDS = NONE
        let a = EndpointAddress::parse("api.example.com").unwrap();
        let id = deterministic_endpoint_id(&a, 443, 3, 7);
        assert!(id.as_str().starts_with("ep-"));
        assert_eq!(id.as_str().len(), 3 + 16);
        // Same desired fields → same id (hostname preserved, not resolved IP).
        assert_eq!(id, deterministic_endpoint_id(&a, 443, 3, 7));
    }

    #[test]
    fn id_charset_rejects_path_like() {
        assert!(UserBackendId::new("../etc/passwd").is_err());
        assert!(EndpointId::new("a/b").is_err());
        assert!(UserBackendId::new("ok_backend.1").is_ok());
    }

    #[test]
    fn empty_set_ok() {
        let set = EndpointSet::from_endpoints(vec![]).unwrap();
        assert!(set.is_empty());
        assert!(set.sole_http_uri().is_none());
    }

    #[test]
    fn legacy_target_single() {
        let ep = endpoint_from_http_target("http://127.0.0.1:9000").unwrap();
        let set = EndpointSet::from_endpoints(vec![ep]).unwrap();
        assert_eq!(
            set.sole_http_uri().as_deref(),
            Some("http://127.0.0.1:9000")
        );
    }

    #[test]
    fn fingerprint_parts_ignore_nothing_runtime() {
        let ep = endpoint_from_http_target("http://127.0.0.1:9000").unwrap();
        let set = EndpointSet::from_endpoints(vec![ep]).unwrap();
        let parts = set.desired_fingerprint_parts();
        assert_eq!(parts.len(), 1);
        assert!(!parts[0].contains("latency"));
    }

    #[test]
    fn policies_default_separated() {
        assert_eq!(
            EndpointSelectionPolicy::default(),
            EndpointSelectionPolicy::WeightedRoundRobin
        );
        assert_eq!(
            EndpointFailoverPolicy::default(),
            EndpointFailoverPolicy::PriorityBands
        );
    }

    #[test]
    fn port_zero_rejected() {
        let mut ep = endpoint_from_http_target("http://127.0.0.1:9000").unwrap();
        ep.port = 0;
        assert!(EndpointSet::from_endpoints(vec![ep]).is_err());
    }
}
