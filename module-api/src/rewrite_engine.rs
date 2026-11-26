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
//! NG-02 / ADR-048: patterned rewrite evaluation (compiled regex, $1..$9).
//! ADR-062 / F1-R07: also `$uri` (path) and `$host` (normalized).

use crate::path_canon::{decode_percent_path, escape_decoded_uri_bytes, escape_uri_bytes};
use exyonq_config_ir::{RewriteCondConfig, RewriteConfig, RewriteFlagConfig, RewriteRuleConfig};
use regex::bytes::Regex as BytesRegex;
use regex::Regex;
use std::borrow::Cow;

/// Request context for rewrite evaluation (no socket types).
#[derive(Debug, Clone, Copy)]
pub struct RewriteRequest<'a> {
    pub path: &'a str,
    pub host: Option<&'a str>,
    pub query: &'a str,
    /// Lowercased header name → value pairs already normalized by the caller.
    pub headers: &'a [(String, String)],
}

/// Outcome of applying a route rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteApplyOutcome {
    NoChange,
    /// Internal URI change. `rematch` is true for `last` / Cap035 literal.
    Internal {
        path: String,
        rematch: bool,
    },
    Redirect {
        status: u16,
        location: String,
    },
    Rejected,
}

/// Precompiled rewrite for one route (plan-build).
#[derive(Debug, Clone)]
pub enum CompiledRewrite {
    Literal(String),
    Rule {
        re: Regex,
        /// Byte-wise twin of `re`, used only when the decoded path is not UTF-8
        /// so a rule matches nginx's `$uri` byte for byte (`None` when the
        /// pattern needs Unicode classes that bytes mode does not provide).
        re_bytes: Option<BytesRegex>,
        replacement: String,
        flag: RewriteFlagConfig,
        when: Vec<CompiledWhen>,
    },
}

#[derive(Debug, Clone)]
pub enum CompiledWhen {
    Path {
        re: Regex,
        /// Byte-wise twin of `re`, for a path condition evaluated against a
        /// decoded target that is not UTF-8 (see `when_match`).
        re_bytes: Option<BytesRegex>,
    },
    Host(String),
    Query(Regex),
    Header {
        name: String,
        value: Option<String>,
    },
}

#[derive(Debug)]
pub enum CompileRewriteError {
    Regex(String),
}

impl std::fmt::Display for CompileRewriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Regex(d) => write!(f, "rewrite compile: {d}"),
        }
    }
}

impl std::error::Error for CompileRewriteError {}

pub fn compile_rewrite(cfg: &RewriteConfig) -> Result<CompiledRewrite, CompileRewriteError> {
    match cfg {
        RewriteConfig::Literal(path) => Ok(CompiledRewrite::Literal(path.clone())),
        RewriteConfig::Rule(rule) => compile_rule(rule),
    }
}

fn compile_rule(rule: &RewriteRuleConfig) -> Result<CompiledRewrite, CompileRewriteError> {
    let re = Regex::new(&rule.pattern)
        .map_err(|e| CompileRewriteError::Regex(format!("pattern: {e}")))?;
    let mut when = Vec::with_capacity(rule.when.len());
    for cond in &rule.when {
        when.push(compile_when(cond)?);
    }
    let re_bytes = regex::bytes::RegexBuilder::new(&rule.pattern)
        .unicode(false)
        .build()
        .ok();
    Ok(CompiledRewrite::Rule {
        re,
        re_bytes,
        replacement: rule.replacement.clone(),
        flag: rule.flag,
        when,
    })
}

fn compile_when(cond: &RewriteCondConfig) -> Result<CompiledWhen, CompileRewriteError> {
    if let Some(path) = &cond.path {
        let re =
            Regex::new(path).map_err(|e| CompileRewriteError::Regex(format!("when.path: {e}")))?;
        let re_bytes = regex::bytes::RegexBuilder::new(path)
            .unicode(false)
            .build()
            .ok();
        return Ok(CompiledWhen::Path { re, re_bytes });
    }
    if let Some(host) = &cond.host {
        return Ok(CompiledWhen::Host(normalize_host(host)));
    }
    if let Some(query) = &cond.query {
        let re = Regex::new(query)
            .map_err(|e| CompileRewriteError::Regex(format!("when.query: {e}")))?;
        return Ok(CompiledWhen::Query(re));
    }
    if let Some(header) = &cond.header {
        return Ok(CompiledWhen::Header {
            name: header.to_ascii_lowercase(),
            value: cond.header_value.clone(),
        });
    }
    Err(CompileRewriteError::Regex("empty when".into()))
}

