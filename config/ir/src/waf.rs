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
//! `[waf]` product IR — engine mode, rulesets, exclusions, and abuse seam.
//!
//! ## WAF_ABUSE_CARDINALITY = SINGLETON
//!
//! Abuse protection is one process-global quota/challenge policy keyed by the
//! host-resolved trusted client IP. Multiple `[waf.abuse]` tables or an array of
//! abuse policies would require undefined precedence (which quota wins for the
//! same IP on overlapping routes). A single `[waf.abuse]` table under `[waf]`
//! keeps enforcement deterministic.

use crate::ConfigError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Off unless the operator sets `enabled = true`. The data plane must stay safe
/// without a WAF; inspection is opt-in, not a silent default.
fn default_waf_enabled() -> bool {
    false
}

fn default_waf_mode() -> WafModeIr {
    WafModeIr::Monitor
}

fn default_waf_engine() -> WafEngineIr {
    WafEngineIr::Native
}

fn default_max_inspection_body_bytes() -> u64 {
    1_048_576
}

fn default_on_inspection_limit() -> WafOnInspectionLimitIr {
    WafOnInspectionLimitIr::LogAndAllow
}

fn default_fail_policy() -> WafFailPolicyIr {
    WafFailPolicyIr::ClosedForInvalidRules
}

fn default_abuse_rps() -> u32 {
    10
}

fn default_abuse_burst() -> u32 {
    20
}

/// Top-level `[waf]` configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WafConfig {
    #[serde(default = "default_waf_enabled")]
    pub enabled: bool,
    #[serde(default = "default_waf_mode")]
    pub mode: WafModeIr,
    #[serde(default = "default_waf_engine")]
    pub engine: WafEngineIr,
    #[serde(default = "default_max_inspection_body_bytes")]
    pub max_inspection_body_bytes: u64,
    #[serde(default = "default_on_inspection_limit")]
    pub on_inspection_limit: WafOnInspectionLimitIr,
    #[serde(default = "default_fail_policy")]
    pub fail_policy: WafFailPolicyIr,
    /// Singleton abuse policy — see module docs (`WAF_ABUSE_CARDINALITY`).
    #[serde(default)]
    pub abuse: WafAbuseConfig,
    #[serde(default, rename = "ruleset")]
    pub rulesets: Vec<WafRulesetConfig>,
    #[serde(default, rename = "exclusion")]
    pub exclusions: Vec<WafExclusionConfig>,
}

impl Default for WafConfig {
    fn default() -> Self {
        Self {
            enabled: default_waf_enabled(),
            mode: default_waf_mode(),
            engine: default_waf_engine(),
            max_inspection_body_bytes: default_max_inspection_body_bytes(),
            on_inspection_limit: default_on_inspection_limit(),
            fail_policy: default_fail_policy(),
            abuse: WafAbuseConfig::default(),
            rulesets: Vec::new(),
            exclusions: Vec::new(),
        }
    }
}

/// Runtime enforcement mode for the signature engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WafModeIr {
    Monitor,
    Block,
    Disabled,
}

/// Product WAF engine selector (v1: native only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WafEngineIr {
    Native,
}

/// Policy when the body inspection budget is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WafOnInspectionLimitIr {
    AllowUninspected,
    Block,
    LogAndAllow,
}

/// Compile-time failure policy (v1: fail closed only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WafFailPolicyIr {
    ClosedForInvalidRules,
}

/// Abuse gate mode (host seam — separate from signature `mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WafAbuseMode {
    #[default]
    Off,
    RateLimit,
    Challenge,
}

/// `[waf.abuse]` — global quota/challenge policy (singleton).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WafAbuseConfig {
    #[serde(default)]
    pub mode: WafAbuseMode,
    #[serde(default = "default_abuse_rps")]
    pub requests_per_second: u32,
    #[serde(default = "default_abuse_burst")]
    pub burst: u32,
}

