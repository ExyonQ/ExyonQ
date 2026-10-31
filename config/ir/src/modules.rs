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
//! Optional official module settings (Fase 2).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModulesConfig {
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub compression: CompressionConfig,
    #[serde(default)]
    pub ratelimit: RateLimitConfig,
    #[serde(default)]
    pub real_ip: RealIpConfig,
    /// NG-18 / ADR-064: `gunzip` response filter (default off).
    #[serde(default, skip_serializing_if = "GunzipConfig::is_default")]
    pub gunzip: GunzipConfig,
    /// NG-18 / ADR-064 Phase C: nginx `sub_filter` rules per route.
    #[serde(default, skip_serializing_if = "SubFilterConfig::is_default")]
    pub sub_filter: SubFilterConfig,
    /// NG-18 / ADR-064 Phase D: nginx `mirror` per route.
    #[serde(default, skip_serializing_if = "MirrorConfig::is_default")]
    pub mirror: MirrorConfig,
    /// NG-18 / ADR-064 Phase E: nginx `slice` per route.
    #[serde(default, skip_serializing_if = "SliceConfig::is_default")]
    pub slice: SliceConfig,
    /// 0.5: nginx `auth_basic` per route.
    #[serde(default, skip_serializing_if = "AuthConfig::is_default")]
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthRouteConfig {
    pub realm: String,
    /// htpasswd file path.
    pub user_file: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, AuthRouteConfig>,
}

impl AuthConfig {
    pub fn is_default(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        for (route, cfg) in &self.routes {
            if cfg.realm.trim().is_empty() {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.auth.routes.{route}: empty realm"
                )));
            }
            if cfg.user_file.trim().is_empty() {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.auth.routes.{route}: empty user_file"
                )));
            }
        }
        Ok(())
    }
}

/// Effective nginx `slice` policy for one route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceRouteConfig {
    /// Segment size in bytes (`slice 1m` → 1048576).
    pub size: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceConfig {
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, SliceRouteConfig>,
}

impl SliceConfig {
    pub fn is_default(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        for (route, cfg) in &self.routes {
            if !(1024..=1_073_741_824).contains(&cfg.size) {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.slice.routes.{route}.size must be 1024..=1073741824, got {}",
                    cfg.size
                )));
            }
        }
        Ok(())
    }
}

/// Effective nginx `mirror` policy for one route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MirrorRouteConfig {
    /// Internal destination paths (`mirror /path;`).
    #[serde(default)]
    pub destinations: Vec<String>,
    /// `mirror_request_body` (nginx 1.27 default: on).
    #[serde(default = "default_true")]
    pub request_body: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MirrorConfig {
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, MirrorRouteConfig>,
}

impl MirrorConfig {
    pub fn is_default(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        for (route, cfg) in &self.routes {
            if cfg.destinations.is_empty() {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.mirror.routes.{route}: no destinations"
                )));
            }
            for dest in &cfg.destinations {
                if !dest.starts_with('/') {
                    return Err(crate::ConfigError::Parse(format!(
                        "modules.mirror.routes.{route}: destination `{dest}` must be an internal path"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// One nginx `sub_filter <pattern> <replacement>` rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubFilterRule {
    pub pattern: String,
    pub replacement: String,
}

/// Effective `sub_filter` policy for one route (importer resolves
/// http/server/location inheritance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubFilterRouteConfig {
    #[serde(default)]
    pub rules: Vec<SubFilterRule>,
    /// nginx `sub_filter_once` (default on).
    #[serde(default = "default_true")]
    pub once: bool,
    /// nginx `sub_filter_types` (default `text/html`).
    #[serde(default = "default_sub_filter_types")]
    pub types: Vec<String>,
    /// nginx `sub_filter_last_modified` (default off: remove Last-Modified).
    #[serde(default = "default_false")]
    pub last_modified: bool,
}

fn default_true() -> bool {
    true
}

fn default_sub_filter_types() -> Vec<String> {
    vec!["text/html".to_string()]
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubFilterConfig {
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, SubFilterRouteConfig>,
}

impl SubFilterConfig {
    pub fn is_default(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        for (route, cfg) in &self.routes {
            for rule in &cfg.rules {
                if rule.pattern.is_empty() {
                    return Err(crate::ConfigError::Parse(format!(
                        "modules.sub_filter.routes.{route}: empty pattern"
                    )));
                }
            }
            if cfg.types.iter().any(|t| t.trim().is_empty()) {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.sub_filter.routes.{route}: empty sub_filter_types entry"
                )));
            }
        }
        Ok(())
    }
}

/// NG-11: trust-aware client-IP resolution (nginx `set_real_ip_from` +
/// `real_ip_header`). When the immediate peer is inside `trusted`, the real
/// client IP is recovered from `header` (right-to-left, skipping trusted
/// hops). Default: disabled — the socket peer is always the client IP and
/// client-supplied forwarding headers can never spoof it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealIpConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Trusted proxy CIDRs / IPs (e.g. `"10.0.0.0/8"`, `"192.0.2.10"`).
    #[serde(default)]
    pub trusted: Vec<String>,
    /// Header carrying the forwarded chain (nginx `real_ip_header`).
    #[serde(default = "default_real_ip_header")]
    pub header: String,
}

