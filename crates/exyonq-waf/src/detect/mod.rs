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
//! Built-in EXY-* detectors (WAF3). Reimplemented for ExyonQ — not a Pingora port.

use crate::exclusion::CompiledExclusion;
use crate::normalize::{normalize_text, NormalizeBudget};
use exyonq_waf_api::{
    WafAction, WafEvidence, WafPhase, WafRequest, WafRuleId, WafTransform, WafViolation,
};
use ipnetwork::IpNetwork;
use regex::Regex;
use std::net::IpAddr;
use std::str::FromStr;

/// Toggle for one built-in detector family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectorToggle {
    pub enabled: bool,
    pub action: WafAction,
}

impl DetectorToggle {
    pub fn block() -> Self {
        Self {
            enabled: true,
            action: WafAction::Block,
        }
    }

    pub fn log() -> Self {
        Self {
            enabled: true,
            action: WafAction::Log,
        }
    }

    pub fn off() -> Self {
        Self {
            enabled: false,
            action: WafAction::Allow,
        }
    }
}

/// IP / CIDR filter config (compiled into snapshot).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpFilterInput {
    pub enabled: bool,
    pub whitelist: Vec<String>,
    pub blacklist: Vec<String>,
}

/// Built-in detector pack (exyonq-core ruleset).
#[derive(Debug, Clone)]
pub struct BuiltinDetectorsInput {
    pub sql_injection: DetectorToggle,
    pub xss: DetectorToggle,
    pub path_traversal: DetectorToggle,
    pub command_injection: DetectorToggle,
    pub header_anomaly: DetectorToggle,
    pub bot_ua: DetectorToggle,
    pub ip_filter: IpFilterInput,
}

