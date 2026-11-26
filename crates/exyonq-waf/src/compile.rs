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

use crate::crs::CrsRulesetInput;
use crate::detect::{compile_detectors, BuiltinDetectorsInput};
use crate::exclusion::CompiledExclusion;
use crate::normalize::NormalizeBudget;
use crate::snapshot::{CompiledChainMember, CompiledCrsRule, CrsMatcher};
use crate::snapshot::{CompiledRule, CompiledRuleset, CompiledWafSnapshot, LiteralMatcher};
use exyonq_waf_api::{InspectionLimits, OnInspectionLimit, WafAction, WafMode, WafPhase};
use thiserror::Error;

pub use crate::detect::{
    BuiltinDetectorsInput as BuiltinDetectors, DetectorToggle, IpFilterEntry, IpFilterInput,
};

/// Policy when rule compilation fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailPolicy {
    /// Reject the entire compile (default / required for product).
    ClosedForInvalidRules,
    /// Drop invalid rules with a diagnostic and keep compiling the rest.
    /// Used by diagnostics/tooling; product configuration stays fail-closed.
    BestEffort,
}

/// Host/path/method/rule exclusion (config shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusionInput {
    pub host: Option<String>,
    pub path_prefix: Option<String>,
    pub method: Option<String>,
    pub rule_ids: Vec<String>,
}

/// Where a literal (WAF2) matcher looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchTarget {
    Path,
    Query,
    Body,
    /// Header name compared case-insensitively.
    Header(String),
}

/// One rule before compile (literal substring matcher).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleInput {
    pub id: String,
    pub enabled: bool,
    pub phases: Vec<WafPhase>,
    pub target: MatchTarget,
    pub pattern: String,
    pub action: WafAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesetInput {
    pub id: String,
    pub enabled: bool,
    pub rules: Vec<RuleInput>,
}

/// Declarative WAF config compiled into [`CompiledWafSnapshot`].
#[derive(Debug, Clone)]
pub struct WafCompileInput {
    pub enabled: bool,
    pub mode: WafMode,
    pub limits: InspectionLimits,
    pub fail_policy: FailPolicy,
    pub normalize: NormalizeBudget,
    pub builtins: BuiltinDetectorsInput,
    pub rulesets: Vec<RulesetInput>,
    pub exclusions: Vec<ExclusionInput>,
}

