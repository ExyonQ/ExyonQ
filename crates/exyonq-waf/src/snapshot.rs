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

use crate::crs::{
    apply_transforms, url_decode, CrsOp, CrsSetVar, CrsSetVarOp, CrsTarget, CrsTransform,
};
use crate::detect::CompiledDetectors;
use crate::exclusion::CompiledExclusion;
use exyonq_waf_api::{
    HeaderView, InspectionLimits, WafAction, WafEvidence, WafMode, WafPhase, WafRequest,
    WafResponse, WafRuleId, WafViolation,
};
use std::collections::HashMap;

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
    pub crs_rules: Vec<CompiledCrsRule>,
    pub inbound_threshold: u32,
    pub crs_detection_only: bool,
    pub crs_disabled_ids: Vec<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledCrsRule {
    pub id: String,
    pub enabled: bool,
    pub phases: Vec<WafPhase>,
    pub matcher: CrsMatcher,
    pub chain: Vec<CrsMatcher>,
    pub action: WafAction,
    pub score: u32,
    pub skip: u32,
    pub skip_after: Option<usize>,
    pub ctl_detection_only: bool,
    pub ctl_remove_ids: Vec<u32>,
    pub setvars: Vec<CrsSetVar>,
    pub paranoia_level: u8,
    pub capture: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CrsMatcher {
    pub targets: Vec<CrsTarget>,
    pub op: CrsOp,
    pub negated: bool,
    pub transforms: Vec<CrsTransform>,
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

    pub(crate) fn inspects_response_body(&self) -> bool {
        self.rulesets.iter().any(|rs| {
            rs.enabled
                && rs
                    .crs_rules
                    .iter()
                    .any(|rule| rule.enabled && rule.phases.contains(&WafPhase::ResponseBodyChunk))
        })
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
            let views = BodyViews::new(req.headers, body_chunk);
            let mut crs_score = 0u32;
            let mut skip_until = 0usize;
            let mut detection_only = rs.crs_detection_only;
            let mut removed_ids = rs.crs_disabled_ids.clone();
            let mut tx: HashMap<String, String> = HashMap::new();
            for (idx, rule) in rs.crs_rules.iter().enumerate() {
                let executing_pl = tx
                    .get("executing_paranoia_level")
                    .and_then(|value| value.parse::<u8>().ok())
                    .unwrap_or(1);
                if rule.paranoia_level > executing_pl {
                    continue;
                }
                if idx < skip_until {
                    continue;
                }
                if !rule.enabled
                    || !rule.phases.contains(&phase)
                    || removed_ids.contains(&rule.id.parse().unwrap_or(0))
                {
                    continue;
                }
                let Some(matched) = rule.matcher.matches_collect(req, &views, &tx) else {
                    continue;
                };
                let mut chain_tx = tx.clone();
                record_matched_vars(&mut chain_tx, &matched);
                if let Some((_, value)) = matched.last() {
                    if rule.capture {
                        apply_captures(&mut chain_tx, &rule.matcher.op, value);
                    }
                }
                if !rule.chain.iter().all(|m| m.matches(req, &views, &chain_tx)) {
                    continue;
                }
                tx = chain_tx;
                let rule_id = WafRuleId::new(rule.id.clone());
                if self.exclusions.iter().any(|ex| ex.matches(req, &rule_id)) {
                    continue;
                }
                crs_score = crs_score.saturating_add(rule.score);
                hits.push((
                    WafViolation {
                        rule_id,
                        phase,
                        message: format!("crs rule {} matched", rule.id),
                        evidence: WafEvidence {
                            field: Some(rule.matcher.field_label()),
                            matched: None,
                            transforms: Vec::new(),
                        },
                    },
                    if detection_only {
                        WafAction::Log
                    } else {
                        rule.action
                    },
                ));
                if rule.skip > 0 {
                    skip_until = skip_until.max(idx + 1 + rule.skip as usize);
                }
                if let Some(after) = rule.skip_after {
                    skip_until = skip_until.max(after);
                }
                if rule.ctl_detection_only {
                    detection_only = true;
                }
                removed_ids.extend(rule.ctl_remove_ids.iter().copied());
                apply_setvars(&mut tx, &rule.setvars);
                if let Some(score) = tx
                    .get("inbound_anomaly_score")
                    .and_then(|value| value.parse::<u32>().ok())
                {
                    crs_score = score;
                }
            }
            if rs.inbound_threshold > 0 && crs_score >= rs.inbound_threshold {
                hits.push((
                    WafViolation {
                        rule_id: WafRuleId::new(format!("crs-anomaly-{}", rs.id)),
                        phase,
                        message: format!(
                            "crs anomaly score {crs_score} >= {}",
                            rs.inbound_threshold
                        ),
                        evidence: WafEvidence {
                            field: None,
                            matched: None,
                            transforms: Vec::new(),
                        },
                    },
                    if detection_only {
                        WafAction::Log
                    } else {
                        WafAction::Block
                    },
                ));
            }
        }
        hits
    }

    pub(crate) fn evaluate_response_phase(
        &self,
        phase: WafPhase,
        req: &WafRequest<'_>,
        resp: &WafResponse<'_>,
    ) -> Vec<(WafViolation, WafAction)> {
        let mut hits = Vec::new();
        for rs in &self.rulesets {
            if !rs.enabled {
                continue;
            }
            let mut crs_score = 0u32;
            let mut skip_until = 0usize;
            let mut detection_only = rs.crs_detection_only;
            let mut removed_ids = rs.crs_disabled_ids.clone();
            let mut tx: HashMap<String, String> = HashMap::new();
            for (idx, rule) in rs.crs_rules.iter().enumerate() {
                let executing_pl = tx
                    .get("executing_paranoia_level")
                    .and_then(|value| value.parse::<u8>().ok())
                    .unwrap_or(1);
                if rule.paranoia_level > executing_pl {
                    continue;
                }
                if idx < skip_until {
                    continue;
                }
                if !rule.enabled
                    || !rule.phases.contains(&phase)
                    || removed_ids.contains(&rule.id.parse().unwrap_or(0))
                {
                    continue;
                }
                if !rule.matcher.matches_response(resp, &tx) {
                    continue;
                }
                let mut chain_tx = tx.clone();
                if let Some(value) = rule.matcher.first_response_value(resp, &tx) {
                    chain_tx.insert("matched_var".to_string(), value.clone());
                    if rule.capture {
                        apply_captures(&mut chain_tx, &rule.matcher.op, &value);
                    }
                }
                if !rule
                    .chain
                    .iter()
                    .all(|m| m.matches_response(resp, &chain_tx))
                {
                    continue;
                }
                tx = chain_tx;
                let rule_id = WafRuleId::new(rule.id.clone());
                if self.exclusions.iter().any(|ex| ex.matches(req, &rule_id)) {
                    continue;
                }
                crs_score = crs_score.saturating_add(rule.score);
                hits.push((
                    WafViolation {
                        rule_id,
                        phase,
                        message: format!("crs rule {} matched", rule.id),
                        evidence: WafEvidence {
                            field: Some(rule.matcher.field_label()),
                            matched: None,
                            transforms: Vec::new(),
                        },
                    },
                    if detection_only {
                        WafAction::Log
                    } else {
                        rule.action
                    },
                ));
                if rule.skip > 0 {
                    skip_until = skip_until.max(idx + 1 + rule.skip as usize);
                }
                if let Some(after) = rule.skip_after {
                    skip_until = skip_until.max(after);
                }
                if rule.ctl_detection_only {
                    detection_only = true;
                }
                removed_ids.extend(rule.ctl_remove_ids.iter().copied());
                apply_setvars(&mut tx, &rule.setvars);
                if let Some(score) = tx
                    .get("inbound_anomaly_score")
                    .and_then(|value| value.parse::<u32>().ok())
                {
                    crs_score = score;
                }
            }
            if rs.inbound_threshold > 0 && crs_score >= rs.inbound_threshold {
                hits.push((
                    WafViolation {
                        rule_id: WafRuleId::new(format!("crs-anomaly-{}", rs.id)),
                        phase,
                        message: format!(
                            "crs anomaly score {crs_score} >= {}",
                            rs.inbound_threshold
                        ),
                        evidence: WafEvidence {
                            field: None,
                            matched: None,
                            transforms: Vec::new(),
                        },
                    },
                    if detection_only {
                        WafAction::Log
                    } else {
                        WafAction::Block
                    },
                ));
            }
        }
        hits
    }
}

