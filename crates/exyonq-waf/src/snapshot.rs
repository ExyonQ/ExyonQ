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

use crate::detect::CompiledDetectors;
use crate::exclusion::CompiledExclusion;
use exyonq_waf_api::{
    HeaderView, InspectionLimits, WafAction, WafEvidence, WafMode, WafPhase, WafRequest, WafRuleId,
    WafViolation,
};

/// Immutable compiled WAF configuration (swap via `Arc` / `ArcSwap`).
#[derive(Debug, Clone)]
pub struct CompiledWafSnapshot {
    pub(crate) enabled: bool,
    pub(crate) mode: WafMode,
    pub(crate) limits: InspectionLimits,
    pub(crate) detectors: CompiledDetectors,
    pub(crate) rulesets: Vec<CompiledRuleset>,
    pub(crate) exclusions: Vec<CompiledExclusion>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledRuleset {
    #[allow(dead_code)] // retained for diagnostics / WAF3 ruleset scoping
    pub id: String,
    pub enabled: bool,
    pub rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledRule {
    pub id: String,
    pub enabled: bool,
    pub phases: Vec<WafPhase>,
    pub matcher: LiteralMatcher,
    pub action: WafAction,
}

#[derive(Debug, Clone)]
pub(crate) enum LiteralMatcher {
    Path(String),
    Query(String),
    Body(String),
    Header { name: String, needle: String },
}

impl CompiledWafSnapshot {
    pub fn mode(&self) -> WafMode {
        self.mode
    }

    pub fn limits(&self) -> InspectionLimits {
        self.limits
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn evaluate_phase(
        &self,
        phase: WafPhase,
        req: &WafRequest<'_>,
        body_chunk: Option<&[u8]>,
    ) -> Vec<(WafViolation, WafAction)> {
        let mut hits = self
            .detectors
            .evaluate(phase, req, body_chunk, &self.exclusions);

        for rs in &self.rulesets {
            if !rs.enabled {
                continue;
            }
            for rule in &rs.rules {
                if !rule.enabled || !rule.phases.contains(&phase) {
                    continue;
                }
                if !rule.matcher.matches(req, body_chunk) {
                    continue;
                }
                let rule_id = WafRuleId::new(rule.id.clone());
                if self.exclusions.iter().any(|ex| ex.matches(req, &rule_id)) {
                    continue;
                }
                hits.push((
                    WafViolation {
                        rule_id,
                        phase,
                        message: format!("rule {} matched", rule.id),
                        evidence: WafEvidence {
                            field: Some(rule.matcher.field_label()),
                            matched: None,
                            transforms: Vec::new(),
                        },
                    },
                    rule.action,
                ));
            }
        }
        hits
    }
}

impl LiteralMatcher {
    fn field_label(&self) -> String {
        match self {
            Self::Path(_) => "path".into(),
            Self::Query(_) => "query".into(),
            Self::Body(_) => "body".into(),
            Self::Header { name, .. } => format!("header:{name}"),
        }
    }

    fn matches(&self, req: &WafRequest<'_>, body_chunk: Option<&[u8]>) -> bool {
        match self {
            Self::Path(needle) => req.path.contains(needle),
            Self::Query(needle) => req
                .query
                .map(|q| q.contains(needle.as_str()))
                .unwrap_or(false),
            // Cap015 / WAF-001: never fail-open on non-UTF8 — match like detectors (lossy)
            // so a needle present in raw bytes still hits.
            Self::Body(needle) => body_chunk
                .map(|b| String::from_utf8_lossy(b).contains(needle.as_str()))
                .unwrap_or(false),
            Self::Header { name, needle } => {
                let Some(raw) = req.headers.get(name) else {
                    return false;
                };
                String::from_utf8_lossy(raw).contains(needle.as_str())
            }
        }
    }
}

/// Reduce rule hits to a single action (strongest wins).
pub(crate) fn strongest_action(actions: impl Iterator<Item = WafAction>) -> WafAction {
    let mut best = WafAction::Allow;
    for a in actions {
        if action_rank(a) > action_rank(best) {
            best = a;
        }
    }
    best
}

fn action_rank(a: WafAction) -> u8 {
    match a {
        WafAction::Allow => 0,
        WafAction::Log => 1,
        WafAction::RateLimit => 2,
        WafAction::Challenge => 3,
        WafAction::Block => 4,
    }
}

/// Test helper: map of headers.
#[derive(Debug, Clone, Default)]
pub struct MapHeaders {
    // (lowercase name, raw value)
    pub entries: Vec<(String, Vec<u8>)>,
}

impl MapHeaders {
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self {
            entries: pairs
                .iter()
                .map(|(n, v)| (n.to_ascii_lowercase(), v.as_bytes().to_vec()))
                .collect(),
        }
    }
}

impl HeaderView for MapHeaders {
    fn get(&self, name: &str) -> Option<&[u8]> {
        let want = name.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|(n, _)| *n == want)
            .map(|(_, v)| v.as_slice())
    }

    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) {
        for (n, v) in &self.entries {
            f(n.as_bytes(), v.as_slice());
        }
    }
}