impl Default for WafAbuseConfig {
    fn default() -> Self {
        Self {
            mode: WafAbuseMode::Off,
            requests_per_second: default_abuse_rps(),
            burst: default_abuse_burst(),
        }
    }
}

/// `[[waf.ruleset]]` — named ruleset (builtins + optional custom rules).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WafRulesetConfig {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub builtins: Option<WafBuiltinToggles>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<WafCustomRuleConfig>,
}

fn default_true() -> bool {
    true
}

/// Optional built-in detector toggles (only fields with runtime effect).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WafBuiltinToggles {
    #[serde(default)]
    pub sql_injection: Option<WafDetectorModeIr>,
    #[serde(default)]
    pub xss: Option<WafDetectorModeIr>,
    #[serde(default)]
    pub path_traversal: Option<WafDetectorModeIr>,
    #[serde(default)]
    pub command_injection: Option<WafDetectorModeIr>,
    #[serde(default)]
    pub header_anomaly: Option<WafDetectorModeIr>,
    #[serde(default)]
    pub bot_ua: Option<WafDetectorModeIr>,
    #[serde(default)]
    pub ip_filter: Option<WafIpFilterConfig>,
}

/// Built-in detector enablement + action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WafDetectorModeIr {
    Off,
    Log,
    Block,
}

/// IP/CIDR filter toggles for built-in ruleset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WafIpFilterConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub whitelist: Vec<String>,
    #[serde(default)]
    pub blacklist: Vec<String>,
}

/// Custom literal rule inside a ruleset (`[[waf.ruleset.rule]]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WafCustomRuleConfig {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub pattern: String,
    pub target: String,
    pub action: WafRuleActionIr,
    #[serde(default)]
    pub phases: Vec<WafPhaseIr>,
}

/// Custom rule action (signature rules — no challenge/rate_limit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WafRuleActionIr {
    Allow,
    Log,
    Block,
}

/// Inspection phase names in config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WafPhaseIr {
    Connection,
    RequestHeaders,
    RequestBodyChunk,
    RequestBodyComplete,
    ResponseHeaders,
    ResponseBodyChunk,
}

/// `[[waf.exclusion]]` — scoped rule suppression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WafExclusionConfig {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub path_prefix: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub rule_ids: Vec<String>,
}

pub(crate) fn validate_waf(waf: &WafConfig) -> Result<(), ConfigError> {
    if waf.max_inspection_body_bytes == 0 {
        return Err(ConfigError::Parse(
            "waf.max_inspection_body_bytes must be positive".into(),
        ));
    }

    if waf.engine != WafEngineIr::Native {
        return Err(ConfigError::Parse("waf.engine must be \"native\"".into()));
    }

    if waf.fail_policy != WafFailPolicyIr::ClosedForInvalidRules {
        return Err(ConfigError::Parse(
            "waf.fail_policy must be \"closed_for_invalid_rules\"".into(),
        ));
    }

    validate_waf_abuse(&waf.abuse)?;

    let mut ruleset_ids = HashSet::new();
    let mut rule_ids = HashSet::new();

    for ruleset in &waf.rulesets {
        let id = ruleset.id.trim();
        if id.is_empty() {
            return Err(ConfigError::Parse(
                "waf.ruleset id must not be empty".into(),
            ));
        }
        if !ruleset_ids.insert(id.to_string()) {
            return Err(ConfigError::Parse(format!(
                "duplicate waf.ruleset id `{id}`"
            )));
        }

        for rule in &ruleset.rules {
            validate_custom_rule(rule)?;
            let rid = rule.id.trim();
            if !rule_ids.insert(rid.to_string()) {
                return Err(ConfigError::Parse(format!(
                    "duplicate waf rule id `{rid}` across rulesets"
                )));
            }
        }
    }

    for exclusion in &waf.exclusions {
        validate_exclusion(exclusion)?;
    }

    Ok(())
}

