//! NGINX location parsing and offline precedence analysis.

use crate::ast::{LocationKind, ParsedLocation, ParsedServer};
use crate::limits::MAX_LOCATIONS_PER_SERVER;
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};
use crate::rewrite::{analyze_rewrites_in_location, parse_rewrite};
use crate::try_files::{analyze_try_files, parse_try_files};
use std::collections::HashSet;

pub fn parse_location_directive(
    directive: &crate::ast::Directive,
    order: usize,
    report: &mut CompatibilityReport,
) -> Option<ParsedLocation> {
    if order >= MAX_LOCATIONS_PER_SERVER {
        report.push(
            &directive.loc,
            "location",
            CompatStatus::Error,
            format!("location count exceeds limit ({MAX_LOCATIONS_PER_SERVER})"),
            None,
        );
        return None;
    }

    let mut args: Vec<&str> = directive.args.iter().map(String::as_str).collect();
    if args.is_empty() {
        report.push(
            &directive.loc,
            "location",
            CompatStatus::Error,
            "location path missing",
            None,
        );
        return None;
    }

    let (kind, raw_pattern, normalized) = parse_location_kind(&mut args, &directive.loc, report)?;

    let mut loc_directives = Vec::new();
    if let Some(block) = &directive.block {
        for child in &block.directives {
            loc_directives.push(child.clone());
        }
    }

    count_location_kind(kind, report);

    let loc = ParsedLocation {
        kind,
        raw_pattern: raw_pattern.clone(),
        normalized_literal_path: normalized,
        order,
        directives: loc_directives,
        loc: directive.loc.clone(),
    };

    report.push_migration(
        &directive.loc,
        "location",
        location_status(kind),
        location_message(kind, &raw_pattern),
        mapping_hint(kind, &raw_pattern),
        None,
        Some(kind.as_str()),
        Some(format!("{} `{raw_pattern}`", kind.as_str())),
        None,
    );

    Some(loc)
}

fn parse_location_kind(
    args: &mut Vec<&str>,
    loc: &SourceLoc,
    report: &mut CompatibilityReport,
) -> Option<(LocationKind, String, Option<String>)> {
    let first = args.remove(0);
    match first {
        "=" => {
            let path = args.first().copied().unwrap_or("/").to_string();
            Some((
                LocationKind::Exact,
                path.clone(),
                Some(normalize_literal_path(&path)),
            ))
        }
        "^~" => {
            let path = args.first().copied().unwrap_or("/").to_string();
            Some((
                LocationKind::PreferentialPrefix,
                path.clone(),
                Some(normalize_literal_path(&path)),
            ))
        }
        "~" => {
            let pattern = args.first().copied().unwrap_or("").to_string();
            if pattern.len() > crate::limits::MAX_REGEX_LENGTH {
                report.push(
                    loc,
                    "location",
                    CompatStatus::Error,
                    format!(
                        "regex pattern exceeds max length ({})",
                        crate::limits::MAX_REGEX_LENGTH
                    ),
                    None,
                );
                return None;
            }
            Some((LocationKind::RegexCaseSensitive, pattern, None))
        }
        "~*" => {
            let pattern = args.first().copied().unwrap_or("").to_string();
            if pattern.len() > crate::limits::MAX_REGEX_LENGTH {
                report.push(
                    loc,
                    "location",
                    CompatStatus::Error,
                    format!(
                        "regex pattern exceeds max length ({})",
                        crate::limits::MAX_REGEX_LENGTH
                    ),
                    None,
                );
                return None;
            }
            Some((LocationKind::RegexCaseInsensitive, pattern, None))
        }
        path if path.starts_with('@') => Some((LocationKind::Named, path.to_string(), None)),
        path => {
            if path.starts_with('~') {
                report.push(
                    loc,
                    "location",
                    CompatStatus::Error,
                    format!("invalid location modifier sequence `{path}`"),
                    None,
                );
                return None;
            }
            Some((
                LocationKind::Prefix,
                path.to_string(),
                Some(normalize_literal_path(path)),
            ))
        }
    }
}