fn expand_tx(value: &str, tx: &HashMap<String, String>) -> String {
    let mut out = value.to_string();
    while let Some(start) = out.find("%{") {
        let Some(end) = out[start + 2..].find('}') else {
            break;
        };
        let key = out[start + 2..start + 2 + end].to_ascii_lowercase();
        // ModSecurity writes collections as `%{ARGS:name}` or `%{ARGS.name}`.
        let key = match key.split_once(':') {
            Some((collection, name)) => format!("{collection}.{name}"),
            None => key,
        };
        // `tx.*` is stored unprefixed; `args.*` keeps its collection prefix.
        let name = key.strip_prefix("tx.").unwrap_or(&key);
        let replacement = tx.get(name).cloned().unwrap_or_default();
        out.replace_range(start..start + 2 + end + 1, &replacement);
    }
    out
}

fn apply_setvars(tx: &mut HashMap<String, String>, vars: &[CrsSetVar]) {
    for var in vars {
        let value = expand_tx(&var.value, tx);
        match var.op {
            CrsSetVarOp::Set => {
                tx.insert(var.name.clone(), value);
            }
            CrsSetVarOp::Add => {
                let current = tx
                    .get(&var.name)
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(0);
                let delta = value.parse::<i64>().unwrap_or(0);
                tx.insert(var.name.clone(), (current + delta).to_string());
            }
            CrsSetVarOp::Sub => {
                let current = tx
                    .get(&var.name)
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(0);
                let delta = value.parse::<i64>().unwrap_or(0);
                tx.insert(var.name.clone(), (current - delta).to_string());
            }
            CrsSetVarOp::Remove => {
                tx.remove(&var.name);
            }
        }
    }
}

