//! Structured `try_files` parsing and classification.

use crate::limits::MAX_TRY_FILES_CANDIDATES;
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};
use crate::variables::report_variables;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryFilesCandidate {
    Uri,
    UriDirectory,
    LiteralPath(String),
    VariableExpression(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryFilesFallback {
    Status(u16),
    Uri(String),
    NamedLocation(String),
    VariableExpression(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryFiles {
    pub candidates: Vec<TryFilesCandidate>,
    pub fallback: Option<TryFilesFallback>,
    pub loc: SourceLoc,
}

pub fn parse_try_files(loc: &SourceLoc, args: &[String]) -> Result<TryFiles, String> {
    if args.len() < 2 {
        return Err("try_files requires at least two arguments".into());
    }
    if args.len() > MAX_TRY_FILES_CANDIDATES {
        return Err(format!(
            "try_files candidate count exceeds limit ({MAX_TRY_FILES_CANDIDATES})"
        ));
    }
    let mut candidates = Vec::new();
    let mut fallback = None;
    for (i, arg) in args.iter().enumerate() {
        let is_last = i == args.len() - 1;
        if is_last {
            fallback = Some(parse_fallback(arg)?);
        } else {
            candidates.push(parse_candidate(arg)?);
        }
    }
    Ok(TryFiles {
        candidates,
        fallback,
        loc: loc.clone(),
    })
}

fn parse_candidate(arg: &str) -> Result<TryFilesCandidate, String> {
    match arg {
        "$uri" => Ok(TryFilesCandidate::Uri),
        "$uri/" => Ok(TryFilesCandidate::UriDirectory),
        s if s.starts_with('$') => Ok(TryFilesCandidate::VariableExpression(s.to_string())),
        s if s.starts_with('/') => Ok(TryFilesCandidate::LiteralPath(s.to_string())),
        s if s.starts_with('=') => {
            let code = s
                .trim_start_matches('=')
                .parse::<u16>()
                .map_err(|_| format!("invalid try_files status fallback `{s}`"))?;
            Ok(TryFilesCandidate::VariableExpression(format!("={code}")))
        }
        other => Err(format!("unrecognized try_files candidate `{other}`")),
    }
}

fn parse_fallback(arg: &str) -> Result<TryFilesFallback, String> {
    if let Some(code) = arg.strip_prefix('=') {
        let status = code
            .parse::<u16>()
            .map_err(|_| format!("invalid try_files status `{arg}`"))?;
        return Ok(TryFilesFallback::Status(status));
    }
    if arg.starts_with('@') {
        return Ok(TryFilesFallback::NamedLocation(arg.to_string()));
    }
    if arg.starts_with('$') {
        return Ok(TryFilesFallback::VariableExpression(arg.to_string()));
    }
    if arg.starts_with('/') {
        return Ok(TryFilesFallback::Uri(arg.to_string()));
    }
    Err(format!("invalid try_files fallback `{arg}`"))
}

pub fn semantic_form(tf: &TryFiles) -> String {
    let mut parts: Vec<String> = tf
        .candidates
        .iter()
        .map(|c| match c {
            TryFilesCandidate::Uri => "$uri".into(),
            TryFilesCandidate::UriDirectory => "$uri/".into(),
            TryFilesCandidate::LiteralPath(p) => p.clone(),
            TryFilesCandidate::VariableExpression(v) => v.clone(),
        })
        .collect();
    if let Some(fb) = &tf.fallback {
        parts.push(match fb {
            TryFilesFallback::Status(s) => format!("={s}"),
            TryFilesFallback::Uri(u) => u.clone(),
            TryFilesFallback::NamedLocation(n) => n.clone(),
            TryFilesFallback::VariableExpression(v) => v.clone(),
        });
    }
    parts.join(" ")
}

pub fn analyze_try_files(
    tf: &TryFiles,
    named_locations: &HashSet<String>,
    report: &mut CompatibilityReport,
) {
    let raw = semantic_form(tf);
    report_variables(&tf.loc, "try_files", &raw, report);
    report.summary.try_files_directives += 1;

    let (status, message, action) = classify_try_files(tf, named_locations);
    if status == CompatStatus::Error && message.contains("unknown named location") {
        report.summary.unresolved_named_locations += 1;
    }
    report.push_migration(
        &tf.loc,
        "try_files",
        status,
        message,
        None,
        Some(action),
        None,
        Some(semantic_form(tf)),
        None,
    );
}

fn classify_try_files(
    tf: &TryFiles,
    named_locations: &HashSet<String>,
) -> (CompatStatus, String, String) {
    if let Some(TryFilesFallback::NamedLocation(name)) = &tf.fallback {
        if !named_locations.contains(name) {
            return (
                CompatStatus::Error,
                format!("try_files references unknown named location `{name}`"),
                "Define named location or replace fallback".into(),
            );
        }
        return (
            CompatStatus::Unsupported,
            format!("try_files named fallback `{name}` requires named-location dispatch"),
            "Requires named-location dispatch".into(),
        );
    }

    let has_uri_dir = tf
        .candidates
        .iter()
        .any(|c| matches!(c, TryFilesCandidate::UriDirectory));
    let has_vars = tf
        .candidates
        .iter()
        .any(|c| matches!(c, TryFilesCandidate::VariableExpression(_)))
        || matches!(tf.fallback, Some(TryFilesFallback::VariableExpression(_)));

    if let Some(mapped) = map_try_files_to_fallback(tf) {
        let note = if mapped.try_uri_dir {
            "ADR-049 / NG-03: try_uri+try_uri_dir + fallback_rewrite (query preserved ≡ $args)"
        } else {
            "ADR-049 / NG-03: try_uri + fallback_rewrite"
        };
        return (
            CompatStatus::Supported,
            format!(
                "try_files → fallback_rewrite = `{}` ({note})",
                mapped.fallback_rewrite
            ),
            format!(
                "route.fallback_rewrite={}; try_uri={}; try_uri_dir={}",
                mapped.fallback_rewrite, mapped.try_uri, mapped.try_uri_dir
            ),
        );
    }

    if let Some(TryFilesFallback::Uri(uri)) = &tf.fallback {
        if uri.contains('?') || has_vars {
            return (
                CompatStatus::Unsupported,
                format!("try_files PHP front controller `{uri}` not representable in runtime"),
                "Requires query-string preservation and static→FastCGI internal redirect".into(),
            );
        }
        return (
            CompatStatus::Partial,
            format!("try_files internal URI fallback `{uri}`"),
            "Requires ADR-049 admitted form ($uri [/$uri/] + absolute path)".into(),
        );
    }

    if let Some(TryFilesFallback::Status(404)) = tf.fallback {
        if tf.candidates == [TryFilesCandidate::Uri]
            || (tf.candidates.len() == 2
                && tf.candidates[0] == TryFilesCandidate::Uri
                && tf.candidates[1] == TryFilesCandidate::UriDirectory)
        {
            return (
                CompatStatus::Partial,
                "try_files static lookup → =404 (no dedicated IR action)".into(),
                "Map manually to static file serving with 404 fallback".into(),
            );
        }
    }

    if has_uri_dir || has_vars {
        return (
            CompatStatus::Partial,
            format!(
                "try_files `{raw}` needs runtime try_files semantics",
                raw = semantic_form(tf)
            ),
            "Requires internal fallback route support".into(),
        );
    }

    (
        CompatStatus::Unsupported,
        format!("try_files `{}` not mapped to IR", semantic_form(tf)),
        "Requires ADR-049 admitted form ($uri [/$uri/] + absolute path)".into(),
    )
}

/// NG-03 / ADR-049 admitted try_files → IR fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedTryFiles {
    pub try_uri: bool,
    pub try_uri_dir: bool,
    pub fallback_rewrite: String,
}

/// Map SPA / WordPress try_files forms onto `fallback_rewrite`.
pub fn map_try_files_to_fallback(tf: &TryFiles) -> Option<MappedTryFiles> {
    let fallback_raw = match &tf.fallback {
        Some(TryFilesFallback::Uri(u)) => u.as_str(),
        Some(TryFilesFallback::VariableExpression(v)) => v.as_str(),
        _ => return None,
    };
    let fallback_rewrite = normalize_fallback_path(fallback_raw)?;

    let only_uri = tf.candidates == [TryFilesCandidate::Uri];
    let uri_and_dir = tf.candidates.len() == 2
        && tf.candidates[0] == TryFilesCandidate::Uri
        && tf.candidates[1] == TryFilesCandidate::UriDirectory;

    if only_uri {
        return Some(MappedTryFiles {
            try_uri: true,
            try_uri_dir: false,
            fallback_rewrite,
        });
    }
    if uri_and_dir {
        return Some(MappedTryFiles {
            try_uri: true,
            try_uri_dir: true,
            fallback_rewrite,
        });
    }
    None
}

/// Strip nginx query / `$is_args$args` sugar; keep Cap035 absolute path.
fn normalize_fallback_path(raw: &str) -> Option<String> {
    let mut path = raw;
    if let Some(idx) = path.find('?') {
        let q = &path[idx + 1..];
        // Admitted: ?$args / ?$query_string meaning "preserve original query".
        if q == "$args" || q == "$query_string" || q.is_empty() {
            path = &path[..idx];
        } else {
            return None;
        }
    }
    if let Some(stripped) = path.strip_suffix("$is_args$args") {
        path = stripped;
    }
    if path.contains('$') {
        return None;
    }
    if !path.starts_with('/') || path.starts_with("//") || path.contains('#') {
        return None;
    }
    Some(path.to_string())
}