fn normalize_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// Apply compiled rewrite against a request.
pub fn apply_compiled_rewrite(
    compiled: &CompiledRewrite,
    req: RewriteRequest<'_>,
) -> RewriteApplyOutcome {
    match compiled {
        CompiledRewrite::Literal(path) => {
            if path_valid_internal(path) {
                RewriteApplyOutcome::Internal {
                    path: path.clone(),
                    rematch: true,
                }
            } else {
                RewriteApplyOutcome::Rejected
            }
        }
        CompiledRewrite::Rule {
            re,
            re_bytes,
            replacement,
            flag,
            when,
        } => {
            // nginx matches against `$uri`, the decoded normalized path, and
            // re-encodes what it forwards. The canonical path keeps its escapes,
            // so decode one pass here.
            let decoded_bytes = decode_percent_path(req.path);
            let decoded_text = decoded_bytes
                .as_deref()
                .and_then(|bytes| std::str::from_utf8(bytes).ok());
            let (haystack, decoded) = match decoded_text {
                Some(text) => (Cow::Owned(text.to_owned()), true),
                None => (Cow::Borrowed(req.path), false),
            };
            let req = RewriteRequest {
                path: haystack.as_ref(),
                ..req
            };
            // A path condition obeys the same rule as the pattern: the URI form
            // first, and the decoded bytes when that form is not UTF-8.
            let byte_haystack = if decoded {
                None
            } else {
                decoded_bytes.as_deref()
            };
            if !when_match(when, req, byte_haystack) {
                return RewriteApplyOutcome::NoChange;
            }
            if let Some(caps) = re.captures(req.path) {
                let Ok(expanded) = expand_replacement(replacement, &caps, req) else {
                    return RewriteApplyOutcome::Rejected;
                };
                let expanded = if decoded {
                    escape_decoded_uri_bytes(&expanded)
                } else {
                    expanded
                };
                return finish_rule(*flag, expanded, req);
            }
            // A decoded path that is not UTF-8 cannot be spelled by a regex
            // literal, so the text rule above matches the URI form. nginx still
            // matches `$uri` byte to byte there: run the byte-wise twin on the
            // decoded bytes, which is what makes `^/i.ndex$` match `%FF`.
            if !decoded {
                if let (Some(bytes), Some(bytes_re)) = (decoded_bytes.as_deref(), re_bytes.as_ref())
                {
                    if let Some(caps) = bytes_re.captures(bytes) {
                        let Ok(expanded) = expand_replacement_bytes(replacement, &caps, req, bytes)
                        else {
                            return RewriteApplyOutcome::Rejected;
                        };
                        return finish_rule(*flag, escape_uri_bytes(&expanded), req);
                    }
                }
            }
            RewriteApplyOutcome::NoChange
        }
    }
}

fn finish_rule(
    flag: RewriteFlagConfig,
    expanded: String,
    req: RewriteRequest<'_>,
) -> RewriteApplyOutcome {
    match flag {
        RewriteFlagConfig::Last => {
            if path_valid_internal(&expanded) {
                RewriteApplyOutcome::Internal {
                    path: expanded,
                    rematch: true,
                }
            } else {
                RewriteApplyOutcome::Rejected
            }
        }
        RewriteFlagConfig::Break => {
            if path_valid_internal(&expanded) {
                RewriteApplyOutcome::Internal {
                    path: expanded,
                    rematch: false,
                }
            } else {
                RewriteApplyOutcome::Rejected
            }
        }
        RewriteFlagConfig::Redirect => {
            let location = append_query(expanded, req.query);
            if location_valid(&location) {
                RewriteApplyOutcome::Redirect {
                    status: 302,
                    location,
                }
            } else {
                RewriteApplyOutcome::Rejected
            }
        }
        RewriteFlagConfig::Permanent => {
            let location = append_query(expanded, req.query);
            if location_valid(&location) {
                RewriteApplyOutcome::Redirect {
                    status: 301,
                    location,
                }
            } else {
                RewriteApplyOutcome::Rejected
            }
        }
    }
}

