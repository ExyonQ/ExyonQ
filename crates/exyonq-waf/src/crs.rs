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
    for action_text in actions.split(',') {
        let action_text = action_text.trim();
        if let Some(v) = action_text.strip_prefix("id:") {
            id = v.parse().unwrap_or(0);
        } else if let Some(v) = action_text.strip_prefix("setvar:tx.inbound_anomaly_score=+") {
            score = v.parse().unwrap_or(5);
        } else if action_text == "deny" || action_text == "block" {
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
                "lowercase" => transforms.push(CrsTransform::Lowercase),
                "urlDecode" | "urlDecodeUni" => transforms.push(CrsTransform::UrlDecode),
                "trim" => transforms.push(CrsTransform::Trim),
                "removeNulls" => transforms.push(CrsTransform::RemoveNulls),
                "base64Decode" => transforms.push(CrsTransform::Base64Decode),
                "htmlEntityDecode" => transforms.push(CrsTransform::HtmlEntityDecode),
                "removeWhitespace" => transforms.push(CrsTransform::RemoveWhitespace),
                "compressWhitespace" => transforms.push(CrsTransform::CompressWhitespace),
                "normalizePath" | "normalisePath" => transforms.push(CrsTransform::NormalizePath),
                "hexDecode" => transforms.push(CrsTransform::HexDecode),
                "length" => transforms.push(CrsTransform::Length),
                "hexEncode" => transforms.push(CrsTransform::HexEncode),
                "jsDecode" => transforms.push(CrsTransform::JsDecode),
                "cssDecode" => transforms.push(CrsTransform::CssDecode),
                "base64DecodeExt" => transforms.push(CrsTransform::Base64DecodeExt),
                "escapeSeqDecode" => transforms.push(CrsTransform::EscapeSeqDecode),
                _ => {}
            }
        }
    }
    if deny {
        action = WafAction::Block;
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
    }
}

fn parse_action_only(actions: &str) -> Option<CrsRuleInput> {
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
    } = parse_actions(actions);
    if rule_id == 0 {
        return None;
    }
    Some(CrsRuleInput {
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
    })
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
        "MULTIPART_PART_HEADERS" => CrsTarget::Header(String::new()),
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
        other if other.starts_with("TX:") => {
            CrsTarget::Tx(other["TX:".len()..].to_ascii_lowercase())
        }
        other if other.starts_with("&TX:") => {
            CrsTarget::TxCount(other["&TX:".len()..].to_ascii_lowercase())
        }
        "&REQUEST_HEADERS" => CrsTarget::HeaderCount(String::new()),
        "&MULTIPART_PART_HEADERS" => CrsTarget::HeaderCount(String::new()),
        "&ARGS" => CrsTarget::ArgCount,
        "&REQUEST_COOKIES" => CrsTarget::CookieCount,
        other if other.starts_with("&REQUEST_HEADERS:") => {
            CrsTarget::HeaderCount(other["&REQUEST_HEADERS:".len()..].to_string())
        }
        other if other.starts_with("&MULTIPART_PART_HEADERS:") => {
            CrsTarget::HeaderCount(other["&MULTIPART_PART_HEADERS:".len()..].to_string())
        }
        other if other.starts_with("REQUEST_COOKIES:/") => CrsTarget::Cookie,
        other if other.starts_with("REQUEST_COOKIES_NAMES:/") => CrsTarget::CookieName,
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
) -> Option<(CrsRuleInput, bool)> {
    if tokens.len() < 2 {
        return None;
    }
    let actions = tokens.get(2).map(String::as_str).unwrap_or("");
    let targets: Vec<CrsTarget> = tokens[0].split('|').filter_map(map_target).collect();
    if targets.is_empty() {
        return None;
    }
    let (negated, operator) = if let Some(rest) = tokens[1].strip_prefix('!') {
        (true, rest)
    } else {
        (false, tokens[1].as_str())
    };
    let op = if let Some(pattern) = operator.strip_prefix("@rx ") {
        match regex::Regex::new(pattern) {
            Ok(re) => CrsOp::Rx(re),
            Err(_) => match fancy_regex::Regex::new(pattern) {
                Ok(re) => CrsOp::FancyRx(std::sync::Arc::new(re)),
                Err(_) => return None,
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
        CrsOp::IpMatch(load_list(base_dir, v.trim())?)
    } else if let Some(v) = operator.strip_prefix("@pmFromFile ") {
        CrsOp::Pm(load_list(base_dir, v.trim())?)
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
        return None;
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
    } = parse_actions(actions);
    if rule_id == 0 && !allow_no_id {
        return None;
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
    Some((
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
                if let Some(rule) = parse_action_only(actions) {
                    rules.push(rule);
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
        let Some((mut rule, mut chained)) =
            parse_rule_tokens(&tokenize(&line["SecRule ".len()..]), base_dir, false)
        else {
            continue;
        };
        let mut chain_broken = false;
        while chained {
            while i < lines.len() && (lines[i].is_empty() || lines[i].starts_with('#')) {
                i += 1;
            }
            if i >= lines.len() || !lines[i].starts_with("SecRule ") {
                chain_broken = true;
                break;
            }
            let Some((next, next_chained)) =
                parse_rule_tokens(&tokenize(&lines[i]["SecRule ".len()..]), base_dir, true)
            else {
                chain_broken = true;
                break;
            };
            i += 1;
            rule.chain.push(next);
            chained = next_chained;
        }
        if chain_broken {
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
    }
}

/// Parse an `application/x-www-form-urlencoded` body into name/value pairs.
pub(crate) fn form_pairs(body: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(body)
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (url_decode(name), url_decode(value))
        })
        .collect()
}

pub(crate) fn url_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
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
            b'+' => {
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
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
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

pub(crate) fn apply_transforms(value: &str, transforms: &[CrsTransform]) -> String {
    let mut out = value.to_string();
    for transform in transforms {
        out = match transform {
            CrsTransform::Lowercase => out.to_ascii_lowercase(),
            CrsTransform::UrlDecode => url_decode(&out),
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
                let mut normalized = out.replace('\\', "/");
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
            CrsTransform::HexEncode => out
                .bytes()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
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
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let path = std::path::Path::new("/tmp/crs-real/main.conf");
        if !path.exists() {
            return;
        }
        let contents = std::fs::read_to_string(path).unwrap();
        let ruleset = parse_rule_file_with_base("crs", &contents, 5, path.parent());
        let parsed: std::collections::HashSet<String> =
            ruleset.rules.iter().map(|rule| rule.id.clone()).collect();
        let mut visited = std::collections::HashSet::new();
        let expanded = expand_includes(&contents, path.parent(), &mut visited);
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
        eprintln!("ALL={} PARSED={}", all.len(), parsed.len());
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
        let safe = WafRequest { query: None, ..hit };
        assert_ne!(
            engine.inspect(WafPhase::RequestHeaders, &safe).action,
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
