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
//! Cap024: opt-in active upstream HTTP health-check configuration (IR).

use crate::ConfigError;
use serde::Deserialize;

pub const DEFAULT_HEALTH_INTERVAL_MS: u64 = 5_000;
pub const DEFAULT_HEALTH_TIMEOUT_MS: u64 = 1_000;
pub const DEFAULT_HEALTH_PATH: &str = "/health";
pub const DEFAULT_HEALTHY_THRESHOLD: u32 = 2;
pub const DEFAULT_UNHEALTHY_THRESHOLD: u32 = 3;

const MIN_INTERVAL_MS: u64 = 100;
const MAX_INTERVAL_MS: u64 = 3_600_000;
const MIN_TIMEOUT_MS: u64 = 1;
const MAX_TIMEOUT_MS: u64 = 60_000;
const MAX_THRESHOLD: u32 = 32;
const MAX_PATH_LEN: usize = 256;

/// Active HTTP health-check knobs for one upstream (Cap024).
///
/// Default: disabled — existing deployments unchanged until explicitly enabled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamHealthCheckConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default = "default_healthy_threshold")]
    pub healthy_threshold: u32,
    #[serde(default = "default_unhealthy_threshold")]
    pub unhealthy_threshold: u32,
}

impl Default for UpstreamHealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_ms: DEFAULT_HEALTH_INTERVAL_MS,
            timeout_ms: DEFAULT_HEALTH_TIMEOUT_MS,
            path: DEFAULT_HEALTH_PATH.to_string(),
            healthy_threshold: DEFAULT_HEALTHY_THRESHOLD,
            unhealthy_threshold: DEFAULT_UNHEALTHY_THRESHOLD,
        }
    }
}

fn default_interval_ms() -> u64 {
    DEFAULT_HEALTH_INTERVAL_MS
}
fn default_timeout_ms() -> u64 {
    DEFAULT_HEALTH_TIMEOUT_MS
}
fn default_path() -> String {
    DEFAULT_HEALTH_PATH.to_string()
}
fn default_healthy_threshold() -> u32 {
    DEFAULT_HEALTHY_THRESHOLD
}
fn default_unhealthy_threshold() -> u32 {
    DEFAULT_UNHEALTHY_THRESHOLD
}

impl UpstreamHealthCheckConfig {
    /// Validate when `enabled`; disabled configs accept defaults without probing.
    pub fn validate(&self, upstream: &str) -> Result<(), ConfigError> {
        if !self.enabled {
            return Ok(());
        }
        if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&self.interval_ms) {
            return Err(ConfigError::Parse(format!(
                "upstream `{upstream}`: health_check.interval_ms must be {MIN_INTERVAL_MS}..={MAX_INTERVAL_MS}"
            )));
        }
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(ConfigError::Parse(format!(
                "upstream `{upstream}`: health_check.timeout_ms must be {MIN_TIMEOUT_MS}..={MAX_TIMEOUT_MS}"
            )));
        }
        if self.timeout_ms > self.interval_ms {
            return Err(ConfigError::Parse(format!(
                "upstream `{upstream}`: health_check.timeout_ms must be <= interval_ms"
            )));
        }
        if !(1..=MAX_THRESHOLD).contains(&self.healthy_threshold) {
            return Err(ConfigError::Parse(format!(
                "upstream `{upstream}`: health_check.healthy_threshold must be 1..={MAX_THRESHOLD}"
            )));
        }
        if !(1..=MAX_THRESHOLD).contains(&self.unhealthy_threshold) {
            return Err(ConfigError::Parse(format!(
                "upstream `{upstream}`: health_check.unhealthy_threshold must be 1..={MAX_THRESHOLD}"
            )));
        }
        validate_health_path(upstream, &self.path)?;
        Ok(())
    }
}

fn validate_health_path(upstream: &str, path: &str) -> Result<(), ConfigError> {
    if path.is_empty() || path.len() > MAX_PATH_LEN {
        return Err(ConfigError::Parse(format!(
            "upstream `{upstream}`: health_check.path length must be 1..={MAX_PATH_LEN}"
        )));
    }
    if !path.starts_with('/') {
        return Err(ConfigError::Parse(format!(
            "upstream `{upstream}`: health_check.path must start with '/'"
        )));
    }
    if path.contains("://") || path.contains(' ') || path.contains('\n') || path.contains('\r') {
        return Err(ConfigError::Parse(format!(
            "upstream `{upstream}`: health_check.path must be a path-only HTTP absolute-path"
        )));
    }
    // Cap024 SSRF: path must not rewrite authority (no host/scheme smuggling).
    if path.contains('@') {
        return Err(ConfigError::Parse(format!(
            "upstream `{upstream}`: health_check.path must not contain '@'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_default_ok() {
        UpstreamHealthCheckConfig::default().validate("u").unwrap();
    }

    #[test]
    fn enabled_rejects_timeout_gt_interval() {
        let c = UpstreamHealthCheckConfig {
            enabled: true,
            interval_ms: 100,
            timeout_ms: 200,
            ..Default::default()
        };
        assert!(c.validate("u").is_err());
    }

    #[test]
    fn enabled_rejects_authority_in_path() {
        let c = UpstreamHealthCheckConfig {
            enabled: true,
            path: "http://evil/health".into(),
            ..Default::default()
        };
        assert!(c.validate("u").is_err());
    }
}