/// nginx/Apache preserve the original query string on an external redirect
/// when the replacement does not declare its own `?` separator. Without this,
/// a canonical-host redirect silently drops campaign/query parameters.
fn append_query(location: String, query: &str) -> String {
    if query.is_empty() || location.contains('?') || location.contains('#') {
        return location;
    }
    format!("{location}?{query}")
}

fn when_match(
    when: &[CompiledWhen],
    req: RewriteRequest<'_>,
    byte_haystack: Option<&[u8]>,
) -> bool {
    when.iter().all(|w| match w {
        CompiledWhen::Path { re, re_bytes } => {
            re.is_match(req.path)
                || byte_haystack.is_some_and(|bytes| {
                    re_bytes
                        .as_ref()
                        .is_some_and(|bytes_re| bytes_re.is_match(bytes))
                })
        }
        CompiledWhen::Host(expected) => req
            .host
            .map(|h| normalize_host(h) == *expected)
            .unwrap_or(false),
        CompiledWhen::Query(re) => re.is_match(req.query),
        CompiledWhen::Header { name, value } => {
            let found = req
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name));
            match (found, value) {
                (None, _) => false,
                (Some((_, v)), None) => !v.is_empty(),
                (Some((_, v)), Some(want)) => v == want,
            }
        }
    })
}

fn expand_replacement(
    template: &str,
    caps: &regex::Captures<'_>,
    req: RewriteRequest<'_>,
) -> Result<String, ()> {
    // ADR-048 + ADR-062 (F1-R07): `$1`..`$9`, `$$`, `$uri`, `$host`.
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('$') => out.push('$'),
            Some(d) if ('1'..='9').contains(&d) => {
                let idx = (d as u8 - b'0') as usize;
                if let Some(m) = caps.get(idx) {
                    out.push_str(m.as_str());
                }
            }
            Some(c0) if c0.is_ascii_alphabetic() || c0 == '_' => {
                let mut name = String::new();
                name.push(c0);
                while matches!(chars.peek(), Some(n) if n.is_ascii_alphanumeric() || *n == '_') {
                    name.push(chars.next().expect("peeked"));
                }
                match name.as_str() {
                    "uri" => out.push_str(req.path),
                    "host" => {
                        let h = req.host.unwrap_or("");
                        out.push_str(&normalize_host_port(h));
                    }
                    _ => return Err(()),
                }
            }
            _ => return Err(()),
        }
    }
    Ok(out)
}

/// Byte-wise `$1..$9`, `$$`, `$uri`, `$host` expansion.
///
/// Each captured byte lands as-is and is escaped once by the caller, exactly
/// like a text capture: nginx re-encodes `$uri` bytes and does not care whether
/// they were UTF-8 to begin with.
fn expand_replacement_bytes(
    template: &str,
    caps: &regex::bytes::Captures<'_>,
    req: RewriteRequest<'_>,
    decoded: &[u8],
) -> Result<Vec<u8>, ()> {
    let mut out = Vec::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '$' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('$') => out.push(b'$'),
            Some(d) if ('1'..='9').contains(&d) => {
                let idx = (d as u8 - b'0') as usize;
                if let Some(m) = caps.get(idx) {
                    out.extend_from_slice(m.as_bytes());
                }
            }
            Some(c0) if c0.is_ascii_alphabetic() || c0 == '_' => {
                let mut name = String::new();
                name.push(c0);
                while matches!(chars.peek(), Some(n) if n.is_ascii_alphanumeric() || *n == '_') {
                    name.push(chars.next().expect("peeked"));
                }
                match name.as_str() {
                    // `$uri` is the decoded path, which is exactly `decoded` here.
                    "uri" => out.extend_from_slice(decoded),
                    "host" => {
                        let h = req.host.unwrap_or("");
                        out.extend_from_slice(normalize_host_port(h).as_bytes());
                    }
                    _ => return Err(()),
                }
            }
            _ => return Err(()),
        }
    }
    Ok(out)
}

fn normalize_host_port(host: &str) -> String {
    let base = normalize_host(host);
    match base.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name.to_string(),
        _ => base,
    }
}