impl Default for BuiltinDetectorsInput {
    fn default() -> Self {
        Self {
            sql_injection: DetectorToggle::block(),
            xss: DetectorToggle::block(),
            path_traversal: DetectorToggle::block(),
            command_injection: DetectorToggle::block(),
            // Weak signals — prefer Log until product policy hardens.
            header_anomaly: DetectorToggle::log(),
            bot_ua: DetectorToggle::log(),
            ip_filter: IpFilterInput::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PatternRule {
    pub id: &'static str,
    pub re: Regex,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledDetectors {
    pub normalize: NormalizeBudget,
    pub sql: Option<(DetectorToggle, Vec<PatternRule>)>,
    pub xss: Option<(DetectorToggle, Vec<PatternRule>)>,
    pub path: Option<(DetectorToggle, Vec<PatternRule>)>,
    pub cmd: Option<(DetectorToggle, Vec<PatternRule>)>,
    pub header_anomaly: Option<DetectorToggle>,
    pub bot: Option<(DetectorToggle, Vec<PatternRule>, Vec<&'static str>)>,
    pub ip: Option<CompiledIpFilter>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledIpFilter {
    pub action: WafAction,
    pub whitelist: Vec<IpNetwork>,
    pub blacklist: Vec<IpNetwork>,
}

const SAFE_HEADERS: &[&str] = &[
    "accept",
    "accept-encoding",
    "accept-language",
    "cache-control",
    "connection",
    "content-length",
    "content-type",
    "host",
    "sec-ch-ua",
    "sec-ch-ua-mobile",
    "sec-ch-ua-platform",
    "sec-fetch-dest",
    "sec-fetch-mode",
    "sec-fetch-site",
    "upgrade-insecure-requests",
];

pub(crate) fn compile_detectors(
    input: &BuiltinDetectorsInput,
    normalize: NormalizeBudget,
) -> Result<CompiledDetectors, String> {
    Ok(CompiledDetectors {
        normalize,
        sql: if input.sql_injection.enabled {
            Some((input.sql_injection, compile_patterns(SQL_PATTERNS)?))
        } else {
            None
        },
        xss: if input.xss.enabled {
            Some((input.xss, compile_patterns(XSS_PATTERNS)?))
        } else {
            None
        },
        path: if input.path_traversal.enabled {
            Some((input.path_traversal, compile_patterns(PATH_PATTERNS)?))
        } else {
            None
        },
        cmd: if input.command_injection.enabled {
            Some((input.command_injection, compile_patterns(CMD_PATTERNS)?))
        } else {
            None
        },
        header_anomaly: if input.header_anomaly.enabled {
            Some(input.header_anomaly)
        } else {
            None
        },
        bot: if input.bot_ua.enabled {
            Some((
                input.bot_ua,
                compile_patterns(BAD_BOT_PATTERNS)?,
                GOOD_BOT_IDS.to_vec(),
            ))
        } else {
            None
        },
        ip: if input.ip_filter.enabled {
            Some(compile_ip_filter(&input.ip_filter)?)
        } else {
            None
        },
    })
}

fn compile_patterns(specs: &[(&'static str, &'static str)]) -> Result<Vec<PatternRule>, String> {
    specs
        .iter()
        .map(|(id, pat)| {
            Regex::new(pat)
                .map(|re| PatternRule { id, re })
                .map_err(|e| format!("{id}: {e}"))
        })
        .collect()
}

fn compile_ip_filter(input: &IpFilterInput) -> Result<CompiledIpFilter, String> {
    let mut whitelist = Vec::new();
    for s in &input.whitelist {
        whitelist.push(parse_ip_or_cidr(s)?);
    }
    let mut blacklist = Vec::new();
    for s in &input.blacklist {
        blacklist.push(parse_ip_or_cidr(s)?);
    }
    Ok(CompiledIpFilter {
        action: WafAction::Block,
        whitelist,
        blacklist,
    })
}

fn parse_ip_or_cidr(s: &str) -> Result<IpNetwork, String> {
    if let Ok(net) = IpNetwork::from_str(s) {
        return Ok(net);
    }
    let ip = IpAddr::from_str(s).map_err(|_| format!("invalid IP/CIDR: {s}"))?;
    match ip {
        IpAddr::V4(v4) => IpNetwork::new(IpAddr::V4(v4), 32).map_err(|e| e.to_string()),
        IpAddr::V6(v6) => IpNetwork::new(IpAddr::V6(v6), 128).map_err(|e| e.to_string()),
    }
}

impl CompiledDetectors {
    pub(crate) fn evaluate(
        &self,
        phase: WafPhase,
        req: &WafRequest<'_>,
        body_chunk: Option<&[u8]>,
        exclusions: &[CompiledExclusion],
    ) -> Vec<(WafViolation, WafAction)> {
        let mut hits = Vec::new();

        match phase {
            WafPhase::Connection | WafPhase::RequestHeaders => {
                if let Some(ip) = &self.ip {
                    if let Some(v) = check_ip(ip, req.client_ip) {
                        push_hit(&mut hits, exclusions, req, v, ip.action);
                    }
                }
                if matches!(phase, WafPhase::RequestHeaders) {
                    self.scan_request_fields(req, &mut hits, exclusions);
                    if let Some(toggle) = self.header_anomaly {
                        if let Some(v) = check_header_anomaly(req, WafPhase::RequestHeaders) {
                            push_hit(&mut hits, exclusions, req, v, toggle.action);
                        }
                    }
                    if let Some((toggle, bad, good)) = &self.bot {
                        if let Some(v) = check_bot(req, bad, good) {
                            push_hit(&mut hits, exclusions, req, v, toggle.action);
                        }
                    }
                }
            }
            WafPhase::RequestBodyChunk | WafPhase::RequestBodyComplete => {
                if let Some(raw) = body_chunk {
                    // Lossy UTF-8: binary bodies must still be inspected (no silent skip).
                    let body = String::from_utf8_lossy(raw);
                    self.scan_haystack(
                        body.as_ref(),
                        "body",
                        phase,
                        req,
                        &mut hits,
                        exclusions,
                        /*injection*/ true,
                    );
                }
            }
            WafPhase::ResponseHeaders | WafPhase::ResponseBodyChunk => {}
        }

        hits
    }

    fn scan_request_fields(
        &self,
        req: &WafRequest<'_>,
        hits: &mut Vec<(WafViolation, WafAction)>,
        exclusions: &[CompiledExclusion],
    ) {
        self.scan_haystack(
            req.path,
            "path",
            WafPhase::RequestHeaders,
            req,
            hits,
            exclusions,
            true,
        );
        if let Some(q) = req.query {
            self.scan_haystack(
                q,
                "query",
                WafPhase::RequestHeaders,
                req,
                hits,
                exclusions,
                true,
            );
        }
        req.headers.for_each(&mut |name, value| {
            // Cap015 / WAF-LOGIC-P2-G: inspect lossy UTF-8 — do not skip obs-text /
            // invalid bytes (would fail-open past header signature rules).
            let name_str = String::from_utf8_lossy(name);
            if is_safe_header(name_str.as_ref()) {
                return;
            }
            let val = String::from_utf8_lossy(value);
            let field = format!("header:{name_str}");
            self.scan_haystack(
                val.as_ref(),
                &field,
                WafPhase::RequestHeaders,
                req,
                hits,
                exclusions,
                true,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_haystack(
        &self,
        raw: &str,
        field: &str,
        phase: WafPhase,
        req: &WafRequest<'_>,
        hits: &mut Vec<(WafViolation, WafAction)>,
        exclusions: &[CompiledExclusion],
        injection_families: bool,
    ) {
        let norm = normalize_text(raw, self.normalize);
        let hay = norm.text.as_ref();

        if injection_families {
            if let Some((toggle, rules)) = &self.sql {
                if let Some(id) = first_match(rules, hay) {
                    push_hit(
                        hits,
                        exclusions,
                        req,
                        violation(id, phase, field, "sql injection pattern", &norm.transforms),
                        toggle.action,
                    );
                }
            }
            if let Some((toggle, rules)) = &self.xss {
                if let Some(id) = first_match(rules, hay) {
                    push_hit(
                        hits,
                        exclusions,
                        req,
                        violation(id, phase, field, "xss pattern", &norm.transforms),
                        toggle.action,
                    );
                }
            }
            if let Some((toggle, rules)) = &self.path {
                if let Some(id) = first_match(rules, hay).or_else(|| first_match(rules, raw)) {
                    push_hit(
                        hits,
                        exclusions,
                        req,
                        violation(
                            id,
                            phase,
                            field,
                            "path traversal / sensitive path",
                            &norm.transforms,
                        ),
                        toggle.action,
                    );
                }
            }
            if let Some((toggle, rules)) = &self.cmd {
                if let Some(id) = first_match(rules, hay).or_else(|| first_match(rules, raw)) {
                    push_hit(
                        hits,
                        exclusions,
                        req,
                        violation(
                            id,
                            phase,
                            field,
                            "command injection pattern",
                            &norm.transforms,
                        ),
                        toggle.action,
                    );
                }
            }
        }
    }
}

fn first_match(rules: &[PatternRule], hay: &str) -> Option<&'static str> {
    if hay.len() < 3 {
        return None;
    }
    rules.iter().find(|r| r.re.is_match(hay)).map(|r| r.id)
}

fn push_hit(
    hits: &mut Vec<(WafViolation, WafAction)>,
    exclusions: &[CompiledExclusion],
    req: &WafRequest<'_>,
    v: WafViolation,
    action: WafAction,
) {
    if exclusions.iter().any(|ex| ex.matches(req, &v.rule_id)) {
        return;
    }
    // Dedup same rule_id in one evaluate call.
    if hits.iter().any(|(h, _)| h.rule_id == v.rule_id) {
        return;
    }
    hits.push((v, action));
}

fn violation(
    id: &str,
    phase: WafPhase,
    field: &str,
    message: &str,
    transforms: &[WafTransform],
) -> WafViolation {
    WafViolation {
        rule_id: WafRuleId::new(id),
        phase,
        message: message.into(),
        evidence: WafEvidence {
            field: Some(field.into()),
            matched: None,
            transforms: transforms.to_vec(),
        },
    }
}

fn is_safe_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SAFE_HEADERS.iter().any(|h| *h == lower)
}

fn check_ip(filter: &CompiledIpFilter, ip: IpAddr) -> Option<WafViolation> {
    if !filter.whitelist.is_empty() && !filter.whitelist.iter().any(|n| n.contains(ip)) {
        return Some(violation(
            "EXY-IP-1002",
            WafPhase::RequestHeaders,
            "client_ip",
            "client IP not in whitelist",
            &[],
        ));
    }
    if filter.blacklist.iter().any(|n| n.contains(ip)) {
        return Some(violation(
            "EXY-IP-1001",
            WafPhase::RequestHeaders,
            "client_ip",
            "client IP blacklisted",
            &[],
        ));
    }
    None
}

fn check_header_anomaly(req: &WafRequest<'_>, phase: WafPhase) -> Option<WafViolation> {
    let ua = req.headers.get("user-agent");
    if ua.is_none() || ua.is_some_and(|v| v.is_empty()) {
        return Some(violation(
            "EXY-HDR-1001",
            phase,
            "header:user-agent",
            "missing or empty User-Agent",
            &[],
        ));
    }
    let mut oversized = false;
    req.headers.for_each(&mut |_n, v| {
        if v.len() > 8192 {
            oversized = true;
        }
    });
    if oversized {
        return Some(violation(
            "EXY-HDR-1002",
            phase,
            "headers",
            "header value exceeds 8KiB",
            &[],
        ));
    }
    None
}

fn check_bot(req: &WafRequest<'_>, bad: &[PatternRule], good: &[&str]) -> Option<WafViolation> {
    let Some(raw) = req.headers.get("user-agent") else {
        return None; // handled by header anomaly
    };
    // Lossy: non-UTF8 UA must still be evaluated (WAF-LOGIC-P2-G).
    let ua = String::from_utf8_lossy(raw);
    let lower = ua.to_ascii_lowercase();
    if good.iter().any(|g| lower.contains(g)) {
        return None;
    }
    if let Some(id) = first_match(bad, ua.as_ref()) {
        return Some(violation(
            id,
            WafPhase::RequestHeaders,
            "header:user-agent",
            "suspicious or malicious User-Agent",
            &[],
        ));
    }
    None
}

// --- Pattern tables (ExyonQ-owned IDs; compact v1 set) ---

const SQL_PATTERNS: &[(&str, &str)] = &[
    ("EXY-SQL-1001", r"(?i)\bunion\b[\s/\*]+.*\bselect\b"),
    ("EXY-SQL-1002", r"(?i)\b(or|and)\b\s+\d+\s*=\s*\d+"),
    (
        "EXY-SQL-1003",
        r"(?i)\b(sleep|benchmark|waitfor\s+delay)\s*\(",
    ),
];

const XSS_PATTERNS: &[(&str, &str)] = &[
    ("EXY-XSS-1001", r"(?i)<\s*script\b"),
    (
        "EXY-XSS-1002",
        r"(?i)\bon(?:load|error|click|mouseover)\s*=",
    ),
    ("EXY-XSS-1003", r"(?i)javascript\s*:"),
];

const PATH_PATTERNS: &[(&str, &str)] = &[
    ("EXY-PATH-1001", r"(?i)(\.\./|\.\.\\|%2e%2e%2f|%2e%2e/)"),
    (
        "EXY-PATH-1002",
        r"(?i)(/etc/passwd|\.htaccess|\.env\b|id_rsa|/proc/self)",
    ),
];

const CMD_PATTERNS: &[(&str, &str)] = &[
    (
        "EXY-CMD-1001",
        r"(?i)(;\s*(ls|cat|id|wget|curl)\b|\|\s*(ls|cat|id)\b|`[^`]+`|\$\([^)]+\))",
    ),
    ("EXY-CMD-1002", r"(?i)(/bin/(?:ba)?sh|cmd\.exe|powershell)"),
];

const BAD_BOT_PATTERNS: &[(&str, &str)] = &[
    (
        "EXY-BOT-1001",
        r"(?i)\b(sqlmap|nikto|nmap|masscan|burpsuite|acunetix)\b",
    ),
    (
        "EXY-BOT-1002",
        r"(?i)\b(dirbuster|gobuster|wpscan|nessus)\b",
    ),
];

const GOOD_BOT_IDS: &[&str] = &[
    "googlebot",
    "bingbot",
    "slurp",
    "duckduckbot",
    "facebookexternalhit",
    "twitterbot",
];