fn normalize_literal_path(path: &str) -> String {
    if path.is_empty() {
        return "/".into();
    }
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn count_location_kind(kind: LocationKind, report: &mut CompatibilityReport) {
    match kind {
        LocationKind::Exact => report.summary.exact_locations += 1,
        LocationKind::Prefix => report.summary.prefix_locations += 1,
        LocationKind::PreferentialPrefix => report.summary.preferential_prefixes += 1,
        LocationKind::RegexCaseSensitive | LocationKind::RegexCaseInsensitive => {
            report.summary.regex_locations += 1;
        }
        LocationKind::Named => report.summary.named_locations += 1,
    }
}

fn location_status(kind: LocationKind) -> CompatStatus {
    match kind {
        LocationKind::Prefix => CompatStatus::Supported,
        LocationKind::Exact | LocationKind::PreferentialPrefix => CompatStatus::Partial,
        LocationKind::RegexCaseSensitive
        | LocationKind::RegexCaseInsensitive
        | LocationKind::Named => CompatStatus::Unsupported,
    }
}

fn location_message(kind: LocationKind, pattern: &str) -> String {
    match kind {
        LocationKind::Prefix => format!("prefix location `{pattern}` accepted"),
        LocationKind::Exact => {
            format!("exact location `{pattern}` parsed; IR has no exact/prefix distinction")
        }
        LocationKind::PreferentialPrefix => {
            format!("preferential prefix `^~ {pattern}` parsed; ExyonQ does not model ^~ priority")
        }
        LocationKind::RegexCaseSensitive => {
            format!("regex location `~ {pattern}` parsed; not mapped to runtime route")
        }
        LocationKind::RegexCaseInsensitive => {
            format!("regex location `~* {pattern}` parsed; not mapped to runtime route")
        }
        LocationKind::Named => {
            format!("named location `{pattern}` parsed; internal dispatch unsupported")
        }
    }
}

fn mapping_hint(kind: LocationKind, pattern: &str) -> Option<String> {
    match kind {
        LocationKind::Prefix => Some(format!("route match.path={pattern}")),
        LocationKind::Exact | LocationKind::PreferentialPrefix => {
            Some(format!("route match.path={pattern} (literal only)"))
        }
        _ => None,
    }
}

pub fn named_location_set(server: &ParsedServer) -> HashSet<String> {
    server
        .locations
        .iter()
        .filter(|l| l.kind == LocationKind::Named)
        .map(|l| l.raw_pattern.clone())
        .collect()
}

pub fn analyze_location_directives(
    location: &ParsedLocation,
    named: &HashSet<String>,
    report: &mut CompatibilityReport,
) {
    let mut rewrites = Vec::new();
    for d in &location.directives {
        match d.name.as_str() {
            "try_files" => match parse_try_files(&d.loc, &d.args) {
                Ok(tf) => analyze_try_files(&tf, named, report),
                Err(msg) => report.push(&d.loc, "try_files", CompatStatus::Error, msg, None),
            },
            "rewrite" => match parse_rewrite(&d.loc, &d.args) {
                Ok(rule) => rewrites.push(rule),
                Err(msg) => report.push(&d.loc, "rewrite", CompatStatus::Error, msg, None),
            },
            _ => {}
        }
    }
    let _redirects = analyze_rewrites_in_location(&rewrites, report);
}

pub fn analyze_precedence(server: &ParsedServer, report: &mut CompatibilityReport) {
    let mut seen_exact: HashSet<String> = HashSet::new();
    let mut seen_prefix: HashSet<String> = HashSet::new();
    let mut seen_regex: HashSet<String> = HashSet::new();
    let mut seen_named: HashSet<String> = HashSet::new();
    let mut literal_paths: Vec<(LocationKind, String, usize)> = Vec::new();

    for loc in &server.locations {
        match loc.kind {
            LocationKind::Exact => {
                let path = loc.normalized_literal_path.clone().unwrap_or_default();
                if !seen_exact.insert(path.clone()) {
                    report.push(
                        &loc.loc,
                        "location",
                        CompatStatus::Error,
                        format!("duplicate exact location `{path}`"),
                        None,
                    );
                }
                literal_paths.push((loc.kind, path, loc.order));
            }
            LocationKind::Prefix | LocationKind::PreferentialPrefix => {
                let path = loc.normalized_literal_path.clone().unwrap_or_default();
                if !seen_prefix.insert(path.clone()) {
                    report.push(
                        &loc.loc,
                        "location",
                        CompatStatus::Error,
                        format!("duplicate prefix location `{path}`"),
                        None,
                    );
                }
                literal_paths.push((loc.kind, path, loc.order));
            }
            LocationKind::RegexCaseSensitive | LocationKind::RegexCaseInsensitive => {
                if !seen_regex.insert(loc.raw_pattern.clone()) {
                    report.push(
                        &loc.loc,
                        "location",
                        CompatStatus::Error,
                        format!("duplicate regex location `{}`", loc.raw_pattern),
                        None,
                    );
                }
            }
            LocationKind::Named => {
                if !seen_named.insert(loc.raw_pattern.clone()) {
                    report.push(
                        &loc.loc,
                        "location",
                        CompatStatus::Error,
                        format!("duplicate named location `{}`", loc.raw_pattern),
                        None,
                    );
                }
            }
        }
    }

    for i in 0..literal_paths.len() {
        for j in (i + 1)..literal_paths.len() {
            let (ki, pi, oi) = &literal_paths[i];
            let (kj, pj, oj) = &literal_paths[j];
            if pi == pj && ki != kj {
                report.summary.location_precedence_risks += 1;
                report.push_migration(
                    &server.loc,
                    "location",
                    CompatStatus::Partial,
                    format!(
                        "exact and prefix locations share path `{pi}` (orders {oi},{oj}); NGINX exact wins, ExyonQ IR may differ"
                    ),
                    None,
                    Some("Review location precedence after migration".into()),
                    None,
                    None,
                    None,
                );
            }
            if pi.starts_with(pj.as_str()) && pi != pj && *ki == LocationKind::Prefix {
                report.summary.location_precedence_risks += 1;
                report.push_migration(
                    &server.loc,
                    "location",
                    CompatStatus::Partial,
                    format!(
                        "prefix `{pi}` is more specific than `{pj}`; order may change in ExyonQ longest-prefix routing"
                    ),
                    None,
                    Some("Review location precedence after migration".into()),
                    None,
                    None,
                    None,
                );
            }
            let _ = (kj, oj);
        }
    }

    if server
        .locations
        .iter()
        .any(|l| l.kind == LocationKind::PreferentialPrefix)
        && server.locations.iter().any(|l| {
            matches!(
                l.kind,
                LocationKind::RegexCaseSensitive | LocationKind::RegexCaseInsensitive
            )
        })
    {
        report.summary.location_precedence_risks += 1;
        if let Some(loc) = server
            .locations
            .iter()
            .find(|l| l.kind == LocationKind::PreferentialPrefix)
        {
            report.push_migration(
                &loc.loc,
                "location",
                CompatStatus::Partial,
                "^~ preferential prefix present with regex locations; NGINX ^~ stops regex search",
                None,
                Some("Review location precedence after migration".into()),
                Some("PreferentialPrefix"),
                None,
                None,
            );
        }
    }

    let has_regex = server.locations.iter().any(|l| {
        matches!(
            l.kind,
            LocationKind::RegexCaseSensitive | LocationKind::RegexCaseInsensitive
        )
    });
    if has_regex {
        for (kind, path, _) in &literal_paths {
            if *kind == LocationKind::Prefix {
                report.summary.location_precedence_risks += 1;
                report.push_migration(
                    &server.loc,
                    "location",
                    CompatStatus::Partial,
                    format!(
                        "regex locations may intercept URIs under prefix `{path}` in NGINX; ExyonQ regex not active"
                    ),
                    None,
                    Some("Review location precedence after migration".into()),
                    None,
                    None,
                    None,
                );
            }
        }
    }
}

pub fn is_runtime_mappable(kind: LocationKind) -> bool {
    matches!(
        kind,
        LocationKind::Prefix | LocationKind::Exact | LocationKind::PreferentialPrefix
    )
}

pub fn route_path(location: &ParsedLocation) -> String {
    location
        .normalized_literal_path
        .clone()
        .unwrap_or_else(|| location.raw_pattern.clone())
}
