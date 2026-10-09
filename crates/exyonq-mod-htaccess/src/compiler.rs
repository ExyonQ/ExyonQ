//! Offline semantic analysis and compilation to [`VhostOverlay`].

use crate::ast::{DirectiveSupport, ParsedDirective, ParsedFile};
use crate::discovery::{discover_htaccess_files, DiscoveredFile, DiscoveryError};
use crate::parser::{parse_htaccess, ParseError};
use crate::report::CompileReport;
use exyonq_module_api::{
    validate_directory_index_candidate, validate_front_controller_target, CompiledFrontController,
    CompiledRedirectRule, CompiledRewriteRedirect, NormalizedDirectory, OverlayEntry, VhostOverlay,
    MAX_DIRECTORY_INDEX_CANDIDATES, MAX_DIRECTORY_INDEX_CANDIDATE_LEN,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("discovery failed: {0}")]
    Discovery(#[from] DiscoveryError),
    #[error("parse failed: {0}")]
    Parse(#[from] ParseError),
    #[error("compile failed: {0}")]
    Message(String),
}

#[derive(Debug, Clone, Default)]
struct PendingRewriteConds {
    require_not_file: bool,
    require_not_directory: bool,
    poisoned: bool,
}

#[derive(Debug, Clone, Default)]
struct DirState {
    directory_index: Option<Vec<String>>,
    redirects: Vec<CompiledRedirectRule>,
    rewrite_redirects: Vec<CompiledRewriteRedirect>,
    indexes_disabled: bool,
    rewrite_engine: bool,
    pending_rewrite_conds: PendingRewriteConds,
    front_controller: Option<CompiledFrontController>,
}

pub struct CompileOutput {
    pub overlay: VhostOverlay,
    pub report: CompileReport,
}

pub fn compile_vhost_overlay(
    site_id: &str,
    document_root: &Path,
    generation: u64,
) -> Result<CompileOutput, CompileError> {
    let discovered = discover_htaccess_files(document_root)?;
    compile_from_discovered(site_id, document_root, generation, discovered)
}

pub fn compile_from_discovered(
    site_id: &str,
    document_root: &Path,
    generation: u64,
    discovered: Vec<DiscoveredFile>,
) -> Result<CompileOutput, CompileError> {
    let mut report = CompileReport::default();
    let mut per_dir: BTreeMap<String, DirState> = BTreeMap::new();

    for file in discovered {
        let parsed = parse_htaccess(&file.absolute.display().to_string(), &file.content)?;
        let dir_key = normalize_directory_key(&file.relative_directory);
        let state = per_dir.entry(dir_key).or_default();
        apply_file(state, &parsed, &mut report);
    }

    let merged = merge_inheritance(&per_dir);
    let mut entries: Vec<OverlayEntry> = merged
        .into_iter()
        .map(|(directory, state)| OverlayEntry {
            directory: NormalizedDirectory(directory),
            directory_index: directory_index_or_front_controller_default(&state)
                .map(|v| Arc::<[String]>::from(v.into_boxed_slice())),
            redirect_rules: Arc::from(state.redirects.into_boxed_slice()),
            rewrite_redirects: Arc::from(state.rewrite_redirects.into_boxed_slice()),
            indexes_disabled: state.indexes_disabled,
            front_controller: state.front_controller,
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.directory.0.len()));

    report.executed = report
        .diagnostics
        .iter()
        .filter(|d| d.starts_with("executed:"))
        .count();
    report.parsed_only = report
        .diagnostics
        .iter()
        .filter(|d| d.starts_with("parsed:"))
        .count();
    report.unknown = report
        .diagnostics
        .iter()
        .filter(|d| d.starts_with("unknown:"))
        .count();

    Ok(CompileOutput {
        overlay: VhostOverlay {
            generation,
            site_id: site_id.to_string(),
            document_root: document_root.display().to_string(),
            entries: Arc::from(entries.into_boxed_slice()),
        },
        report,
    })
}

fn normalize_directory_key(dir: &str) -> String {
    let mut s = dir.replace('\\', "/");
    if !s.starts_with('/') {
        s.insert(0, '/');
    }
    if s != "/" && !s.ends_with('/') {
        s.push('/');
    }
    s
}

fn apply_file(state: &mut DirState, file: &ParsedFile, report: &mut CompileReport) {
    for directive in &file.directives {
        match classify_and_apply(state, directive, report) {
            Ok(DirectiveSupport::Executed) => {
                report.push_diag(format!(
                    "executed:{}:{}:{}",
                    directive.loc.file, directive.loc.line, directive.name
                ));
            }
            Ok(DirectiveSupport::ParsedOnly) => {
                report.push_diag(format!(
                    "parsed:{}:{}:{}",
                    directive.loc.file, directive.loc.line, directive.name
                ));
            }
            Ok(DirectiveSupport::Unknown) => {
                report.push_diag(format!(
                    "unknown:{}:{}:{}",
                    directive.loc.file, directive.loc.line, directive.name
                ));
            }
            Err(msg) => report.errors.push(msg),
        }
    }
}

fn classify_and_apply(
    state: &mut DirState,
    d: &ParsedDirective,
    report: &mut CompileReport,
) -> Result<DirectiveSupport, String> {
    match d.name.as_str() {
        "directoryindex" => {
            if d.args.is_empty() {
                return Err(format!(
                    "{}:{} DirectoryIndex missing args",
                    d.loc.file, d.loc.line
                ));
            }
            let validated = validate_directory_index_list(&d.args)?;
            state.directory_index = Some(validated);
            Ok(DirectiveSupport::Executed)
        }
        "redirect" => {
            let (status, from, to) = parse_redirect_args(d)?;
            validate_literal_path(&from)?;
            validate_redirect_target(&to)?;
            state.redirects.push(CompiledRedirectRule {
                from_path: from,
                status,
                location: to,
            });
            Ok(DirectiveSupport::Executed)
        }
        "redirectmatch" => {
            let _ = d.args.len();
            report.push_diag(format!(
                "parsed:{}:{} RedirectMatch not executed in v0",
                d.loc.file, d.loc.line
            ));
            Ok(DirectiveSupport::ParsedOnly)
        }
        "rewriteengine" => {
            let mode = d.args.first().map(|s| s.to_ascii_lowercase());
            state.rewrite_engine = matches!(mode.as_deref(), Some(s) if s == "on");
            Ok(DirectiveSupport::Executed)
        }
        "rewritebase" => {
            report.push_diag(format!(
                "parsed:{}:{} RewriteBase not executed in v0",
                d.loc.file, d.loc.line
            ));
            Ok(DirectiveSupport::ParsedOnly)
        }
        "rewriterule" => {
            if !state.rewrite_engine {
                return Ok(DirectiveSupport::ParsedOnly);
            }
            if is_noop_rewrite_rule(d) {
                state.pending_rewrite_conds = PendingRewriteConds::default();
                return Ok(DirectiveSupport::ParsedOnly);
            }
            if let Some(fc) = compile_front_controller_rule(&state.pending_rewrite_conds, d)? {
                state.front_controller = Some(fc);
                state.pending_rewrite_conds = PendingRewriteConds::default();
                Ok(DirectiveSupport::Executed)
            } else if let Some(rule) = compile_safe_rewrite_redirect(d)? {
                state.pending_rewrite_conds = PendingRewriteConds::default();
                state.rewrite_redirects.push(rule);
                Ok(DirectiveSupport::Executed)
            } else {
                state.pending_rewrite_conds = PendingRewriteConds::default();
                Ok(DirectiveSupport::ParsedOnly)
            }
        }
        "rewritecond" => {
            if !state.rewrite_engine {
                return Ok(DirectiveSupport::ParsedOnly);
            }
            match parse_front_controller_cond(d) {
                Some(kind) => {
                    if !state.pending_rewrite_conds.poisoned {
                        match kind {
                            RewriteCondKind::NotFile => {
                                state.pending_rewrite_conds.require_not_file = true;
                            }
                            RewriteCondKind::NotDirectory => {
                                state.pending_rewrite_conds.require_not_directory = true;
                            }
                        }
                    }
                }
                None => {
                    state.pending_rewrite_conds = PendingRewriteConds {
                        poisoned: true,
                        ..PendingRewriteConds::default()
                    };
                    report.push_diag(format!(
                        "parsed:{}:{} RewriteCond unsupported for front controller",
                        d.loc.file, d.loc.line
                    ));
                }
            }
            Ok(DirectiveSupport::ParsedOnly)
        }
        "errordocument" => Ok(DirectiveSupport::ParsedOnly),
        "options" => {
            for arg in &d.args {
                if arg.eq_ignore_ascii_case("-indexes") {
                    state.indexes_disabled = true;
                } else if arg.eq_ignore_ascii_case("+indexes") {
                    return Err(format!(
                        "{}:{} Options +Indexes unsupported",
                        d.loc.file, d.loc.line
                    ));
                } else {
                    report.push_diag(format!(
                        "parsed:{}:{} Options `{}` not executed",
                        d.loc.file, d.loc.line, arg
                    ));
                }
            }
            Ok(DirectiveSupport::Executed)
        }
        other => {
            report.push_diag(format!("unknown:{}:{}:{}", d.loc.file, d.loc.line, other));
            Ok(DirectiveSupport::Unknown)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RewriteCondKind {
    NotFile,
    NotDirectory,
}

fn parse_front_controller_cond(d: &ParsedDirective) -> Option<RewriteCondKind> {
    if d.args.len() < 2 {
        return None;
    }
    let test = d.args[0].as_str();
    let negate = d.args[1].eq_ignore_ascii_case("!-f")
        || d.args[1].eq_ignore_ascii_case("!-d")
        || d.args[1].eq_ignore_ascii_case("!f")
        || d.args[1].eq_ignore_ascii_case("!d");
    if !negate {
        return None;
    }
    if test != "%{REQUEST_FILENAME}" {
        return None;
    }
    if d.args[1].eq_ignore_ascii_case("!-f") || d.args[1].eq_ignore_ascii_case("!f") {
        return Some(RewriteCondKind::NotFile);
    }
    if d.args[1].eq_ignore_ascii_case("!-d") || d.args[1].eq_ignore_ascii_case("!d") {
        return Some(RewriteCondKind::NotDirectory);
    }
    None
}

fn is_noop_rewrite_rule(d: &ParsedDirective) -> bool {
    d.args.len() >= 2 && (d.args[1] == "-" || d.args[1] == "/dev/null")
}

fn compile_front_controller_rule(
    pending: &PendingRewriteConds,
    d: &ParsedDirective,
) -> Result<Option<CompiledFrontController>, String> {
    if pending.poisoned {
        return Ok(None);
    }
    if !pending.require_not_file || !pending.require_not_directory {
        return Ok(None);
    }
    if d.args.len() < 2 {
        return Ok(None);
    }
    let pattern = d.args[0].as_str();
    let replacement = d.args[1].as_str();
    let flags = d.args.get(2).map(String::as_str).unwrap_or("");
    if !is_catch_all_rewrite_pattern(pattern) {
        return Ok(None);
    }
    if replacement.contains('$') || replacement.contains('%') {
        return Ok(None);
    }
    if !parse_internal_rewrite_flags(flags)? {
        return Ok(None);
    }
    let target = normalize_internal_rewrite_target(replacement)?;
    if !validate_front_controller_target(&target) {
        return Err(format!(
            "{}:{} invalid front-controller target `{target}`",
            d.loc.file, d.loc.line
        ));
    }
    Ok(Some(CompiledFrontController {
        target_uri: Arc::from(target),
        require_not_file: true,
        require_not_directory: true,
        preserve_query: true,
    }))
}

fn is_catch_all_rewrite_pattern(pattern: &str) -> bool {
    matches!(pattern, "." | "^" | "^.*$" | "^.*" | ".*")
}

fn normalize_internal_rewrite_target(replacement: &str) -> Result<String, String> {
    if replacement.starts_with("http://") || replacement.starts_with("https://") {
        return Err("external front-controller target unsupported".into());
    }
    let path = if replacement.starts_with('/') {
        replacement.to_string()
    } else {
        format!("/{replacement}")
    };
    Ok(path)
}

fn parse_internal_rewrite_flags(flags: &str) -> Result<bool, String> {
    let flags = flags.trim();
    let inner = flags
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(flags);
    if inner.is_empty() {
        return Ok(true);
    }
    for flag in inner.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if flag.eq_ignore_ascii_case("l") || flag.eq_ignore_ascii_case("last") {
            continue;
        }
        if flag.eq_ignore_ascii_case("nc") {
            continue;
        }
        if flag.starts_with('R') || flag.starts_with('r') {
            return Ok(false);
        }
        if flag.eq_ignore_ascii_case("qsa")
            || flag.eq_ignore_ascii_case("qsd")
            || flag.eq_ignore_ascii_case("qsl")
            || flag.eq_ignore_ascii_case("p")
            || flag.eq_ignore_ascii_case("n")
            || flag.eq_ignore_ascii_case("s")
            || flag.eq_ignore_ascii_case("c")
            || flag.eq_ignore_ascii_case("e")
            || flag.eq_ignore_ascii_case("t")
            || flag.eq_ignore_ascii_case("break")
        {
            return Ok(false);
        }
        if is_setenv_flag(flag) {
            continue;
        }
        // An unknown flag must not discard the rest of the file.
        return Ok(false);
    }
    Ok(true)
}

fn is_setenv_flag(flag: &str) -> bool {
    let flag = flag.trim_matches(|c| c == '[' || c == ']');
    let bytes = flag.as_bytes();
    bytes.len() >= 2 && bytes[0].eq_ignore_ascii_case(&b'e') && bytes[1] == b'='
}

fn directory_index_or_front_controller_default(state: &DirState) -> Option<Vec<String>> {
    if let Some(index) = &state.directory_index {
        return Some(index.clone());
    }
    let target = state.front_controller.as_ref()?.target_uri.as_ref();
    let name = target.trim_start_matches('/');
    if name.eq_ignore_ascii_case("index.php") {
        Some(vec!["index.php".to_string()])
    } else {
        None
    }
}

fn parse_redirect_args(d: &ParsedDirective) -> Result<(u16, String, String), String> {
    if d.args.len() != 3 {
        return Err(format!(
            "{}:{} Redirect requires status from to",
            d.loc.file, d.loc.line
        ));
    }
    let status: u16 = d.args[0]
        .parse()
        .map_err(|_| format!("{}:{} invalid Redirect status", d.loc.file, d.loc.line))?;
    if status != 301 && status != 302 {
        return Err(format!(
            "{}:{} Redirect status must be 301 or 302",
            d.loc.file, d.loc.line
        ));
    }
    Ok((status, normalize_uri(&d.args[1]), d.args[2].clone()))
}

fn compile_safe_rewrite_redirect(
    d: &ParsedDirective,
) -> Result<Option<CompiledRewriteRedirect>, String> {
    if d.args.len() < 2 {
        return Err(format!(
            "{}:{} RewriteRule missing pattern/replacement",
            d.loc.file, d.loc.line
        ));
    }
    let pattern = &d.args[0];
    let replacement = &d.args[1];
    let flags = d.args.get(2).map(String::as_str).unwrap_or("");
    let (status, has_redirect) = parse_rewrite_flags(flags)?;
    if !has_redirect {
        return Ok(None);
    }
    let Some(from) = literal_from_pattern(pattern) else {
        return Ok(None);
    };
    if replacement.contains('$') {
        return Ok(None);
    }
    validate_literal_path(&from)?;
    validate_redirect_target(replacement)?;
    Ok(Some(CompiledRewriteRedirect {
        from_path: from,
        status,
        location: replacement.clone(),
    }))
}

fn parse_rewrite_flags(flags: &str) -> Result<(u16, bool), String> {
    let mut status = 302u16;
    let mut has_redirect = false;
    for flag in flags.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if flag.eq_ignore_ascii_case("l") {
            continue;
        }
        if flag.eq_ignore_ascii_case("nc") {
            continue;
        }
        if let Some(code) = flag.strip_prefix('R').or_else(|| flag.strip_prefix('r')) {
            has_redirect = true;
            if code.is_empty() {
                status = 302;
            } else if let Ok(n) = code.parse::<u16>() {
                if n != 301 && n != 302 {
                    return Err(format!("unsupported redirect code {n}"));
                }
                status = n;
            } else {
                return Err(format!("invalid RewriteRule flag `{flag}`"));
            }
            continue;
        }
        if flag.eq_ignore_ascii_case("last") || flag.eq_ignore_ascii_case("break") {
            return Ok((302, false));
        }
        // E= and any other unknown flag are ignored so they cannot refuse the file.
        continue;
    }
    Ok((status, has_redirect))
}

fn literal_from_pattern(pattern: &str) -> Option<String> {
    let inner = pattern.strip_prefix('^')?.strip_suffix('$')?;
    if inner.contains('(') || inner.contains('$') || inner.contains('*') || inner.contains('?') {
        return None;
    }
    if inner.is_empty() || !inner.starts_with('/') {
        return None;
    }
    Some(inner.to_string())
}

fn normalize_uri(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn validate_literal_path(path: &str) -> Result<(), String> {
    if path.contains("..") || path.contains('\0') {
        return Err(format!("invalid path `{path}`"));
    }
    if !path.starts_with('/') {
        return Err(format!("path must be absolute `{path}`"));
    }
    Ok(())
}

fn validate_redirect_target(target: &str) -> Result<(), String> {
    if target.contains('\0') {
        return Err("invalid redirect target".into());
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        return Ok(());
    }
    validate_literal_path(target)
}

fn merge_inheritance(per_dir: &BTreeMap<String, DirState>) -> BTreeMap<String, DirState> {
    let mut out = BTreeMap::new();
    for (dir, local) in per_dir {
        let mut merged = DirState::default();
        let mut ancestors: Vec<String> = per_dir
            .keys()
            .filter(|p| p.as_str() != dir.as_str() && dir.starts_with(p.as_str()))
            .cloned()
            .collect();
        ancestors.sort_by_key(|p| p.len());
        for anc in &ancestors {
            merge_state(&mut merged, &per_dir[anc]);
        }
        merge_state(&mut merged, local);
        out.insert(dir.clone(), merged);
    }
    out
}

fn merge_state(dst: &mut DirState, src: &DirState) {
    if let Some(idx) = &src.directory_index {
        dst.directory_index = Some(idx.clone());
    }
    dst.redirects.extend(src.redirects.clone());
    dst.rewrite_redirects.extend(src.rewrite_redirects.clone());
    if src.indexes_disabled {
        dst.indexes_disabled = true;
    }
    dst.rewrite_engine = src.rewrite_engine || dst.rewrite_engine;
    if let Some(fc) = &src.front_controller {
        dst.front_controller = Some(fc.clone());
    }
}

fn validate_directory_index_list(args: &[String]) -> Result<Vec<String>, String> {
    if args.len() > MAX_DIRECTORY_INDEX_CANDIDATES {
        return Err(format!(
            "DirectoryIndex exceeds max {MAX_DIRECTORY_INDEX_CANDIDATES} candidates"
        ));
    }
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        if arg.len() > MAX_DIRECTORY_INDEX_CANDIDATE_LEN {
            return Err(format!(
                "DirectoryIndex candidate exceeds max {MAX_DIRECTORY_INDEX_CANDIDATE_LEN} bytes"
            ));
        }
        if !validate_directory_index_candidate(arg) {
            return Err(format!("invalid DirectoryIndex candidate `{arg}`"));
        }
        out.push(arg.clone());
    }
    Ok(out)
}