fn validate_waf_abuse(abuse: &WafAbuseConfig) -> Result<(), ConfigError> {
    if abuse.mode == WafAbuseMode::Off {
        return Ok(());
    }
    if abuse.requests_per_second == 0 {
        return Err(ConfigError::Parse(
            "waf.abuse.requests_per_second must be >= 1 when mode is not off".into(),
        ));
    }
    if abuse.burst == 0 {
        return Err(ConfigError::Parse(
            "waf.abuse.burst must be >= 1 when mode is not off".into(),
        ));
    }
    Ok(())
}

fn validate_custom_rule(rule: &WafCustomRuleConfig) -> Result<(), ConfigError> {
    let id = rule.id.trim();
    if id.is_empty() {
        return Err(ConfigError::Parse(
            "waf.ruleset.rule id must not be empty".into(),
        ));
    }
    if rule.pattern.is_empty() {
        return Err(ConfigError::Parse(format!(
            "waf rule `{id}` pattern must not be empty"
        )));
    }
    if rule.phases.is_empty() {
        return Err(ConfigError::Parse(format!(
            "waf rule `{id}` must define at least one phase"
        )));
    }
    parse_match_target(&rule.target)
        .map_err(|msg| ConfigError::Parse(format!("waf rule `{id}` target invalid: {msg}")))?;
    Ok(())
}

fn validate_exclusion(exclusion: &WafExclusionConfig) -> Result<(), ConfigError> {
    let host_empty = exclusion.host.as_ref().is_none_or(|h| h.trim().is_empty());
    let path_empty = exclusion
        .path_prefix
        .as_ref()
        .is_none_or(|p| p.trim().is_empty());
    let method_empty = exclusion
        .method
        .as_ref()
        .is_none_or(|m| m.trim().is_empty());
    let rule_ids_empty =
        exclusion.rule_ids.is_empty() || exclusion.rule_ids.iter().all(|id| id.trim().is_empty());

    if rule_ids_empty {
        return Err(ConfigError::Parse(
            "waf.exclusion rule_ids must not be empty".into(),
        ));
    }

    if host_empty && path_empty && method_empty && rule_ids_empty {
        return Err(ConfigError::Parse(
            "waf.exclusion must specify host, path_prefix, method, or rule_ids".into(),
        ));
    }

    for id in &exclusion.rule_ids {
        if id.trim().is_empty() {
            return Err(ConfigError::Parse(
                "waf.exclusion rule_ids must not contain empty entries".into(),
            ));
        }
    }

    Ok(())
}