impl CrsMatcher {
    fn target_label(target: &CrsTarget) -> String {
        match target {
            CrsTarget::Path => "path".into(),
            CrsTarget::Method => "method".into(),
            CrsTarget::Header(name) if name.is_empty() => "headers".into(),
            CrsTarget::Header(name) => format!("header:{name}"),
            CrsTarget::HeaderName => "header_names".into(),
            CrsTarget::HeaderExcept(name) => format!("!header:{name}"),
            CrsTarget::Args => "args".into(),
            CrsTarget::ArgName => "args_names".into(),
            CrsTarget::QueryString => "query_string".into(),
            CrsTarget::Filename => "filename".into(),
            CrsTarget::Basename => "basename".into(),
            CrsTarget::Cookie => "cookies".into(),
            CrsTarget::CookieName => "cookie_names".into(),
            CrsTarget::Body => "body".into(),
            CrsTarget::ResponseStatus => "response_status".into(),
            CrsTarget::ResponseHeader(name) => format!("response_header:{name}"),
            CrsTarget::ResponseBody => "response_body".into(),
            CrsTarget::ClientIp => "client_ip".into(),
            CrsTarget::Unconditional => "unconditional".into(),
            CrsTarget::Tx(name) => format!("tx:{name}"),
            CrsTarget::TxCount(name) => format!("txcount:{name}"),
            CrsTarget::HeaderCount(name) => format!("header_count:{name}"),
            CrsTarget::ArgCount => "arg_count".into(),
            CrsTarget::CookieCount => "cookie_count".into(),
            CrsTarget::RequestLine => "request_line".into(),
            CrsTarget::RequestProtocol => "request_protocol".into(),
            CrsTarget::RequestBodyProcessor => "request_body_processor".into(),
            CrsTarget::UniqueId => "unique_id".into(),
            CrsTarget::Files => "files".into(),
            CrsTarget::FilesNames => "files_names".into(),
            CrsTarget::Xml => "xml".into(),
            CrsTarget::XmlAttributes => "xml_attributes".into(),
            CrsTarget::MatchedVars => "matched_vars".into(),
            CrsTarget::MatchedVarsNames => "matched_vars_names".into(),
            CrsTarget::ArgsNamed(name) => format!("arg:{name}"),
            CrsTarget::RequestBodyLength => "request_body_length".into(),
            CrsTarget::ArgsCombinedSize => "args_combined_size".into(),
            CrsTarget::FilesCombinedSize => "files_combined_size".into(),
        }
    }

