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

use exyonq_config_ir::{RewriteCondConfig, RewriteConfig, RewriteFlagConfig, RewriteRuleConfig};
use regex::Regex;

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
        replacement: String,
        flag: RewriteFlagConfig,
        when: Vec<CompiledWhen>,
    },
}

#[derive(Debug, Clone)]
pub enum CompiledWhen {
    Path(Regex),
    Host(String),
    Query(Regex),
    Header { name: String, value: Option<String> },
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
    Ok(CompiledRewrite::Rule {
        re,
        replacement: rule.replacement.clone(),
        flag: rule.flag,
        when,
    })
}

fn compile_when(cond: &RewriteCondConfig) -> Result<CompiledWhen, CompileRewriteError> {
    if let Some(path) = &cond.path {
        let re =
            Regex::new(path).map_err(|e| CompileRewriteError::Regex(format!("when.path: {e}")))?;
        return Ok(CompiledWhen::Path(re));
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
            replacement,
            flag,
            when,
        } => {
            if !when_match(when, req) {
                return RewriteApplyOutcome::NoChange;
            }
            let Some(caps) = re.captures(req.path) else {
                return RewriteApplyOutcome::NoChange;
            };
            let Ok(expanded) = expand_replacement(replacement, &caps, req) else {
                return RewriteApplyOutcome::Rejected;
            };
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
                    if location_valid(&expanded) {
                        RewriteApplyOutcome::Redirect {
                            status: 302,
                            location: expanded,
                        }
                    } else {
                        RewriteApplyOutcome::Rejected
                    }
                }
                RewriteFlagConfig::Permanent => {
                    if location_valid(&expanded) {
                        RewriteApplyOutcome::Redirect {
                            status: 301,
                            location: expanded,
                        }
                    } else {
                        RewriteApplyOutcome::Rejected
                    }
                }
            }
        }
    }
}

fn when_match(when: &[CompiledWhen], req: RewriteRequest<'_>) -> bool {
    when.iter().all(|w| match w {
        CompiledWhen::Path(re) => re.is_match(req.path),
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
