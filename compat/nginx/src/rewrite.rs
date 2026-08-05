//! Structured `rewrite` parsing and classification.

use crate::limits::{MAX_REGEX_LENGTH, MAX_REWRITE_RULES_PER_LOCATION};
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};
use crate::variables::report_variables;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteFlag {
    Last,
    Break,
    Redirect,
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteRule {
    pub pattern: String,
    pub replacement: String,
    pub flag: Option<RewriteFlag>,
    pub loc: SourceLoc,
}

pub fn parse_rewrite(loc: &SourceLoc, args: &[String]) -> Result<RewriteRule, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("rewrite requires pattern and replacement (optional flag)".into());
    }
    let pattern = &args[0];
    if pattern.len() > MAX_REGEX_LENGTH {
        return Err(format!(
            "rewrite pattern exceeds max length ({MAX_REGEX_LENGTH})"
        ));
    }
    let replacement = args[1].clone();
    let flag = args.get(2).map(|f| parse_flag(f)).transpose()?;
    Ok(RewriteRule {
        pattern: pattern.clone(),
        replacement,
        flag,
        loc: loc.clone(),
    })
}

fn parse_flag(raw: &str) -> Result<RewriteFlag, String> {
    match raw {
        "last" => Ok(RewriteFlag::Last),
        "break" => Ok(RewriteFlag::Break),
        "redirect" => Ok(RewriteFlag::Redirect),
        "permanent" => Ok(RewriteFlag::Permanent),
        other => Err(format!("unknown rewrite flag `{other}`")),
    }
}

pub fn semantic_form(rule: &RewriteRule) -> String {
    match rule.flag {
        Some(f) => format!("{} {} {}", rule.pattern, rule.replacement, flag_name(f)),
        None => format!("{} {}", rule.pattern, rule.replacement),
    }
}

fn flag_name(flag: RewriteFlag) -> &'static str {
    match flag {
        RewriteFlag::Last => "last",
        RewriteFlag::Break => "break",
        RewriteFlag::Redirect => "redirect",
        RewriteFlag::Permanent => "permanent",
    }
}

/// Literal exact path pattern: `^/foo$` with no capture groups.
pub fn is_safe_literal_redirect(rule: &RewriteRule) -> Option<(u16, String, String)> {
    let flag = rule.flag?;
    let status = match flag {
        RewriteFlag::Permanent => 301,
        RewriteFlag::Redirect => 302,
        RewriteFlag::Last | RewriteFlag::Break => return None,
    };
    if !is_literal_path_pattern(&rule.pattern) {
        return None;
    }
    if rule.replacement.contains('$') {
        return None;
    }
    if !rule.replacement.starts_with('/') {
        return None;
    }
    let from = literal_from_pattern(&rule.pattern)?;
    Some((status, from, rule.replacement.clone()))
}

fn is_literal_path_pattern(pattern: &str) -> bool {
    pattern.starts_with('^')
        && pattern.ends_with('$')
        && !pattern.contains('(')
        && pattern.len() >= 3
}

fn literal_from_pattern(pattern: &str) -> Option<String> {
    let inner = pattern.strip_prefix('^')?.strip_suffix('$')?;
    if inner.is_empty() || !inner.starts_with('/') {
        return None;
    }
    if inner.contains('*') || inner.contains('?') {
        return None;
    }
    Some(inner.to_string())
}

pub fn analyze_rewrite(
    rule: &RewriteRule,
    report: &mut CompatibilityReport,
) -> Option<(u16, String)> {
    report.summary.rewrite_rules += 1;
    let raw = semantic_form(rule);
    report_variables(&rule.loc, "rewrite", &raw, report);

    if let Some((status, from, to)) = is_safe_literal_redirect(rule) {
        report.summary.safe_redirect_rewrites += 1;
        report.push_migration(
            &rule.loc,
            "rewrite",
            CompatStatus::Supported,
            format!("rewrite `{from}` → `{to}` maps to HTTP {status} redirect"),
            Some(format!("route.redirect status={status} location={to}")),
            Some("Safe to replace with HTTP redirect".into()),
            None,
            Some(raw),
            None,
        );
        return Some((status, to));
    }

    let (status, message, action) = classify_rewrite(rule);
    if matches!(
        rule.flag,
        Some(RewriteFlag::Last) | Some(RewriteFlag::Break)
    ) {
        report.summary.internal_rewrites_unsupported += 1;
    }
    report.push_migration(
        &rule.loc,
        "rewrite",
        status,
        message,
        None,
        Some(action),
        None,
        Some(raw),
        None,
    );
    None
}

fn classify_rewrite(rule: &RewriteRule) -> (CompatStatus, String, String) {
    if rule.pattern.contains('(') || rule.replacement.contains('$') {
        return (
            CompatStatus::Unsupported,
            format!(
                "rewrite `{}` uses regex captures or variables",
                semantic_form(rule)
            ),
            "Requires regex rewrite engine".into(),
        );
    }
    match rule.flag {
        Some(RewriteFlag::Last) => (
            CompatStatus::Unsupported,
            format!(
                "rewrite `{}` with flag last re-selects location",
                semantic_form(rule)
            ),
            "Requires regex rewrite engine (last → new location search)".into(),
        ),
        Some(RewriteFlag::Break) => (
            CompatStatus::Unsupported,
            format!(
                "rewrite `{}` with flag break stops in current location",
                semantic_form(rule)
            ),
            "Requires regex rewrite engine (break → stay in location)".into(),
        ),
        Some(RewriteFlag::Permanent) | Some(RewriteFlag::Redirect) => (
            CompatStatus::Partial,
            format!(
                "rewrite `{}` redirect not literal-safe for automatic mapping",
                semantic_form(rule)
            ),
            "Requires regex rewrite engine".into(),
        ),
        None => (
            CompatStatus::Unsupported,
            format!(
                "rewrite `{}` implies internal URI rewrite",
                semantic_form(rule)
            ),
            "Requires regex rewrite engine".into(),
        ),
    }
}

pub fn analyze_rewrites_in_location(
    rules: &[RewriteRule],
    report: &mut CompatibilityReport,
) -> Vec<(u16, String)> {
    if rules.len() > MAX_REWRITE_RULES_PER_LOCATION {
        if let Some(first) = rules.first() {
            report.push(
                &first.loc,
                "rewrite",
                CompatStatus::Error,
                format!("rewrite rule count exceeds limit ({MAX_REWRITE_RULES_PER_LOCATION})"),
                None,
            );
        }
        return Vec::new();
    }
    rules
        .iter()
        .filter_map(|r| analyze_rewrite(r, report))
        .collect()
}