fn default_real_ip_header() -> String {
    "x-forwarded-for".to_string()
}

impl Default for RealIpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            trusted: Vec::new(),
            header: default_real_ip_header(),
        }
    }
}

impl RealIpConfig {
    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        if !self.enabled {
            return Ok(());
        }
        if self.trusted.is_empty() {
            return Err(crate::ConfigError::Parse(
                "modules.real_ip: enabled requires at least one trusted CIDR".into(),
            ));
        }
        for t in &self.trusted {
            let ok = t.parse::<ipnetwork::IpNetwork>().is_ok()
                || t.trim().parse::<std::net::IpAddr>().is_ok();
            if !ok {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.real_ip.trusted entry `{t}` is not a CIDR or IP"
                )));
            }
        }
        let name = self.header.trim();
        let token_ok = !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
        if !token_ok {
            return Err(crate::ConfigError::Parse(format!(
                "modules.real_ip.header `{}` is not a valid header name",
                self.header
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_metrics_path")]
    pub path: String,
    #[serde(default = "default_health_path")]
    pub health_path: String,
    /// Required when metrics is enabled and any `server.listen` is non-loopback
    /// (LA-CAP054-008). Optional on loopback-only binds (Cap054 E2E).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrape_bearer_token: Option<String>,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: default_metrics_path(),
            health_path: default_health_path(),
            scrape_bearer_token: None,
        }
    }
}

/// NG-18 / ADR-064: nginx `gunzip` response filter.
///
/// `buffer_bytes` bounds the decoder output per poll; it is the ExyonQ
/// equivalent of `gunzip_buffers number size` (total bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GunzipConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_gunzip_buffer_bytes")]
    pub buffer_bytes: usize,
}

fn default_gunzip_buffer_bytes() -> usize {
    32 * 1024
}

impl Default for GunzipConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            buffer_bytes: default_gunzip_buffer_bytes(),
        }
    }
}

impl GunzipConfig {
    pub fn is_default(&self) -> bool {
        !self.enabled && self.buffer_bytes == default_gunzip_buffer_bytes()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        if !(1024..=1_048_576).contains(&self.buffer_bytes) {
            return Err(crate::ConfigError::Parse(format!(
                "modules.gunzip.buffer_bytes must be 1024..=1048576, got {}",
                self.buffer_bytes
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompressionConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_min_bytes")]
    pub min_bytes: usize,
    /// flate2 level 1–9 (default 1 = fastest, bench P9).
    #[serde(default = "default_compression_level")]
    pub level: u32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_bytes: default_min_bytes(),
            level: default_compression_level(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_rps")]
    pub requests_per_second: u32,
    #[serde(default = "default_burst")]
    pub burst: u32,
    /// 0.5: nginx `limit_req_zone` equivalents.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub zones: std::collections::BTreeMap<String, RateLimitZoneConfig>,
    /// 0.5: route name → zone name (`limit_req zone=...`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub routes: std::collections::BTreeMap<String, String>,
}

/// 0.5: one `limit_req_zone` (key is `ip`, `host`, or `header:<name>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitZoneConfig {
    pub key: String,
    #[serde(default = "default_rps")]
    pub requests_per_second: u32,
    #[serde(default = "default_burst")]
    pub burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_second: default_rps(),
            burst: default_burst(),
            zones: std::collections::BTreeMap::new(),
            routes: std::collections::BTreeMap::new(),
        }
    }
}

fn default_false() -> bool {
    false
}

fn default_metrics_path() -> String {
    "/metrics".to_string()
}

fn default_health_path() -> String {
    // Cap054: core `handle_request` owns GET /health as liveness body "ok" (WS5).
    // Metrics JSON health must not collide with that probe path.
    "/exyonq-metrics-health".to_string()
}

fn default_min_bytes() -> usize {
    256
}

fn default_compression_level() -> u32 {
    1
}

fn default_rps() -> u32 {
    100
}

fn default_burst() -> u32 {
    200
}
