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
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_second: default_rps(),
            burst: default_burst(),
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