    fn field_label(&self) -> String {
        self.targets
            .iter()
            .map(Self::target_label)
            .collect::<Vec<_>>()
            .join("|")
    }

    fn matches(
        &self,
        req: &WafRequest<'_>,
        views: &BodyViews<'_>,
        tx: &HashMap<String, String>,
    ) -> bool {
        self.matches_collect(req, views, tx).is_some()
    }

    /// Matched `(variable name, transformed value)` pairs, or `None` when the
    /// rule does not match. Drives `MATCHED_VAR` / `MATCHED_VAR_NAME`.
    fn matches_collect(
        &self,
        req: &WafRequest<'_>,
        views: &BodyViews<'_>,
        tx: &HashMap<String, String>,
    ) -> Option<Vec<TargetValue>> {
        let mut tx = tx.clone();
        for (name, value) in query_pairs(req) {
            tx.insert(format!("args.{name}"), value);
        }
        let excluded: Vec<String> = self
            .targets
            .iter()
            .filter_map(|target| match target {
                CrsTarget::HeaderExcept(name) => Some(name.to_ascii_lowercase()),
                _ => None,
            })
            .collect();
        let mut values = Vec::new();
        for target in &self.targets {
            values.extend(request_target_values(target, req, views, &tx, &excluded));
        }
        self.matches_values(&values, &tx)
    }

    fn matches_response(&self, resp: &WafResponse<'_>, tx: &HashMap<String, String>) -> bool {
        let mut values = Vec::new();
        for target in &self.targets {
            values.extend(response_target_values(target, resp, tx));
        }
        self.matches_values(&values, tx).is_some()
    }

    fn first_response_value(
        &self,
        resp: &WafResponse<'_>,
        tx: &HashMap<String, String>,
    ) -> Option<String> {
        for target in &self.targets {
            if let Some((_, value)) = response_target_values(target, resp, tx).into_iter().next() {
                return Some(apply_transforms(&value, &self.transforms));
            }
        }
        None
    }

