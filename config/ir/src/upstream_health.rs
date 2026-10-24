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
    /// NG-10: expected probe response statuses; empty = any 2xx.
    #[serde(default)]
    pub expect_status: Vec<u16>,
    /// NG-10: substring that must appear within the first 64 KiB of the body.
    #[serde(default)]
    pub expect_body: Option<String>,
    /// NG-10: `"Name"` = header must be present; `"Name: value"` = value match.
    #[serde(default)]
    pub expect_header: Option<String>,
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
            expect_status: Vec::new(),
            expect_body: None,
            expect_header: None,
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
        for code in &self.expect_status {
            // 1xx is never surfaced as a final response status — such an
            // expectation can never match and would pin the peer unhealthy.
            if !(200..=599).contains(code) {
                return Err(ConfigError::Parse(format!(
                    "upstream `{upstream}`: health_check.expect_status code {code} must be 200..=599"
                )));
            }
        }
        if let Some(body) = &self.expect_body {
            if body.is_empty() || body.len() > MAX_EXPECT_BODY_LEN {
                return Err(ConfigError::Parse(format!(
                    "upstream `{upstream}`: health_check.expect_body length must be 1..={MAX_EXPECT_BODY_LEN}"
                )));
            }
            // Control chars would render via `{:?}` as `\u{XX}` — invalid TOML.
            if body.chars().any(|c| c < '\x20' || c == '\x7f') {
                return Err(ConfigError::Parse(format!(
                    "upstream `{upstream}`: health_check.expect_body must not contain control characters"
                )));
            }
        }
        if let Some(h) = &self.expect_header {
            validate_expect_header(upstream, h)?;
        }
        Ok(())
    }
}

const MAX_EXPECT_BODY_LEN: usize = 4096;

/// NG-10: `"Name"` (presence) or `"Name: value"` (exact value match after
/// trimming) — header name must be a valid HTTP token.
fn validate_expect_header(upstream: &str, spec: &str) -> Result<(), ConfigError> {
    let (name, value) = match spec.split_once(':') {
        Some((n, v)) => (n.trim(), Some(v.trim())),
        None => (spec.trim(), None),
    };
    let token_ok = !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
    if !token_ok {
        return Err(ConfigError::Parse(format!(
            "upstream `{upstream}`: health_check.expect_header `{spec}` has an invalid header name"
        )));
    }
    if let Some(v) = value {
        // to_str() only yields visible ASCII; a non-ASCII value can never
        // match, and control chars break the TOML renderer's `{:?}` escaping.
        if v.len() > 1024 || !v.is_ascii() || v.chars().any(|c| c < '\x20' || c == '\x7f') {
            return Err(ConfigError::Parse(format!(
                "upstream `{upstream}`: health_check.expect_header value invalid"
            )));
        }
    }
    Ok(())
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

    #[test]
    fn ng10_expect_fields_parse_and_validate() {
        let c: UpstreamHealthCheckConfig = toml::from_str(
            r#"enabled = true
               expect_status = [200, 204]
               expect_body = "READY"
               expect_header = "X-Health: pass""#,
        )
        .unwrap();
        assert_eq!(c.expect_status, vec![200, 204]);
        c.validate("u").unwrap();

        for bad in [
            UpstreamHealthCheckConfig {
                enabled: true,
                expect_status: vec![99],
                ..Default::default()
            },
            // 1xx never surfaces as a final status — unreachable expectation.
            UpstreamHealthCheckConfig {
                enabled: true,
                expect_status: vec![101],
                ..Default::default()
            },
            UpstreamHealthCheckConfig {
                enabled: true,
                expect_header: Some("Bad Header".into()),
                ..Default::default()
            },
            // Control chars break the TOML renderer's `{:?}` escaping.
            UpstreamHealthCheckConfig {
                enabled: true,
                expect_body: Some("a\u{7f}".into()),
                ..Default::default()
            },
            UpstreamHealthCheckConfig {
                enabled: true,
                expect_header: Some("X-H: v\u{e9}e".into()),
                ..Default::default()
            },
            UpstreamHealthCheckConfig {
                enabled: true,
                expect_body: Some("x".repeat(5000)),
                ..Default::default()
            },
        ] {
            assert!(bad.validate("u").is_err());
        }
    }
}