impl Default for WafCompileInput {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: WafMode::Monitor,
            limits: InspectionLimits::default(),
            fail_policy: FailPolicy::ClosedForInvalidRules,
            normalize: NormalizeBudget::default(),
            builtins: BuiltinDetectorsInput::default(),
            rulesets: Vec::new(),
            exclusions: Vec::new(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompileError {
    #[error("invalid rule id (empty)")]
    EmptyRuleId,
    #[error("rule {0}: empty pattern")]
    EmptyPattern(String),
    #[error("rule {0}: no phases")]
    NoPhases(String),
    #[error("rule {0}: header target name is empty")]
    EmptyHeaderName(String),
    #[error("ruleset id is empty")]
    EmptyRulesetId,
    #[error("exclusion rule_id is empty")]
    EmptyExclusionRuleId,
    #[error("rule {0}: Challenge/RateLimit reserved for host abuse seam")]
    AbuseActionOnSignatureRule(String),
    #[error("detector compile failed: {0}")]
    Detector(String),
}

/// Compile input → immutable snapshot. Invalid rules fail closed.
pub fn compile_waf(input: WafCompileInput) -> Result<CompiledWafSnapshot, CompileError> {
    let fail_policy = input.fail_policy;
    let detectors =
        compile_detectors(&input.builtins, input.normalize).map_err(CompileError::Detector)?;

    let mut rulesets = Vec::with_capacity(input.rulesets.len());
    let mut stats = crate::snapshot::CompileStats::default();
    for rs in input.rulesets {
        if rs.id.trim().is_empty() {
            return Err(CompileError::EmptyRulesetId);
        }
        let mut rules = Vec::new();
        for rule in rs.rules {
            match compile_rule(rule) {
                Ok(rule) => {
                    stats.parsed += 1;
                    rules.push(rule);
                }
                Err(err) if fail_policy == FailPolicy::BestEffort => {
                    stats.discarded += 1;
                    stats.diagnostics.push(err.to_string());
                }
                Err(err) => return Err(err),
            }
        }
        rulesets.push(CompiledRuleset {
            id: rs.id,
            enabled: rs.enabled,
            rules,
            crs_rules: Vec::new(),
            inbound_threshold: 0,
            crs_detection_only: false,
            crs_disabled_ids: Vec::new(),
        });
    }

    let mut exclusions = Vec::with_capacity(input.exclusions.len());
    for ex in input.exclusions {
        for id in &ex.rule_ids {
            if id.trim().is_empty() {
                return Err(CompileError::EmptyExclusionRuleId);
            }
        }
        exclusions.push(CompiledExclusion {
            host: ex.host,
            path_prefix: ex.path_prefix,
            method: ex.method,
            rule_ids: ex.rule_ids,
        });
    }

    Ok(CompiledWafSnapshot {
        enabled: input.enabled,
        mode: input.mode,
        limits: input.limits,
        detectors,
        rulesets,
        exclusions,
        stats,
    })
}

/// Compile a CRS ruleset into the existing WAF snapshot.
pub fn compile_crs_ruleset(input: CrsRulesetInput) -> Result<CompiledRuleset, CompileError> {
    if input.id.trim().is_empty() {
        return Err(CompileError::EmptyRulesetId);
    }
    let markers = input.markers.clone();
    let mut crs_rules = Vec::with_capacity(input.rules.len());
    for rule in input.rules {
        if rule.id.trim().is_empty() {
            return Err(CompileError::EmptyRuleId);
        }
        if rule.phases.is_empty() {
            return Err(CompileError::NoPhases(rule.id));
        }
        let matcher = CrsMatcher {
            targets: rule.targets.clone(),
            op: rule.op.clone(),
            negated: rule.negated,
            transforms: rule.transforms.clone(),
        };
        let chain = rule
            .chain
            .iter()
            .map(|chained| CompiledChainMember {
                matcher: CrsMatcher {
                    targets: chained.targets.clone(),
                    op: chained.op.clone(),
                    negated: chained.negated,
                    transforms: chained.transforms.clone(),
                },
                capture: chained.capture,
                setvars: chained.setvars.clone(),
            })
            .collect();
        let skip_after = rule
            .skip_after
            .as_ref()
            .and_then(|label| markers.get(label).copied());
        crs_rules.push(CompiledCrsRule {
            id: rule.id,
            enabled: rule.enabled,
            phases: rule.phases,
            matcher,
            chain,
            action: rule.action,
            score: rule.score,
            skip: rule.skip,
            skip_after,
            ctl_detection_only: rule.ctl_detection_only,
            ctl_remove_ids: rule.ctl_remove_ids,
            setvars: rule.setvars,
            paranoia_level: rule.paranoia_level,
            capture: rule.capture,
        });
    }
    Ok(CompiledRuleset {
        id: input.id,
        enabled: input.enabled,
        rules: Vec::new(),
        crs_rules,
        inbound_threshold: input.inbound_threshold,
        crs_detection_only: input.detection_only,
        crs_disabled_ids: input.disabled_ids,
    })
}

/// Compile the native WAF plus CRS rulesets.
pub fn compile_waf_with_crs(
    input: WafCompileInput,
    crs: &[CrsRulesetInput],
) -> Result<CompiledWafSnapshot, CompileError> {
    let mut snapshot = compile_waf(input)?;
    for ruleset in crs {
        snapshot.stats.parsed += ruleset.rules.len();
        snapshot.stats.discarded += ruleset.discarded;
        snapshot
            .stats
            .diagnostics
            .extend(ruleset.diagnostics.iter().cloned());
        snapshot
            .rulesets
            .push(compile_crs_ruleset(ruleset.clone())?);
    }
    Ok(snapshot)
}

fn compile_rule(rule: RuleInput) -> Result<CompiledRule, CompileError> {
    let id = rule.id.trim();
    if id.is_empty() {
        return Err(CompileError::EmptyRuleId);
    }
    if matches!(rule.action, WafAction::Challenge | WafAction::RateLimit) {
        return Err(CompileError::AbuseActionOnSignatureRule(id.to_string()));
    }
    if rule.pattern.is_empty() {
        return Err(CompileError::EmptyPattern(id.to_string()));
    }
    if rule.phases.is_empty() {
        return Err(CompileError::NoPhases(id.to_string()));
    }
    let matcher = match rule.target {
        MatchTarget::Path => LiteralMatcher::Path(rule.pattern),
        MatchTarget::Query => LiteralMatcher::Query(rule.pattern),
        MatchTarget::Body => LiteralMatcher::Body(rule.pattern),
        MatchTarget::Header(name) => {
            if name.trim().is_empty() {
                return Err(CompileError::EmptyHeaderName(id.to_string()));
            }
            LiteralMatcher::Header {
                name,
                needle: rule.pattern,
            }
        }
    };
    Ok(CompiledRule {
        id: id.to_string(),
        enabled: rule.enabled,
        phases: rule.phases,
        matcher,
        action: rule.action,
    })
}

impl WafCompileInput {
    pub fn with_on_limit(mut self, on_limit: OnInspectionLimit) -> Self {
        self.limits.on_limit = on_limit;
        self
    }

    /// Disable all built-in detectors (literal rulesets only).
    pub fn builtins_off(mut self) -> Self {
        self.builtins = BuiltinDetectorsInput {
            sql_injection: crate::detect::DetectorToggle::off(),
            xss: crate::detect::DetectorToggle::off(),
            path_traversal: crate::detect::DetectorToggle::off(),
            command_injection: crate::detect::DetectorToggle::off(),
            header_anomaly: crate::detect::DetectorToggle::off(),
            bot_ua: crate::detect::DetectorToggle::off(),
            ip_filter: IpFilterInput::default(),
        };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ruleset(rules: Vec<RuleInput>) -> RulesetInput {
        RulesetInput {
            id: "test".into(),
            enabled: true,
            rules,
        }
    }

    fn rule(id: &str, pattern: &str) -> RuleInput {
        RuleInput {
            id: id.into(),
            enabled: true,
            phases: vec![WafPhase::RequestHeaders],
            target: MatchTarget::Path,
            pattern: pattern.into(),
            action: WafAction::Block,
        }
    }

    #[test]
    fn f02_fail_policy_is_effective_and_counted() {
        let input = || WafCompileInput {
            builtins: BuiltinDetectorsInput::default(),
            rulesets: vec![ruleset(vec![rule("1", "/ok"), rule("2", "")])],
            ..Default::default()
        };

        let closed = compile_waf(input()).expect_err("closed policy must reject");
        assert_eq!(closed, CompileError::EmptyPattern("2".into()));

        let mut best_effort = input();
        best_effort.fail_policy = FailPolicy::BestEffort;
        let snapshot = compile_waf(best_effort).expect("best effort keeps valid rules");
        let stats = snapshot.compile_stats();
        assert_eq!(stats.parsed, 1, "only the valid rule is loaded");
        assert_eq!(stats.discarded, 1, "the empty-pattern rule is counted");
        assert_eq!(stats.diagnostics.len(), 1);
        assert!(stats.diagnostics[0].contains("empty pattern"));
        assert_eq!(snapshot.rulesets[0].rules.len(), 1);
        assert_eq!(snapshot.rulesets[0].rules[0].id, "1");
    }

    #[test]
    fn f02_crs_parser_discards_with_diagnostic_and_propagates_stats() {
        let crs = crate::crs::parse_rule_file(
            "crs",
            "SecRule REQUEST_URI \"@contains attack\" \"id:990001,deny\"\nSecRule broken\n",
            0,
        );
        assert_eq!(crs.rules.len(), 1);
        assert_eq!(crs.discarded, 1);
        assert!(crs.diagnostics[0].contains("line 2"));

        let snapshot = compile_waf_with_crs(WafCompileInput::default(), &[crs])
            .expect("valid CRS rule compiles");
        assert_eq!(snapshot.compile_stats().parsed, 1);
        assert_eq!(snapshot.compile_stats().discarded, 1);
        assert_eq!(snapshot.compile_stats().diagnostics.len(), 1);
    }
}