fn parse_match_target(raw: &str) -> Result<(), &'static str> {
    let raw = raw.trim();
    match raw {
        "path" | "query" | "body" => Ok(()),
        _ if raw.starts_with("header:") => {
            let name = raw.strip_prefix("header:").unwrap_or("").trim();
            if name.is_empty() {
                Err("header target requires non-empty name after header:")
            } else {
                Ok(())
            }
        }
        _ => Err("expected path, query, body, or header:<name>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fingerprint, AppConfig};

    const MINIMAL: &str = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"
[[upstream]]
name = "backend"
target = "http://127.0.0.1:9000"
"#;

    #[test]
    fn absent_waf_defaults() {
        let config: AppConfig = MINIMAL.parse().expect("parse");
        assert!(!config.waf.enabled);
        assert_eq!(config.waf.mode, WafModeIr::Monitor);
        assert_eq!(config.waf.engine, WafEngineIr::Native);
        assert_eq!(config.waf.max_inspection_body_bytes, 1_048_576);
        assert_eq!(
            config.waf.on_inspection_limit,
            WafOnInspectionLimitIr::LogAndAllow
        );
        assert_eq!(
            config.waf.fail_policy,
            WafFailPolicyIr::ClosedForInvalidRules
        );
        assert_eq!(config.waf.abuse.mode, WafAbuseMode::Off);
        assert_eq!(config.waf.abuse.requests_per_second, 10);
        assert_eq!(config.waf.abuse.burst, 20);
        assert!(config.waf.rulesets.is_empty());
        assert!(config.waf.exclusions.is_empty());
    }

    #[test]
    fn full_waf_block_parses() {
        let input = format!(
            r#"
{MINIMAL}
[waf]
enabled = true
mode = "block"
engine = "native"
max_inspection_body_bytes = 65536
on_inspection_limit = "block"
fail_policy = "closed_for_invalid_rules"

[waf.abuse]
mode = "rate_limit"
requests_per_second = 5
burst = 15

[[waf.ruleset]]
id = "exyonq-core"
enabled = true

[[waf.exclusion]]
host = "example.com"
path_prefix = "/upload"
rule_ids = ["EXY-XSS-1001"]
"#
        );
        let config: AppConfig = input.parse().expect("parse");
        assert_eq!(config.waf.mode, WafModeIr::Block);
        assert_eq!(config.waf.max_inspection_body_bytes, 65536);
        assert_eq!(
            config.waf.on_inspection_limit,
            WafOnInspectionLimitIr::Block
        );
        assert_eq!(config.waf.abuse.mode, WafAbuseMode::RateLimit);
        assert_eq!(config.waf.rulesets.len(), 1);
        assert_eq!(config.waf.rulesets[0].id, "exyonq-core");
        assert_eq!(config.waf.exclusions.len(), 1);
    }

    #[test]
    fn waf_abuse_invalid_mode_fails_parse() {
        let input = format!(
            r#"
{MINIMAL}
[waf.abuse]
mode = "not_a_mode"
"#
        );
        assert!(input.parse::<AppConfig>().is_err());
    }

    #[test]
    fn waf_abuse_zero_rps_when_enabled_rejected() {
        let input = format!(
            r#"
{MINIMAL}
[waf.abuse]
mode = "challenge"
requests_per_second = 0
burst = 20
"#
        );
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(
            err.to_string().contains("waf.abuse"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn duplicate_ruleset_id_rejected() {
        let input = format!(
            r#"
{MINIMAL}
[[waf.ruleset]]
id = "a"
[[waf.ruleset]]
id = "a"
"#
        );
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(err.to_string().contains("duplicate waf.ruleset"), "{err}");
    }

    #[test]
    fn duplicate_rule_id_across_rulesets_rejected() {
        let input = format!(
            r#"
{MINIMAL}
[[waf.ruleset]]
id = "a"
[[waf.ruleset.rule]]
id = "R1"
pattern = "x"
target = "path"
action = "block"
phases = ["request_headers"]

[[waf.ruleset]]
id = "b"
[[waf.ruleset.rule]]
id = "R1"
pattern = "y"
target = "query"
action = "log"
phases = ["request_headers"]
"#
        );
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(err.to_string().contains("duplicate waf rule id"), "{err}");
    }

    #[test]
    fn empty_exclusion_rule_ids_rejected() {
        let input = format!(
            r#"
{MINIMAL}
[[waf.exclusion]]
host = "example.com"
rule_ids = []
"#
        );
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(err.to_string().contains("rule_ids"), "{err}");
    }

    #[test]
    fn zero_max_inspection_rejected() {
        let input = format!(
            r#"
{MINIMAL}
[waf]
max_inspection_body_bytes = 0
"#
        );
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(
            err.to_string().contains("max_inspection_body_bytes"),
            "{err}"
        );
    }

    #[test]
    fn fingerprint_changes_when_abuse_mode_changes() {
        let config: AppConfig = MINIMAL.parse().unwrap();
        let base = fingerprint(&config);
        let mut changed = config.clone();
        changed.waf.abuse.mode = WafAbuseMode::RateLimit;
        assert_ne!(base, fingerprint(&changed));
    }

    #[test]
    fn fingerprint_changes_when_waf_mode_changes() {
        let config: AppConfig = MINIMAL.parse().unwrap();
        let base = fingerprint(&config);
        let mut changed = config.clone();
        changed.waf.mode = WafModeIr::Block;
        assert_ne!(base, fingerprint(&changed));
    }
}