fn path_valid_internal(path: &str) -> bool {
    !path.is_empty()
        && path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('?')
        && !path.contains('#')
        && !path.as_bytes().iter().any(|&b| b <= 0x20 || b == 0x7f)
        && path.parse::<http::Uri>().is_ok()
}

fn location_valid(location: &str) -> bool {
    !location.is_empty() && !location.as_bytes().iter().any(|&b| b <= 0x20 || b == 0x7f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_config_ir::RewriteRuleConfig;

    #[test]
    fn capture_last_rematch() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/blog/(.*)$".into(),
            replacement: "/posts/$1".into(),
            flag: RewriteFlagConfig::Last,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(
            &compiled,
            RewriteRequest {
                path: "/blog/hello",
                host: None,
                query: "",
                headers: &[],
            },
        );
        assert_eq!(
            out,
            RewriteApplyOutcome::Internal {
                path: "/posts/hello".into(),
                rematch: true,
            }
        );
    }

    fn request(path: &str) -> RewriteRequest<'_> {
        RewriteRequest {
            path,
            host: None,
            query: "",
            headers: &[],
        }
    }

    /// nginx matches `rewrite` against `$uri`, the decoded path, so a rule
    /// written with the decoded literal must match an escaped request-target and
    /// its replacement must come back in URI form.
    #[test]
    fn decoded_literal_matches_an_escaped_target() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/(a b\.txt)$".into(),
            replacement: "/dst/$1".into(),
            flag: RewriteFlagConfig::Last,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(&compiled, request("/a%20b.txt"));
        assert_eq!(
            out,
            RewriteApplyOutcome::Internal {
                path: "/dst/a%20b.txt".into(),
                rematch: true,
            }
        );
    }

    /// The capture is decoded text, so a redirect `Location` is escaped too
    /// instead of carrying a space.
    #[test]
    fn redirect_location_escapes_a_decoded_capture() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/(a b\.txt)$".into(),
            replacement: "https://example.com/$1".into(),
            flag: RewriteFlagConfig::Permanent,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(&compiled, request("/a%20b.txt"));
        assert_eq!(
            out,
            RewriteApplyOutcome::Redirect {
                status: 301,
                location: "https://example.com/a%20b.txt".into(),
            }
        );
    }

    /// Bytes that are not UTF-8 still match a decoded literal, as nginx does:
    /// the byte-wise twin sees `/raw/i\xFFndex`, so `i.ndex` cashes.
    #[test]
    fn non_utf8_target_matches_decoded_bytes() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/raw/(i.ndex)$".into(),
            replacement: "/mirror/$1".into(),
            flag: RewriteFlagConfig::Last,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(&compiled, request("/raw/i%FFndex"));
        assert_eq!(
            out,
            RewriteApplyOutcome::Internal {
                path: "/mirror/i%FFndex".into(),
                rematch: true,
            }
        );
    }

    /// A rule written against the escaped spelling keeps working: the text match
    /// on the URI form runs first and is not replaced.
    #[test]
    fn non_utf8_target_keeps_its_uri_form() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/raw/(.*)$".into(),
            replacement: "/mirror/$1".into(),
            flag: RewriteFlagConfig::Last,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(&compiled, request("/raw/i%FFndex"));
        assert_eq!(
            out,
            RewriteApplyOutcome::Internal {
                path: "/mirror/i%FFndex".into(),
                rematch: true,
            }
        );
    }

    /// A `when.path` condition matches a decoded non-UTF-8 target as nginx does:
    /// both the condition and the rule see `/raw/i\xFFndex`, not `%FF`.
    #[test]
    fn non_utf8_target_matches_a_decoded_when_path() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/raw/(i.ndex)$".into(),
            replacement: "/mirror/$1".into(),
            flag: RewriteFlagConfig::Last,
            when: vec![RewriteCondConfig {
                path: Some(r"^/raw/i.ndex$".into()),
                host: None,
                query: None,
                header: None,
                header_value: None,
            }],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(&compiled, request("/raw/i%FFndex"));
        assert_eq!(
            out,
            RewriteApplyOutcome::Internal {
                path: "/mirror/i%FFndex".into(),
                rematch: true,
            }
        );
    }

    /// A `when.path` that does not match the decoded bytes still vetoes the rule.
    #[test]
    fn non_utf8_target_when_path_still_vetoes() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/raw/(i.ndex)$".into(),
            replacement: "/mirror/$1".into(),
            flag: RewriteFlagConfig::Last,
            when: vec![RewriteCondConfig {
                path: Some(r"^/other/".into()),
                host: None,
                query: None,
                header: None,
                header_value: None,
            }],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        assert_eq!(
            apply_compiled_rewrite(&compiled, request("/raw/i%FFndex")),
            RewriteApplyOutcome::NoChange
        );
    }

    /// `$uri` in the byte branch is the decoded path, escaped once on the way out.
    #[test]
    fn non_utf8_dollar_uri_is_decoded_and_escaped_once() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/raw/i.ndex$".into(),
            replacement: "/mirror$uri".into(),
            flag: RewriteFlagConfig::Last,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(&compiled, request("/raw/i%FFndex"));
        assert_eq!(
            out,
            RewriteApplyOutcome::Internal {
                path: "/mirror/raw/i%FFndex".into(),
                rematch: true,
            }
        );
    }

    #[test]
    fn capture_redirect_permanent() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/old/(.*)$".into(),
            replacement: "/new/$1".into(),
            flag: RewriteFlagConfig::Permanent,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(
            &compiled,
            RewriteRequest {
                path: "/old/x",
                host: None,
                query: "",
                headers: &[],
            },
        );
        assert_eq!(
            out,
            RewriteApplyOutcome::Redirect {
                status: 301,
                location: "/new/x".into(),
            }
        );
    }

    #[test]
    fn permanent_redirect_preserves_original_query() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/(.*)$".into(),
            replacement: "https://example.com/$1".into(),
            flag: RewriteFlagConfig::Permanent,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(
            &compiled,
            RewriteRequest {
                path: "/old",
                host: Some("www.example.com"),
                query: "utm_source=x&a=1",
                headers: &[],
            },
        );
        assert_eq!(
            out,
            RewriteApplyOutcome::Redirect {
                status: 301,
                location: "https://example.com/old?utm_source=x&a=1".into(),
            }
        );
    }

    #[test]
    fn redirect_replacement_with_query_keeps_its_query() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/old$".into(),
            replacement: "/new?fixed=1".into(),
            flag: RewriteFlagConfig::Redirect,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(
            &compiled,
            RewriteRequest {
                path: "/old",
                host: None,
                query: "ignored=1",
                headers: &[],
            },
        );
        assert_eq!(
            out,
            RewriteApplyOutcome::Redirect {
                status: 302,
                location: "/new?fixed=1".into(),
            }
        );
    }

    #[test]
    fn when_host_gates_rule() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/x$".into(),
            replacement: "/y".into(),
            flag: RewriteFlagConfig::Break,
            when: vec![RewriteCondConfig {
                host: Some("app.example".into()),
                ..Default::default()
            }],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        assert_eq!(
            apply_compiled_rewrite(
                &compiled,
                RewriteRequest {
                    path: "/x",
                    host: Some("other.example"),
                    query: "",
                    headers: &[],
                },
            ),
            RewriteApplyOutcome::NoChange
        );
        assert_eq!(
            apply_compiled_rewrite(
                &compiled,
                RewriteRequest {
                    path: "/x",
                    host: Some("app.example"),
                    query: "",
                    headers: &[],
                },
            ),
            RewriteApplyOutcome::Internal {
                path: "/y".into(),
                rematch: false,
            }
        );
    }

    #[test]
    fn uri_and_host_vars_in_replacement() {
        let cfg = RewriteConfig::Rule(RewriteRuleConfig {
            pattern: r"^/mirror$".into(),
            replacement: "/echo/$host$uri".into(),
            flag: RewriteFlagConfig::Last,
            when: vec![],
        });
        let compiled = compile_rewrite(&cfg).expect("compile");
        let out = apply_compiled_rewrite(
            &compiled,
            RewriteRequest {
                path: "/mirror",
                host: Some("App.Example:8443"),
                query: "x=1",
                headers: &[],
            },
        );
        assert_eq!(
            out,
            RewriteApplyOutcome::Internal {
                path: "/echo/app.example/mirror".into(),
                rematch: true,
            }
        );
    }
}