    /// Values that satisfied the operator (empty for a negated rule, which
    /// matches precisely because nothing matched).
    fn matches_values(
        &self,
        values: &[TargetValue],
        tx: &HashMap<String, String>,
    ) -> Option<Vec<TargetValue>> {
        // A missing variable never matches, negated or not (ModSecurity: there is
        // nothing to negate). Without this, `!@rx` rules fire on absent headers.
        if values.is_empty() {
            return None;
        }
        let mut hits: Vec<TargetValue> = Vec::new();
        for (name, raw) in values {
            let value = apply_transforms(raw, &self.transforms);
            let hit = match &self.op {
                CrsOp::Rx(re) => re.is_match(&value),
                CrsOp::FancyRx(re) => match re.is_match(&value) {
                    Ok(hit) => hit,
                    Err(err) => {
                        // PCRE2 parity: an engine error is a non-match, never a
                        // silent success — but it is logged, not swallowed.
                        tracing::warn!(%err, "fancy-regex evaluation error; rule treated as no-match");
                        false
                    }
                },
                CrsOp::Contains(v) => value.contains(v.as_str()),
                CrsOp::StartsWith(v) => value.starts_with(v.as_str()),
                CrsOp::Eq(v) => value == expand_tx(v, tx),
                // ModSecurity @pm/@pmFromFile are case-insensitive phrase lists.
                CrsOp::Pm(items) => {
                    let haystack = value.to_ascii_lowercase();
                    items
                        .iter()
                        .any(|item| haystack.contains(&expand_tx(item, tx).to_ascii_lowercase()))
                }
                CrsOp::EqNum(n) => numeric_cmp(&value, n, tx).is_some_and(|(v, n)| v == n),
                CrsOp::Ge(n) => numeric_cmp(&value, n, tx).is_some_and(|(v, n)| v >= n),
                CrsOp::Gt(n) => numeric_cmp(&value, n, tx).is_some_and(|(v, n)| v > n),
                CrsOp::Le(n) => numeric_cmp(&value, n, tx).is_some_and(|(v, n)| v <= n),
                CrsOp::Lt(n) => numeric_cmp(&value, n, tx).is_some_and(|(v, n)| v < n),
                CrsOp::EndsWith(v) => value.ends_with(v.as_str()),
                CrsOp::VerifyCc => luhn_valid(&value),
                CrsOp::ValidateUtf8Encoding => std::str::from_utf8(value.as_bytes()).is_err(),
                CrsOp::ValidateUrlEncoding => {
                    let bytes = value.as_bytes();
                    let mut index = 0;
                    let mut invalid = false;
                    while index < bytes.len() {
                        if bytes[index] == b'%' {
                            if index + 2 >= bytes.len()
                                || !bytes[index + 1].is_ascii_hexdigit()
                                || !bytes[index + 2].is_ascii_hexdigit()
                            {
                                invalid = true;
                                break;
                            }
                            index += 3;
                        } else {
                            index += 1;
                        }
                    }
                    invalid
                }
                CrsOp::Within(items) => items.iter().any(|item| {
                    expand_tx(item, tx)
                        .split_whitespace()
                        .any(|part| value == part)
                }),
                CrsOp::IpMatch(cidrs) => value
                    .trim()
                    .parse::<std::net::IpAddr>()
                    .map(|ip| {
                        cidrs.iter().any(|cidr| {
                            cidr.parse::<ipnetwork::IpNetwork>()
                                .map(|network| network.contains(ip))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false),
                CrsOp::Unconditional => true,
                CrsOp::DetectSqli => crate::detect::detect_sql(&value),
                CrsOp::DetectXss => crate::detect::detect_xss(&value),
                CrsOp::ValidateByteRange(ranges) => value.bytes().any(|byte| {
                    !ranges
                        .iter()
                        .any(|(start, end)| byte >= *start && byte <= *end)
                }),
                CrsOp::NoMatch => false,
            };
            if hit {
                hits.push((name.clone(), value));
            }
        }
        if self.negated {
            hits.is_empty().then(Vec::new)
        } else {
            (!hits.is_empty()).then_some(hits)
        }
    }
}

fn luhn_valid(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .filter_map(|c| c.to_digit(10))
        .collect();
    if digits.len() < 13 {
        return false;
    }
    let mut sum = 0;
    for (index, digit) in digits.iter().rev().enumerate() {
        let mut digit = *digit;
        if index % 2 == 1 {
            digit *= 2;
            if digit > 9 {
                digit -= 9;
            }
        }
        sum += digit;
    }
    sum % 10 == 0
}

/// Publish the current rule's matched variables for `MATCHED_VAR`,
/// `MATCHED_VAR_NAME`, `MATCHED_VARS` and `MATCHED_VARS_NAMES`. ModSecurity
/// scopes these to the rule (or chain) being evaluated, not the transaction.
fn record_matched_vars(tx: &mut HashMap<String, String>, matched: &[TargetValue]) {
    tx.retain(|key, _| !key.starts_with("matched_vars."));
    tx.insert("matched_vars_count".to_string(), matched.len().to_string());
    for (index, (name, value)) in matched.iter().enumerate() {
        tx.insert(format!("matched_vars.{index}"), value.clone());
        tx.insert(format!("matched_vars_name.{index}"), name.clone());
    }
    if let Some((name, value)) = matched.last() {
        tx.insert("matched_var".to_string(), value.clone());
        tx.insert("matched_var_name".to_string(), name.clone());
    } else {
        tx.remove("matched_var");
        tx.remove("matched_var_name");
    }
}

fn apply_captures(tx: &mut HashMap<String, String>, op: &CrsOp, value: &str) {
    match op {
        CrsOp::Rx(re) => {
            if let Some(caps) = re.captures(value) {
                for index in 0..caps.len() {
                    tx.insert(
                        index.to_string(),
                        caps.get(index)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default(),
                    );
                }
            }
        }
        CrsOp::FancyRx(re) => {
            if let Ok(Some(caps)) = re.captures(value) {
                for index in 0..caps.len() {
                    tx.insert(
                        index.to_string(),
                        caps.get(index)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default(),
                    );
                }
            }
        }
        _ => {}
    }
}

fn numeric_cmp(value: &str, operand: &str, tx: &HashMap<String, String>) -> Option<(i64, i64)> {
    let value = value.trim().parse::<i64>().ok()?;
    let operand = expand_tx(operand, tx).parse::<i64>().ok()?;
    Some((value, operand))
}

fn cookie_pairs(req: &WafRequest<'_>) -> Vec<(String, String)> {
    req.headers
        .get("cookie")
        .map(|value| {
            String::from_utf8_lossy(value)
                .split(';')
                .filter_map(|pair| {
                    let pair = pair.trim();
                    let (name, value) = pair.split_once('=')?;
                    Some((name.to_string(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// ModSecurity ARGS: GET parameters first, then urlencoded POST parameters.
fn all_args(req: &WafRequest<'_>, views: &BodyViews<'_>) -> Vec<(String, String)> {
    let mut args = query_pairs(req);
    args.extend(views.form.iter().cloned());
    args
}

fn query_pairs(req: &WafRequest<'_>) -> Vec<(String, String)> {
    req.query
        .map(|query| {
            query
                .split('&')
                .map(|pair| {
                    let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                    (url_decode(name), url_decode(value))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Per-request multipart/XML views, parsed once and reused by every rule.
pub(crate) struct BodyViews<'a> {
    body: Option<&'a [u8]>,
    files: Vec<crate::multipart::FilePart>,
    xml: Option<crate::xml::XmlValues>,
    /// `application/x-www-form-urlencoded` body parameters. ModSecurity's ARGS
    /// collection is GET + POST parameters, so these join the query pairs.
    form: Vec<(String, String)>,
}

impl<'a> BodyViews<'a> {
    fn new(headers: &dyn exyonq_waf_api::HeaderView, body: Option<&'a [u8]>) -> Self {
        let content_type = headers
            .get("content-type")
            .map(|value| String::from_utf8_lossy(value).to_string())
            .unwrap_or_default();
        let bytes = body.unwrap_or(&[]);
        Self {
            body,
            files: crate::multipart::file_parts(&content_type, bytes).unwrap_or_default(),
            xml: if crate::xml::is_xml_content_type(&content_type) {
                crate::xml::parse(bytes)
            } else {
                None
            },
            form: if content_type
                .to_ascii_lowercase()
                .starts_with("application/x-www-form-urlencoded")
            {
                crate::crs::form_pairs(bytes)
            } else {
                Vec::new()
            },
        }
    }
}

/// One evaluated target: the ModSecurity-style variable name plus its value.
pub(crate) type TargetValue = (String, String);

fn named(name: &str, value: String) -> TargetValue {
    (name.to_string(), value)
}

fn request_target_values(
    target: &CrsTarget,
    req: &WafRequest<'_>,
    views: &BodyViews<'_>,
    tx: &HashMap<String, String>,
    excluded_headers: &[String],
) -> Vec<TargetValue> {
    match target {
        CrsTarget::Path => vec![named(
            "REQUEST_URI",
            match req.query {
                Some(query) => format!("{}?{}", req.path, query),
                None => req.path.to_string(),
            },
        )],
        CrsTarget::Method => vec![named("REQUEST_METHOD", req.method.to_string())],
        CrsTarget::Header(name) if name.is_empty() => {
            let mut values = Vec::new();
            req.headers.for_each(&mut |header, value| {
                let header = String::from_utf8_lossy(header).to_ascii_lowercase();
                if !excluded_headers.iter().any(|excluded| excluded == &header) {
                    values.push(named(
                        &format!("REQUEST_HEADERS:{header}"),
                        String::from_utf8_lossy(value).to_string(),
                    ));
                }
            });
            values
        }
        CrsTarget::Header(name) => req
            .headers
            .get(name)
            .map(|value| {
                vec![named(
                    &format!("REQUEST_HEADERS:{name}"),
                    String::from_utf8_lossy(value).to_string(),
                )]
            })
            .unwrap_or_default(),
        CrsTarget::HeaderName => {
            let mut values = Vec::new();
            req.headers.for_each(&mut |name, _| {
                let name = String::from_utf8_lossy(name).to_string();
                values.push(named(&format!("REQUEST_HEADERS_NAMES:{name}"), name));
            });
            values
        }
        CrsTarget::Args => all_args(req, views)
            .into_iter()
            .map(|(name, value)| named(&format!("ARGS:{name}"), value))
            .collect(),
        CrsTarget::ArgName => all_args(req, views)
            .into_iter()
            .map(|(name, _)| named(&format!("ARGS_NAMES:{name}"), name))
            .collect(),
        CrsTarget::QueryString => vec![named("QUERY_STRING", req.query.unwrap_or("").to_string())],
        CrsTarget::Filename => vec![named("REQUEST_FILENAME", req.path.to_string())],
        CrsTarget::Basename => vec![named(
            "REQUEST_BASENAME",
            req.path.rsplit('/').next().unwrap_or("").to_string(),
        )],
        CrsTarget::Cookie => cookie_pairs(req)
            .into_iter()
            .map(|(name, value)| named(&format!("REQUEST_COOKIES:{name}"), value))
            .collect(),
        CrsTarget::CookieName => cookie_pairs(req)
            .into_iter()
            .map(|(name, _)| named(&format!("REQUEST_COOKIES_NAMES:{name}"), name))
            .collect(),
        CrsTarget::Body => views
            .body
            .map(|body| {
                vec![named(
                    "REQUEST_BODY",
                    String::from_utf8_lossy(body).to_string(),
                )]
            })
            .unwrap_or_default(),
        CrsTarget::ClientIp => vec![named("REMOTE_ADDR", req.client_ip.to_string())],
        CrsTarget::Tx(name) => tx
            .get(name)
            .cloned()
            .map(|value| vec![named(&format!("TX:{name}"), value)])
            .unwrap_or_default(),
        CrsTarget::TxCount(name) => vec![named(
            &format!("&TX:{name}"),
            if tx.contains_key(name) { "1" } else { "0" }.to_string(),
        )],
        CrsTarget::HeaderCount(name) if name.is_empty() => {
            let mut count = 0usize;
            req.headers.for_each(&mut |_, _| count += 1);
            vec![named("&REQUEST_HEADERS", count.to_string())]
        }
        CrsTarget::HeaderCount(name) => vec![named(
            &format!("&REQUEST_HEADERS:{name}"),
            req.headers
                .get(name)
                .map(|_| "1".to_string())
                .unwrap_or_else(|| "0".to_string()),
        )],
        CrsTarget::ArgCount => vec![named("&ARGS", all_args(req, views).len().to_string())],
        CrsTarget::CookieCount => vec![named(
            "&REQUEST_COOKIES",
            cookie_pairs(req).len().to_string(),
        )],
        CrsTarget::RequestLine => vec![named(
            "REQUEST_LINE",
            format!(
                "{} {}{} HTTP/1.1",
                req.method,
                req.path,
                req.query.map(|q| format!("?{q}")).unwrap_or_default()
            ),
        )],
        CrsTarget::RequestProtocol => vec![named("REQUEST_PROTOCOL", "HTTP/1.1".to_string())],
        CrsTarget::RequestBodyProcessor => {
            let content_type = req
                .headers
                .get("content-type")
                .map(|value| String::from_utf8_lossy(value).to_ascii_lowercase())
                .unwrap_or_default();
            let processor = if content_type.contains("multipart/form-data") {
                "MULTIPART"
            } else if content_type.contains("xml") {
                "XML"
            } else if content_type.contains("json") {
                "JSON"
            } else {
                "URLENCODED"
            };
            vec![named("REQBODY_PROCESSOR", processor.to_string())]
        }
        CrsTarget::UniqueId => vec![named(
            "UNIQUE_ID",
            req.headers
                .get("unique-id")
                .or_else(|| req.headers.get("x-request-id"))
                .map(|value| String::from_utf8_lossy(value).to_string())
                .unwrap_or_default(),
        )],
        CrsTarget::Files => views
            .files
            .iter()
            .map(|part| named(&format!("FILES:{}", part.field), part.filename.clone()))
            .collect(),
        CrsTarget::FilesNames => views
            .files
            .iter()
            .map(|part| named(&format!("FILES_NAMES:{}", part.field), part.field.clone()))
            .collect(),
        CrsTarget::Xml => views
            .xml
            .as_ref()
            .map(|xml| {
                xml.texts
                    .iter()
                    .map(|text| named("XML:/*", text.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        CrsTarget::XmlAttributes => views
            .xml
            .as_ref()
            .map(|xml| {
                xml.attrs
                    .iter()
                    .map(|attr| named("XML://@*", attr.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        CrsTarget::ArgsCombinedSize => vec![named(
            "ARGS_COMBINED_SIZE",
            all_args(req, views)
                .into_iter()
                .map(|(name, value)| name.len() + value.len())
                .sum::<usize>()
                .to_string(),
        )],
        CrsTarget::FilesCombinedSize => vec![named(
            "FILES_COMBINED_SIZE",
            views
                .files
                .iter()
                .map(|part| part.size)
                .sum::<usize>()
                .to_string(),
        )],
        CrsTarget::ArgsNamed(name) => all_args(req, views)
            .into_iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| vec![named(&format!("ARGS:{name}"), value)])
            .unwrap_or_default(),
        CrsTarget::RequestBodyLength => vec![named(
            "REQUEST_BODY_LENGTH",
            req.headers
                .get("content-length")
                .map(|value| String::from_utf8_lossy(value).to_string())
                .unwrap_or_else(|| {
                    views
                        .body
                        .map(|body| body.len().to_string())
                        .unwrap_or_default()
                }),
        )],
        CrsTarget::MatchedVars => matched_var_values(tx, false),
        CrsTarget::MatchedVarsNames => matched_var_values(tx, true),
        CrsTarget::Unconditional => vec![named("", String::new())],
        CrsTarget::HeaderExcept(_) => Vec::new(),
        _ => Vec::new(),
    }
}

/// Variables matched by the current rule (or its chain starter), stored by
/// `evaluate_phase` as `matched_vars_count` + indexed keys.
fn matched_var_values(tx: &HashMap<String, String>, names: bool) -> Vec<TargetValue> {
    let count = tx
        .get("matched_vars_count")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    (0..count)
        .filter_map(|index| {
            let value = tx.get(&format!("matched_vars.{index}"))?.clone();
            let name = tx
                .get(&format!("matched_vars_name.{index}"))
                .cloned()
                .unwrap_or_default();
            if names {
                Some(named("MATCHED_VARS_NAMES", name))
            } else {
                Some(named(&name, value))
            }
        })
        .collect()
}

fn response_target_values(
    target: &CrsTarget,
    resp: &WafResponse<'_>,
    tx: &HashMap<String, String>,
) -> Vec<TargetValue> {
    match target {
        CrsTarget::ResponseStatus => vec![named("RESPONSE_STATUS", resp.status.to_string())],
        CrsTarget::ResponseHeader(name) if name.is_empty() => {
            let mut values = Vec::new();
            resp.headers.for_each(&mut |header, value| {
                values.push(named(
                    &format!(
                        "RESPONSE_HEADERS:{}",
                        String::from_utf8_lossy(header).to_ascii_lowercase()
                    ),
                    String::from_utf8_lossy(value).to_string(),
                ));
            });
            values
        }
        CrsTarget::ResponseHeader(name) => resp
            .headers
            .get(name)
            .map(|value| {
                vec![named(
                    &format!("RESPONSE_HEADERS:{name}"),
                    String::from_utf8_lossy(value).to_string(),
                )]
            })
            .unwrap_or_default(),
        CrsTarget::ResponseBody => resp
            .body
            .map(|body| {
                vec![named(
                    "RESPONSE_BODY",
                    String::from_utf8_lossy(body).to_string(),
                )]
            })
            .unwrap_or_default(),
        CrsTarget::Tx(name) => tx
            .get(name)
            .cloned()
            .map(|value| vec![named(&format!("TX:{name}"), value)])
            .unwrap_or_default(),
        CrsTarget::Unconditional => vec![named("", String::new())],
        _ => Vec::new(),
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
