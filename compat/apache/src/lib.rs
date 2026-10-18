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
//! Apache httpd → ExyonQ offline importer.
//!
//! Maps common constructs onto existing ExyonQ IR equivalents (static, redirect,
//! TLS, reverse proxy, FastCGI). Anything outside the allowlist is
//! `REJECTED_UNSUPPORTED` with `file:line` and refuses usable IR (fail-closed).
//!
//! AP-07 / ADR-053: path-based migrate expands `Include` / `IncludeOptional`.

mod include;

use anyhow::{bail, Result};
use exyonq_compat_common::{CompatLevel, CompatNote, MigrationReport};
use include::IncludeCtx;
use std::path::Path;

pub const IMPORTER_NAME: &str = "apache";
pub const IMPORTER_VERSION: &str = "0.5.0";

/// String-only migrate saw Include without a filesystem base (ADR-053).
pub const EXY_AP_INCLUDE_0001: &str = "EXY-AP-INCLUDE-0001";

#[derive(Debug, Clone)]
struct Rejected {
    file: String,
    line: usize,
    directive: String,
    detail: String,
}

#[derive(Debug, Clone)]
struct RedirectDraft {
    status: u16,
    from_path: String,
    location: String,
}

/// NG-02 / ADR-048 patterned rewrite (and optional redirect-via-rewrite).
#[derive(Debug, Clone)]
struct RewriteDraft {
    /// Route selector: regex from RewriteRule pattern (unique identity).
    match_regex: String,
    pattern: String,
    replacement: String,
    /// `last` | `break` | `redirect` | `permanent`
    flag: &'static str,
    when: Vec<WhenDraft>,
}

#[derive(Debug, Clone, Default)]
struct WhenDraft {
    path: Option<String>,
    host: Option<String>,
    query: Option<String>,
    header: Option<String>,
    header_value: Option<String>,
}

#[derive(Debug, Clone)]
struct AliasDraft {
    url_path: String,
    fs_path: String,
}

#[derive(Debug, Clone)]
struct ProxyDraft {
    path_prefix: String,
    target: String,
}

#[derive(Debug, Clone)]
struct PhpDraft {
    address: String,
    transport: &'static str,
}

#[derive(Debug, Clone, Default)]
struct TlsDraft {
    engine: bool,
    cert: Option<String>,
    key: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct VirtualHostDraft {
    open_line: usize,
    listen_hint: Option<String>,
    server_name: Option<String>,
    server_alias: Vec<String>,
    document_root: Option<String>,
    directory_index: Option<String>,
    tls: TlsDraft,
    redirects: Vec<RedirectDraft>,
    rewrites: Vec<RewriteDraft>,
    aliases: Vec<AliasDraft>,
    proxies: Vec<ProxyDraft>,
    php: Option<PhpDraft>,
    /// NG-04 / ADR-050: status code → absolute path.
    error_pages: Vec<(u16, String)>,
    rewrite_engine: bool,
    /// RewriteCond lines waiting to attach to the next RewriteRule (AND).
    pending_conds: Vec<WhenDraft>,
    /// Inside `<FilesMatch \.php$>` (or similar).
    in_php_files_match: bool,
    /// F2 / ADR-061: ExpiresActive On seen in this vhost.
    expires_active: bool,
    /// MIME → max_age seconds (ExpiresByType / ExpiresDefault as `"*"`).
    expires_by_mime: Vec<(String, u64)>,
    /// ADR-062 / F1-R19: `Header set Name Value` → route.response_headers (always=true).
    response_headers: Vec<(String, String)>,
}

#[derive(Debug)]
struct ParseState {
    current_file: String,
    global_listen: Option<String>,
    vhosts: Vec<VirtualHostDraft>,
    current: Option<VirtualHostDraft>,
    rejected: Vec<Rejected>,
    notes: Vec<CompatNote>,
}

impl Default for ParseState {
    fn default() -> Self {
        Self {
            current_file: "input.conf".into(),
            global_listen: None,
            vhosts: Vec::new(),
            current: None,
            rejected: Vec::new(),
            notes: Vec::new(),
        }
    }
}

/// Legacy CLI API: `(toml, MigrationReport)`.
pub fn migrate(input: &str) -> Result<(String, MigrationReport)> {
    migrate_labeled("input.conf", input)
}

pub fn migrate_labeled(label: &str, input: &str) -> Result<(String, MigrationReport)> {
    let (res, report) = migrate_with_report(label, input);
    match res {
        Ok(toml) => Ok((toml, report)),
        Err(e) => Err(e),
    }
}

/// Path-based migrate — expands `Include` / `IncludeOptional` (ADR-053).
pub fn migrate_path(path: &Path) -> Result<(String, MigrationReport)> {
    let (res, report) = migrate_path_with_report(path);
    match res {
        Ok(toml) => Ok((toml, report)),
        Err(e) => Err(e),
    }
}

/// Always returns a report (including on refusal). String migrate does **not**
/// expand Includes — use [`migrate_path_with_report`].
pub fn migrate_with_report(label: &str, input: &str) -> (Result<String>, MigrationReport) {
    let mut state = ParseState {
        current_file: label.to_string(),
        ..Default::default()
    };
    if let Err(e) = parse_httpd_lines(input, &mut state, None, 0, None) {
        return (
            Err(e),
            MigrationReport {
                source: IMPORTER_NAME.to_string(),
                notes: state.notes,
                unsupported: rejected_list(&state.rejected),
            },
        );
    }
    finish_migrate(state)
}

/// Path-based migrate with Include expansion (AP-07).
pub fn migrate_path_with_report(path: &Path) -> (Result<String>, MigrationReport) {
    let mut state = ParseState {
        current_file: path.display().to_string(),
        ..Default::default()
    };
    let mut ctx = IncludeCtx::new(path);
    if let Err(e) = parse_httpd_file(path, 0, &mut state, &mut ctx) {
        return (
            Err(e),
            MigrationReport {
                source: IMPORTER_NAME.to_string(),
                notes: state.notes,
                unsupported: rejected_list(&state.rejected),
            },
        );
    }
    finish_migrate(state)
}

fn finish_migrate(state: ParseState) -> (Result<String>, MigrationReport) {
    let unsupported = rejected_list(&state.rejected);
    if !unsupported.is_empty() {
        return (
            Err(anyhow::anyhow!(
                "apache importer refused usable IR ({} REJECTED_UNSUPPORTED)",
                unsupported.len()
            )),
            MigrationReport {
                source: IMPORTER_NAME.to_string(),
                notes: state.notes,
                unsupported,
            },
        );
    }

    match render_toml(&state) {
        Ok(toml) => {
            let mut notes = state.notes;
            notes.push(CompatNote {
                level: CompatLevel::Migrated,
                message: format!(
                    "apache {IMPORTER_VERSION}: mapped {} VirtualHost(s) to ExyonQ IR equivalents",
                    state.vhosts.len()
                ),
            });
            (
                Ok(toml),
                MigrationReport {
                    source: IMPORTER_NAME.to_string(),
                    notes,
                    unsupported: Vec::new(),
                },
            )
        }
        Err(e) => (
            Err(e),
            MigrationReport {
                source: IMPORTER_NAME.to_string(),
                notes: state.notes,
                unsupported,
            },
        ),
    }
}

fn rejected_list(rejected: &[Rejected]) -> Vec<String> {
    rejected
        .iter()
        .map(|r| {
            format!(
                "REJECTED_UNSUPPORTED {}:{} `{}` — {}",
                r.file, r.line, r.directive, r.detail
            )
        })
        .collect()
}

fn parse_httpd_file(
    path: &Path,
    depth: usize,
    state: &mut ParseState,
    ctx: &mut IncludeCtx,
) -> Result<()> {
    let (canonical, content) = match ctx.begin_file(path, depth) {
        Ok(v) => v,
        Err(d) => {
            reject(state, 1, "Include", d.message());
            bail!("{}", d.message());
        }
    };
    let prev = std::mem::replace(&mut state.current_file, canonical.display().to_string());
    let res = parse_httpd_lines(&content, state, Some(&canonical), depth, Some(ctx));
    state.current_file = prev;
    res
}

fn parse_httpd_lines(
    input: &str,
    state: &mut ParseState,
    including_file: Option<&Path>,
    depth: usize,
    mut ctx: Option<&mut IncludeCtx>,
) -> Result<()> {
    for (idx, raw_line) in input.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        let (name, args) = split_directive(line);
        if eq_ci(name, "Include") || eq_ci(name, "IncludeOptional") {
            handle_include(
                line_no,
                name,
                args,
                state,
                including_file,
                depth,
                ctx.as_deref_mut(),
            )?;
            continue;
        }

        if let Some(rest) = line.strip_prefix("<VirtualHost") {
            if state.current.is_some() {
                bail!("line {line_no}: nested <VirtualHost> is not supported");
            }
            let mut draft = VirtualHostDraft {
                open_line: line_no,
                ..Default::default()
            };
            if let Some(addr) = rest.strip_suffix('>').map(str::trim) {
                draft.listen_hint = Some(normalize_vhost_addr(addr));
            }
            state.current = Some(draft);
            continue;
        }

        if line.eq_ignore_ascii_case("</VirtualHost>") {
            let Some(mut draft) = state.current.take() else {
                bail!("line {line_no}: </VirtualHost> without open block");
            };
            draft.in_php_files_match = false;
            validate_vhost(&draft, state);
            state.vhosts.push(draft);
            continue;
        }

        // FilesMatch open/close (PHP handler only).
        if let Some(rest) = line.strip_prefix("<FilesMatch") {
            let Some(draft) = state.current.as_mut() else {
                reject(
                    state,
                    line_no,
                    "FilesMatch",
                    "FilesMatch only supported inside VirtualHost",
                );
                continue;
            };
            let pat = rest
                .strip_suffix('>')
                .map(str::trim)
                .unwrap_or("")
                .trim_matches('"');
            if is_php_files_match_pattern(pat) {
                draft.in_php_files_match = true;
            } else {
                reject(
                    state,
                    line_no,
                    "FilesMatch",
                    format!("only FilesMatch for PHP (e.g. \\.php$) is mapped; got `{pat}`"),
                );
            }
            continue;
        }
        if line.eq_ignore_ascii_case("</FilesMatch>") {
            if let Some(draft) = state.current.as_mut() {
                draft.in_php_files_match = false;
            } else {
                reject(
                    state,
                    line_no,
                    "</FilesMatch>",
                    "closing tag outside VirtualHost",
                );
            }
            continue;
        }

        let (name, args) = split_directive(line);
        if state.current.is_some() {
            handle_vhost_directive(line_no, name, args, state);
        } else {
            handle_global_directive(line_no, name, args, state);
        }
    }

    // Nested Include fragments may sit inside an open VirtualHost — only the
    // outermost parse may demand closure (ADR-053).
    if depth == 0 && state.current.is_some() {
        bail!("unclosed <VirtualHost> at end of file");
    }
    Ok(())
}

fn handle_include(
    line_no: usize,
    name: &str,
    args: &str,
    state: &mut ParseState,
    including_file: Option<&Path>,
    depth: usize,
    ctx: Option<&mut IncludeCtx>,
) -> Result<()> {
    let optional = eq_ci(name, "IncludeOptional");
    let pattern = unquote(args.trim());
    if pattern.is_empty() {
        reject(
            state,
            line_no,
            name,
            format!("{EXY_AP_INCLUDE_0001}: {name} requires a path or glob argument"),
        );
        return Ok(());
    }

    let Some(ctx) = ctx else {
        reject(
            state,
            line_no,
            name,
            format!(
                "{EXY_AP_INCLUDE_0001}: {name} requires path-based migrate (migrate_path / exyonq-compat --input <file>)"
            ),
        );
        return Ok(());
    };
    let Some(including) = including_file else {
        reject(
            state,
            line_no,
            name,
            format!("{EXY_AP_INCLUDE_0001}: {name} requires path-based migrate"),
        );
        return Ok(());
    };

    let targets = match ctx.resolve_targets(including, pattern, optional) {
        Ok(paths) => paths,
        Err(include::IncludeDiag::OptionalEmpty { .. }) => {
            state.notes.push(CompatNote {
                level: CompatLevel::Approximated,
                message: format!(
                    "{}:{}: {}",
                    state.current_file,
                    line_no,
                    include::IncludeDiag::OptionalEmpty {
                        pattern: pattern.to_string(),
                    }
                    .message()
                ),
            });
            return Ok(());
        }
        Err(d) => {
            reject(state, line_no, name, d.message());
            return Ok(());
        }
    };

    state.notes.push(CompatNote {
        level: CompatLevel::Migrated,
        message: format!(
            "{}:{}: expanded {name} `{pattern}` ({} file(s))",
            state.current_file,
            line_no,
            targets.len()
        ),
    });

    for target in targets {
        parse_httpd_file(&target, depth + 1, state, ctx)?;
    }
    Ok(())
}

fn validate_vhost(draft: &VirtualHostDraft, state: &mut ParseState) {
    if draft.server_name.is_none() {
        reject(
            state,
            draft.open_line,
            "ServerName",
            "required inside VirtualHost",
        );
    }
    // Static site needs DocumentRoot; PHP-only or proxy-only may omit if they
    // have another primary action — still require DocumentRoot when php is set
    // (fcgi_pool.document_root) or when no proxy covers `/`.
    let has_root_proxy = draft
        .proxies
        .iter()
        .any(|p| p.path_prefix == "/" || p.path_prefix.is_empty());
    let has_rewrites = !draft.rewrites.is_empty() || !draft.redirects.is_empty();
    if draft.document_root.is_none() && draft.php.is_none() && !has_root_proxy && !has_rewrites {
        reject(
            state,
            draft.open_line,
            "DocumentRoot",
            "required unless PHP-FPM, ProxyPass /, or RewriteRule/Redirect is configured",
        );
    }
    if draft.tls.engine && (draft.tls.cert.is_none() || draft.tls.key.is_none()) {
        reject(
            state,
            draft.open_line,
            "SSLEngine",
            "SSLEngine on requires SSLCertificateFile and SSLCertificateKeyFile",
        );
    }
    if !draft.pending_conds.is_empty() {
        reject(
            state,
            draft.open_line,
            "RewriteCond",
            "RewriteCond without a following RewriteRule in this VirtualHost",
        );
    }
}

fn handle_global_directive(line: usize, name: &str, args: &str, state: &mut ParseState) {
    if eq_ci(name, "Listen") {
        if args.is_empty() {
            reject(state, line, name, "Listen requires a port or address:port");
            return;
        }
        state.global_listen = Some(normalize_listen(args));
        return;
    }
    if name.starts_with('<') {
        reject(
            state,
            line,
            name,
            "container/block not supported outside VirtualHost in this importer cut",
        );
        return;
    }
    reject(
        state,
        line,
        name,
        "directive not supported at global scope in this importer cut",
    );
}

fn handle_vhost_directive(line: usize, name: &str, args: &str, state: &mut ParseState) {
    // Capture php-files-match flag before other borrows.
    let in_php = state
        .current
        .as_ref()
        .map(|d| d.in_php_files_match)
        .unwrap_or(false);

    if eq_ci(name, "ServerName") {
        if let Some(d) = state.current.as_mut() {
            d.server_name = Some(first_token(args).to_string());
        }
        return;
    }
    if eq_ci(name, "ServerAlias") {
        if let Some(d) = state.current.as_mut() {
            for tok in args.split_whitespace() {
                d.server_alias.push(tok.to_string());
            }
        }
        return;
    }
    if eq_ci(name, "DocumentRoot") {
        if let Some(d) = state.current.as_mut() {
            d.document_root = Some(unquote(args.trim()).to_string());
        }
        return;
    }
    if eq_ci(name, "DirectoryIndex") {
        if let Some(d) = state.current.as_mut() {
            d.directory_index = Some(first_token(args).to_string());
        }
        state.notes.push(CompatNote {
            level: CompatLevel::Approximated,
            message: format!(
                "line {line}: DirectoryIndex maps only the first filename to route index"
            ),
        });
        return;
    }
    if eq_ci(name, "SSLEngine") {
        let on = first_token(args).eq_ignore_ascii_case("on");
        if let Some(d) = state.current.as_mut() {
            d.tls.engine = on;
        }
        if !on && !first_token(args).eq_ignore_ascii_case("off") {
            reject(state, line, name, "SSLEngine expects on|off");
        }
        return;
    }
    if eq_ci(name, "SSLCertificateFile") {
        if let Some(d) = state.current.as_mut() {
            d.tls.cert = Some(unquote(args.trim()).to_string());
        }
        return;
    }
    if eq_ci(name, "SSLCertificateKeyFile") {
        if let Some(d) = state.current.as_mut() {
            d.tls.key = Some(unquote(args.trim()).to_string());
        }
        return;
    }
    if eq_ci(name, "Redirect") {
        match parse_redirect(args) {
            Ok(r) => {
                if let Some(d) = state.current.as_mut() {
                    d.redirects.push(r);
                }
            }
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "Alias") {
        let mut parts = args.split_whitespace();
        let url = parts.next().map(unquote);
        let fs = parts.next().map(unquote);
        match (url, fs) {
            (Some(u), Some(f)) if u.starts_with('/') => {
                if let Some(d) = state.current.as_mut() {
                    d.aliases.push(AliasDraft {
                        url_path: u.to_string(),
                        fs_path: f.to_string(),
                    });
                }
            }
            _ => reject(
                state,
                line,
                name,
                "Alias requires URL-path and filesystem-path",
            ),
        }
        return;
    }
    if eq_ci(name, "ProxyPass") {
        match parse_proxy_pass(args) {
            Ok(p) => {
                if let Some(d) = state.current.as_mut() {
                    d.proxies.push(p);
                }
            }
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "ProxyPassReverse") {
        // No ExyonQ Location-rewrite yet — declare difference if pair matches ProxyPass.
        state.notes.push(CompatNote {
            level: CompatLevel::Approximated,
            message: format!(
                "line {line}: ProxyPassReverse has no IR equivalent yet; ignored for mapping (document manually)"
            ),
        });
        // Still reject to avoid silent semantic drop of reverse Location fixups.
        reject(
            state,
            line,
            name,
            "no ExyonQ equivalent for response Location rewrite (ProxyPassReverse); remove or wait for product support",
        );
        return;
    }
    if eq_ci(name, "ProxyPassMatch") {
        match parse_php_proxy_pass_match(args) {
            Ok(php) => {
                if let Some(d) = state.current.as_mut() {
                    d.php = Some(php);
                }
                state.notes.push(CompatNote {
                    level: CompatLevel::Migrated,
                    message: format!(
                        "line {line}: ProxyPassMatch PHP → [[fcgi_pool]] + route.fastcgi"
                    ),
                });
            }
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "SetHandler") {
        if !in_php {
            reject(
                state,
                line,
                name,
                "SetHandler only mapped inside <FilesMatch \\.php$>",
            );
            return;
        }
        match parse_set_handler_php(args) {
            Ok(php) => {
                if let Some(d) = state.current.as_mut() {
                    d.php = Some(php);
                }
            }
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "RewriteEngine") {
        let on = first_token(args).eq_ignore_ascii_case("on");
        if let Some(d) = state.current.as_mut() {
            d.rewrite_engine = on;
        }
        return;
    }
    if eq_ci(name, "RewriteRule") {
        let engine = state
            .current
            .as_ref()
            .map(|d| d.rewrite_engine)
            .unwrap_or(false);
        if !engine {
            reject(state, line, name, "RewriteRule requires RewriteEngine On");
            return;
        }
        let when = state
            .current
            .as_mut()
            .map(|d| std::mem::take(&mut d.pending_conds))
            .unwrap_or_default();
        match map_rewrite_rule(args, when) {
            Ok(MappedRewrite::Redirect(r)) => {
                if let Some(d) = state.current.as_mut() {
                    d.redirects.push(r);
                }
                state.notes.push(CompatNote {
                    level: CompatLevel::Migrated,
                    message: format!(
                        "line {line}: safe RewriteRule [R] → route.redirect (literal path only)"
                    ),
                });
            }
            Ok(MappedRewrite::Rule(rw)) => {
                if let Some(d) = state.current.as_mut() {
                    d.rewrites.push(rw);
                }
                state.notes.push(CompatNote {
                    level: CompatLevel::Migrated,
                    message: format!(
                        "line {line}: RewriteRule → match.regex + rewrite rule (ADR-048 / NG-02)"
                    ),
                });
            }
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "RewriteCond") {
        let engine = state
            .current
            .as_ref()
            .map(|d| d.rewrite_engine)
            .unwrap_or(false);
        if !engine {
            reject(state, line, name, "RewriteCond requires RewriteEngine On");
            return;
        }
        match parse_rewrite_cond(args) {
            Ok(when) => {
                if let Some(d) = state.current.as_mut() {
                    d.pending_conds.push(when);
                }
                state.notes.push(CompatNote {
                    level: CompatLevel::Migrated,
                    message: format!(
                        "line {line}: RewriteCond → rewrite.when (ADR-048 path/host/query/header)"
                    ),
                });
            }
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "ErrorDocument") {
        match parse_error_document(args) {
            Ok((code, path)) => {
                if let Some(vh) = state.current.as_mut() {
                    vh.error_pages.push((code, path.clone()));
                    state.notes.push(CompatNote {
                        level: CompatLevel::Migrated,
                        message: format!(
                            "line {line}: ErrorDocument {code} → error_pages[\"{code}\"] = `{path}` (ADR-050 / NG-04)"
                        ),
                    });
                } else {
                    reject(
                        state,
                        line,
                        name,
                        "ErrorDocument outside <VirtualHost> not supported",
                    );
                }
            }
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "Header") {
        // ADR-062: `Header set Name Value` only (always=true on emit).
        let tokens: Vec<&str> = args.split_whitespace().collect();
        if tokens.len() >= 3 && tokens[0].eq_ignore_ascii_case("set") {
            let hname = tokens[1].to_string();
            let value = tokens[2..].join(" ");
            if is_apache_response_header_denied(&hname) {
                reject(
                    state,
                    line,
                    name,
                    format!("Header set `{hname}` denied by ADR-062 denylist"),
                );
                return;
            }
            if value.contains('$') || value.contains('%') {
                reject(
                    state,
                    line,
                    name,
                    "Header set with expressions/vars not supported (ADR-062 literals only)",
                );
                return;
            }
            if let Some(d) = state.current.as_mut() {
                if d.response_headers
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case(&hname))
                {
                    reject(
                        state,
                        line,
                        name,
                        format!("duplicate Header set `{hname}` (ADR-062)"),
                    );
                    return;
                }
                d.response_headers.push((hname.clone(), value.clone()));
                state.notes.push(CompatNote {
                    level: CompatLevel::Migrated,
                    message: format!(
                        "line {line}: Header set {hname} → response_headers always=true (ADR-062)"
                    ),
                });
            } else {
                reject(state, line, name, "Header outside <VirtualHost> not supported");
            }
        } else if !tokens.is_empty()
            && matches!(
                tokens[0].to_ascii_lowercase().as_str(),
                "append" | "add" | "edit" | "unset" | "merge" | "echo"
            )
        {
            reject(
                state,
                line,
                name,
                format!(
                    "Header {} not supported (ADR-062: only `Header set`)",
                    tokens[0]
                ),
            );
        } else {
            reject(state, line, name, "Header expects `set Name Value` (ADR-062)");
        }
        return;
    }
    if eq_ci(name, "ExpiresActive") {
        let on = first_token(args).eq_ignore_ascii_case("on");
        if let Some(d) = state.current.as_mut() {
            d.expires_active = on;
        }
        if !on && !first_token(args).eq_ignore_ascii_case("off") {
            reject(state, line, name, "ExpiresActive expects on|off");
        }
        return;
    }
    if eq_ci(name, "ExpiresByType") {
        match parse_expires_by_type(args) {
            Ok((mime, max_age)) => {
                if let Some(d) = state.current.as_mut() {
                    if !d.expires_active {
                        state.notes.push(CompatNote {
                            level: CompatLevel::Approximated,
                            message: format!(
                                "line {line}: ExpiresByType without ExpiresActive On — still recorded for migration"
                            ),
                        });
                    }
                    d.expires_by_mime.push((mime, max_age));
                    d.expires_active = true;
                }
            }
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "ExpiresDefault") {
        match parse_expires_duration(args.trim()) {
            Ok(max_age) => {
                if let Some(d) = state.current.as_mut() {
                    d.expires_by_mime.push(("*".into(), max_age));
                    d.expires_active = true;
                }
            }
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "AuthType")
        || eq_ci(name, "AuthUserFile")
        || eq_ci(name, "AuthName")
        || eq_ci(name, "Require")
    {
        reject(
            state,
            line,
            name,
            "Basic Auth / Require not implemented in ExyonQ yet",
        );
        return;
    }

    reject(
        state,
        line,
        name,
        "directive not in apache equivalent allowlist for this cut",
    );
}

/// F2 / ADR-061: `ExpiresByType text/css "access plus 1 month"` or `… A2592000`.
fn parse_expires_by_type(args: &str) -> Result<(String, u64), String> {
    let args = args.trim();
    let (mime, rest) = match args.split_once(char::is_whitespace) {
        Some((m, r)) => (unquote(m.trim()).to_string(), r.trim()),
        None => return Err("ExpiresByType requires MIME type and duration".into()),
    };
    if mime.is_empty() || !mime.contains('/') && mime != "*" {
        return Err("ExpiresByType MIME type must look like type/subtype".into());
    }
    let max_age = parse_expires_duration(rest)?;
    Ok((mime, max_age))
}

/// Parse Apache Expires duration → seconds (access-based only).
fn parse_expires_duration(spec: &str) -> Result<u64, String> {
    let spec = unquote(spec.trim());
    if spec.is_empty() {
        return Err("empty Expires duration".into());
    }
    // A2592000 / M2592000 — only access (A) supported this cut.
    if let Some(rest) = spec.strip_prefix('A').or_else(|| spec.strip_prefix('a')) {
        if rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
            return rest
                .parse::<u64>()
                .map_err(|_| format!("invalid Expires A-code `{spec}`"));
        }
    }
    if spec.starts_with('M') || spec.starts_with('m') {
        return Err("modification-based Expires (M####) not supported; use A#### or access plus …".into());
    }
    // "access plus 1 month"
    let lower = spec.to_ascii_lowercase();
    let body = lower
        .strip_prefix("access plus ")
        .or_else(|| lower.strip_prefix("access "))
        .ok_or_else(|| {
            format!("unsupported Expires duration `{spec}` (want A#### or access plus N unit)")
        })?;
    let mut parts = body.split_whitespace();
    let n: u64 = parts
        .next()
        .ok_or_else(|| format!("missing count in `{spec}`"))?
        .parse()
        .map_err(|_| format!("bad count in `{spec}`"))?;
    let unit = parts
        .next()
        .ok_or_else(|| format!("missing unit in `{spec}`"))?;
    let mult = match unit.trim_end_matches('s') {
        "year" => 365 * 24 * 3600u64,
        "month" => 30 * 24 * 3600,
        "week" => 7 * 24 * 3600,
        "day" => 24 * 3600,
        "hour" => 3600,
        "minute" => 60,
        "second" => 1,
        _ => {
            return Err(format!(
                "unsupported Expires unit `{unit}` (year|month|week|day|hour|minute|second)"
            ))
        }
    };
    Ok(n.saturating_mul(mult))
}

fn is_apache_response_header_denied(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "cache-control" || lower == "set-cookie" {
        return true;
    }
    if lower.starts_with("proxy-") {
        return true;
    }
    matches!(
        lower.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn emit_response_headers_toml(out: &mut String, headers: &[(String, String)]) {
    for (name, value) in headers {
        out.push_str("[[route.response_headers]]\n");
        out.push_str(&format!(
            "name = \"{}\"\nvalue = \"{}\"\nalways = true\n",
            escape_toml(name),
            escape_toml(value)
        ));
    }
}

fn emit_cache_headers_toml(out: &mut String, by_mime: &[(String, u64)]) {
    if by_mime.is_empty() {
        return;
    }
    for (mime, max_age) in by_mime {
        out.push_str(&format!(
            "[route.cache_headers.by_mime.\"{}\"]\n",
            escape_toml(mime)
        ));
        out.push_str(&format!("max_age = {max_age}\n"));
    }
}

/// NG-04 / ADR-050: `ErrorDocument 404 /404.html`
fn parse_error_document(args: &str) -> Result<(u16, String), String> {
    let mut parts = args.split_whitespace();
    let code_raw = parts
        .next()
        .ok_or("ErrorDocument requires status and path")?;
    let path_raw = parts
        .next()
        .ok_or("ErrorDocument requires status and path")?;
    if parts.next().is_some() {
        return Err("ErrorDocument: unexpected trailing tokens".into());
    }
    let code: u16 = code_raw
        .parse()
        .map_err(|_| format!("ErrorDocument status `{code_raw}` must be an integer"))?;
    if !(400..=599).contains(&code) {
        return Err(format!("ErrorDocument status {code} must be in 400..=599"));
    }
    let path = unquote(path_raw).to_string();
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("//") {
        return Err(format!(
            "ErrorDocument URL redirect `{path}` not supported (ADR-050)"
        ));
    }
    if !path.starts_with('/') || path.contains('?') || path.contains('#') {
        return Err(format!(
            "ErrorDocument target `{path}` must be an absolute path (no text body / URL)"
        ));
    }
    Ok((code, path))
}

fn parse_redirect(args: &str) -> Result<RedirectDraft, String> {
    let mut toks: Vec<&str> = args.split_whitespace().collect();
    if toks.len() < 2 {
        return Err("Redirect requires path and target".into());
    }
    let mut status = 302u16;
    let first = toks[0];
    if first.eq_ignore_ascii_case("permanent") {
        status = 301;
        toks.remove(0);
    } else if first.eq_ignore_ascii_case("temp") {
        status = 302;
        toks.remove(0);
    } else if first.chars().all(|c| c.is_ascii_digit()) {
        status = first
            .parse()
            .map_err(|_| format!("invalid Redirect status `{first}`"))?;
        if !matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Err(format!("unsupported Redirect status {status}"));
        }
        toks.remove(0);
    }
    if toks.len() < 2 {
        return Err("Redirect requires path and target".into());
    }
    let from = unquote(toks[0]);
    let location = unquote(toks[1]);
    if !from.starts_with('/') {
        return Err("Redirect URL-path must start with /".into());
    }
    if location.is_empty() {
        return Err("Redirect target empty".into());
    }
    Ok(RedirectDraft {
        status,
        from_path: from.to_string(),
        location: location.to_string(),
    })
}

fn parse_proxy_pass(args: &str) -> Result<ProxyDraft, String> {
    let mut parts = args.split_whitespace();
    let path = parts.next().map(unquote).ok_or("ProxyPass requires path")?;
    let target = parts.next().map(unquote).ok_or("ProxyPass requires url")?;
    if parts.next().is_some() {
        return Err("ProxyPass extra arguments not supported".into());
    }
    if !path.starts_with('/') {
        return Err("ProxyPass path must start with /".into());
    }
    let target = normalize_proxy_target(target)?;
    Ok(ProxyDraft {
        path_prefix: path.trim_end_matches('/').to_string(),
        target,
    })
}

/// Authority-only http://host:port (optional trailing slash).
fn normalize_proxy_target(target: &str) -> Result<String, String> {
    let t = target.trim_end_matches('/');
    if !(t.starts_with("http://") || t.starts_with("https://")) {
        return Err("ProxyPass target must be http:// or https:// authority".into());
    }
    let rest = t
        .split_once("://")
        .map(|(_, r)| r)
        .ok_or("invalid ProxyPass URL")?;
    if rest.contains('/') {
        return Err(
            "ProxyPass target must be authority-only (no upstream path); use ExyonQ route path"
                .into(),
        );
    }
    if rest.contains('$') || rest.contains('{') {
        return Err("ProxyPass variables not supported".into());
    }
    Ok(t.to_string())
}

fn parse_php_proxy_pass_match(args: &str) -> Result<PhpDraft, String> {
    // ProxyPassMatch ^/(.+\.php.*)$ unix:/run/php/php-fpm.sock|fcgi://localhost/
    let mut parts = args.split_whitespace();
    let pattern = parts.next().ok_or("ProxyPassMatch requires pattern")?;
    let handler = parts.next().ok_or("ProxyPassMatch requires handler")?;
    if !pattern.to_ascii_lowercase().contains(".php") {
        return Err("ProxyPassMatch only mapped for PHP patterns".into());
    }
    parse_fcgi_handler(handler)
}

fn parse_set_handler_php(args: &str) -> Result<PhpDraft, String> {
    let raw = unquote(args.trim());
    parse_fcgi_handler(raw)
}

fn parse_fcgi_handler(handler: &str) -> Result<PhpDraft, String> {
    let h = handler.trim().trim_matches('"');
    if let Some(rest) = h.strip_prefix("proxy:unix:") {
        // unix:/path|fcgi://localhost
        let path = rest
            .split('|')
            .next()
            .ok_or("invalid proxy:unix SetHandler")?;
        if path.is_empty() {
            return Err("empty unix socket path".into());
        }
        return Ok(PhpDraft {
            address: path.to_string(),
            transport: "unix",
        });
    }
    if let Some(rest) = h.strip_prefix("proxy:fcgi://") {
        // fcgi://127.0.0.1:9000/…
        let auth = rest.split('/').next().unwrap_or(rest);
        if auth.is_empty() {
            return Err("empty fcgi authority".into());
        }
        return Ok(PhpDraft {
            address: auth.to_string(),
            transport: "tcp",
        });
    }
    if let Some(rest) = h.strip_prefix("unix:") {
        let path = rest.split('|').next().unwrap_or(rest);
        return Ok(PhpDraft {
            address: path.to_string(),
            transport: "unix",
        });
    }
    Err(
        "SetHandler/ProxyPassMatch PHP handler must be proxy:unix:…|fcgi://… or proxy:fcgi://…"
            .into(),
    )
}

enum MappedRewrite {
    Redirect(RedirectDraft),
    Rule(RewriteDraft),
}

/// Map `RewriteRule pattern subst [flags]` onto redirect IR or NG-02 rewrite rule.
fn map_rewrite_rule(args: &str, when: Vec<WhenDraft>) -> Result<MappedRewrite, String> {
    let mut parts = args.split_whitespace();
    let pattern_raw = parts.next().ok_or("RewriteRule requires pattern")?;
    let replacement_raw = parts.next().ok_or("RewriteRule requires substitution")?;
    let flags = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return Err("RewriteRule: unexpected trailing tokens".into());
    }

    let mut pattern = normalize_vhost_rewrite_pattern(unquote(pattern_raw));
    let replacement = unquote(replacement_raw).to_string();
    let parsed = parse_rewrite_flags(flags)?;

    if parsed.nc && !pattern.starts_with("(?i)") && !pattern.starts_with("(?i:") {
        pattern = format!("(?i){pattern}");
    }

    if replacement_has_unsupported_vars(&replacement) {
        return Err(
            "RewriteRule substitution supports only $1..$9 and $$ (no %N / env vars)".into(),
        );
    }

    // Safe literal external redirect → Cap033 redirect route (no rewrite field).
    if let Some(status) = parsed.redirect_status {
        if when.is_empty() {
            if let Some(from) = literal_path_from_pattern(&pattern) {
                if !replacement.contains('$') {
                    return Ok(MappedRewrite::Redirect(RedirectDraft {
                        status,
                        from_path: from,
                        location: replacement,
                    }));
                }
            }
        }
        let flag = if status == 301 || status == 308 {
            "permanent"
        } else {
            "redirect"
        };
        return Ok(MappedRewrite::Rule(RewriteDraft {
            match_regex: pattern.clone(),
            pattern,
            replacement,
            flag,
            when,
        }));
    }

    // Internal rewrite: [L] / bare → last; [END] not mapped.
    if parsed.end {
        return Err("RewriteRule [END] is not mapped; use [L] → flag=last".into());
    }
    let flag = "last";

    Ok(MappedRewrite::Rule(RewriteDraft {
        match_regex: pattern.clone(),
        pattern,
        replacement,
        flag,
        when,
    }))
}

#[derive(Debug, Default)]
struct ParsedRewriteFlags {
    redirect_status: Option<u16>,
    nc: bool,
    end: bool,
}

fn parse_rewrite_flags(flags: &str) -> Result<ParsedRewriteFlags, String> {
    let f = flags.trim().trim_start_matches('[').trim_end_matches(']');
    let mut out = ParsedRewriteFlags::default();
    if f.is_empty() {
        return Ok(out);
    }
    for part in f.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if p.eq_ignore_ascii_case("R") || p.eq_ignore_ascii_case("redirect") {
            out.redirect_status = Some(out.redirect_status.unwrap_or(302));
        } else if let Some(code) = p
            .strip_prefix("R=")
            .or_else(|| p.strip_prefix("r="))
            .or_else(|| p.strip_prefix("redirect="))
        {
            let status: u16 = code
                .parse()
                .map_err(|_| format!("invalid redirect status in flag `{p}`"))?;
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                return Err(format!("unsupported redirect status {status}"));
            }
            out.redirect_status = Some(status);
        } else if p.eq_ignore_ascii_case("L") || p.eq_ignore_ascii_case("last") {
            // last is default for internal; with R it is still redirect.
        } else if p.eq_ignore_ascii_case("NC") || p.eq_ignore_ascii_case("nocase") {
            out.nc = true;
        } else if p.eq_ignore_ascii_case("END") {
            out.end = true;
        } else if p.eq_ignore_ascii_case("B")
            || p.eq_ignore_ascii_case("BNP")
            || p.eq_ignore_ascii_case("QSA")
            || p.eq_ignore_ascii_case("QSD")
            || p.eq_ignore_ascii_case("NE")
            || p.eq_ignore_ascii_case("PT")
            || p.eq_ignore_ascii_case("P")
            || p.eq_ignore_ascii_case("F")
            || p.eq_ignore_ascii_case("G")
            || p.eq_ignore_ascii_case("N")
            || p.eq_ignore_ascii_case("C")
            || p.eq_ignore_ascii_case("NS")
            || p.eq_ignore_ascii_case("S")
            || p.eq_ignore_ascii_case("T")
            || p.eq_ignore_ascii_case("H")
        {
            return Err(format!(
                "RewriteRule flag `[{p}]` not mapped (fail-closed); supported: R[=code], L, NC"
            ));
        } else {
            return Err(format!("unknown RewriteRule flag `{p}`"));
        }
    }
    Ok(out)
}

/// VirtualHost context: patterns usually include a leading `/`. If the pattern
/// starts with `^` but the next character is not `/`, insert `/` so path match
/// aligns with ExyonQ URI paths.
fn normalize_vhost_rewrite_pattern(pattern: &str) -> String {
    let p = pattern.trim();
    if let Some(rest) = p.strip_prefix('^') {
        if !rest.is_empty() && !rest.starts_with('/') && !rest.starts_with('(') {
            return format!("^/{rest}");
        }
    }
    p.to_string()
}

fn replacement_has_unsupported_vars(replacement: &str) -> bool {
    let mut chars = replacement.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            return true;
        }
        if c != '$' {
            continue;
        }
        match chars.next() {
            Some('$') => {}
            Some(d) if ('1'..='9').contains(&d) => {}
            _ => return true,
        }
    }
    false
}

/// Map one RewriteCond TestString CondPattern [flags] → ADR-048 `when` entry.
fn parse_rewrite_cond(args: &str) -> Result<WhenDraft, String> {
    let mut parts = args.split_whitespace();
    let test = parts.next().ok_or("RewriteCond requires TestString")?;
    let pattern = parts.next().ok_or("RewriteCond requires CondPattern")?;
    let flags = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return Err("RewriteCond: unexpected trailing tokens".into());
    }

    let f = flags.trim().trim_start_matches('[').trim_end_matches(']');
    let mut nc = false;
    for part in f.split(',').filter(|p| !p.trim().is_empty()) {
        let p = part.trim();
        if p.eq_ignore_ascii_case("NC") || p.eq_ignore_ascii_case("nocase") {
            nc = true;
        } else if p.eq_ignore_ascii_case("OR") {
            return Err("RewriteCond [OR] not mapped (only AND of preceding conds)".into());
        } else if p.eq_ignore_ascii_case("NV") {
            // no-op for mapped TestStrings
        } else {
            return Err(format!("RewriteCond flag `[{p}]` not mapped"));
        }
    }

    let mut pat = unquote(pattern).to_string();
    // Apache `!` negation — not in ADR-048 when.
    if pat.starts_with('!') {
        return Err("RewriteCond negation (!) not mapped".into());
    }
    // Lexicographic / integer comparisons.
    if pat.starts_with('>') || pat.starts_with('<') || pat.starts_with('=') {
        if let Some(exact) = pat.strip_prefix('=') {
            // Exact string match — wrap as anchored regex.
            let escaped = regex_escape_literal(exact);
            pat = format!("^{escaped}$");
        } else {
            return Err("RewriteCond lexicographic/integer comparisons not mapped".into());
        }
    }
    if nc && !pat.starts_with("(?i)") {
        pat = format!("(?i){pat}");
    }

    let test = test.trim();
    if eq_ci(test, "%{REQUEST_URI}") || eq_ci(test, "%{THE_REQUEST}") {
        // THE_REQUEST is broader; only allow REQUEST_URI for path.
        if eq_ci(test, "%{THE_REQUEST}") {
            return Err("%{THE_REQUEST} not mapped; use %{REQUEST_URI}".into());
        }
        return Ok(WhenDraft {
            path: Some(pat),
            ..Default::default()
        });
    }
    if eq_ci(test, "%{HTTP_HOST}") || eq_ci(test, "%{SERVER_NAME}") {
        // Prefer exact host when pattern is ^literal$ without regex meta.
        if let Some(host) = literal_host_from_cond_pattern(&pat) {
            return Ok(WhenDraft {
                host: Some(host),
                ..Default::default()
            });
        }
        // Regex host not in ADR-048 when.host (exact only) — reject.
        return Err(
            "RewriteCond %{HTTP_HOST} only maps exact host (=host or ^host$); regex host unsupported"
                .into(),
        );
    }
    if eq_ci(test, "%{QUERY_STRING}") {
        return Ok(WhenDraft {
            query: Some(pat),
            ..Default::default()
        });
    }
    if let Some(hdr) = test
        .strip_prefix("%{HTTP:")
        .and_then(|s| s.strip_suffix('}'))
    {
        let name = hdr.trim();
        if name.is_empty() {
            return Err("RewriteCond %{HTTP:} empty header name".into());
        }
        return Ok(WhenDraft {
            header: Some(name.to_string()),
            header_value: Some(pat),
            ..Default::default()
        });
    }
    Err(format!(
        "RewriteCond TestString `{test}` not mapped; supported: %{{REQUEST_URI}}, %{{HTTP_HOST}}, %{{QUERY_STRING}}, %{{HTTP:Header}}"
    ))
}

fn literal_host_from_cond_pattern(pat: &str) -> Option<String> {
    let p = pat.trim();
    let p = p.strip_prefix("(?i)").unwrap_or(p);
    let inner = if let Some(i) = p.strip_prefix('^').and_then(|s| s.strip_suffix('$')) {
        i
    } else if p
        .chars()
        .any(|c| matches!(c, '^' | '$' | '(' | '[' | '*' | '?' | '+' | '|'))
    {
        return None;
    } else {
        p
    };
    unescape_host_literal(inner)
}

/// Host exact match: letters/digits/hyphen/dot; `\.` → `.`. Reject other regex meta.
fn unescape_host_literal(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('.') => out.push('.'),
                _ => return None,
            }
        } else if matches!(
            c,
            '*' | '?' | '+' | '|' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            return None;
        } else {
            out.push(c);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn regex_escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn literal_path_from_pattern(pattern: &str) -> Option<String> {
    let p = pattern.trim();
    let inner = p.strip_prefix('^')?.strip_suffix('$')?;
    if inner.contains('(')
        || inner.contains('[')
        || inner.contains('*')
        || inner.contains('+')
        || inner.contains('?')
        || inner.contains('.')
        || inner.contains('|')
    {
        // Allow only escaped dots? For paths we disallow any regex meta.
        // Exception: none — literal paths have no meta.
        return None;
    }
    if !inner.starts_with('/') {
        return None;
    }
    Some(inner.to_string())
}

fn is_php_files_match_pattern(pat: &str) -> bool {
    let p = pat.trim().trim_matches('"');
    p.contains("php")
        && (p.contains(r"\.php") || p.contains(".php") || p == r"\.php$" || p.ends_with("php$"))
}

fn render_toml(state: &ParseState) -> Result<String> {
    if state.vhosts.is_empty() {
        bail!("apache importer requires at least one <VirtualHost> block");
    }
    let default_listen = state
        .global_listen
        .clone()
        .unwrap_or_else(|| "0.0.0.0:80".to_string());

    let mut out = String::from("config_version = 1\n\n");
    let mut upstream_i = 0usize;
    let mut fcgi_i = 0usize;

    for (i, vh) in state.vhosts.iter().enumerate() {
        let listen = vh
            .listen_hint
            .clone()
            .unwrap_or_else(|| default_listen.clone());
        let mut names = Vec::new();
        if let Some(sn) = &vh.server_name {
            names.push(sn.clone());
        }
        names.extend(vh.server_alias.iter().cloned());

        let mut route_names: Vec<String> = Vec::new();
        let mut route_blocks = String::new();
        let mut upstream_blocks = String::new();
        let mut fcgi_blocks = String::new();

        // Redirects first (more specific paths).
        for (ri, r) in vh.redirects.iter().enumerate() {
            let name = format!("vh{}_redir{}", i + 1, ri + 1);
            route_names.push(name.clone());
            route_blocks.push_str("[[route]]\n");
            route_blocks.push_str(&format!("name = \"{name}\"\n"));
            route_blocks.push_str(&format!(
                "match = {{ path = \"{}\" }}\n",
                escape_toml(&r.from_path)
            ));
            route_blocks.push_str(&format!(
                "redirect = {{ status = {}, location = \"{}\" }}\n\n",
                r.status,
                escape_toml(&r.location)
            ));
        }

        // NG-02 patterned rewrites (match.regex + rewrite rule / when).
        for (wi, rw) in vh.rewrites.iter().enumerate() {
            let name = format!("vh{}_rw{}", i + 1, wi + 1);
            route_names.push(name.clone());
            route_blocks.push_str("[[route]]\n");
            route_blocks.push_str(&format!("name = \"{name}\"\n"));
            route_blocks.push_str(&format!(
                "match = {{ regex = \"{}\" }}\n",
                escape_toml(&rw.match_regex)
            ));
            route_blocks.push_str(&format_rewrite_toml(rw));
            route_blocks.push('\n');
        }

        // Aliases.
        for (ai, a) in vh.aliases.iter().enumerate() {
            let name = format!("vh{}_alias{}", i + 1, ai + 1);
            route_names.push(name.clone());
            route_blocks.push_str("[[route]]\n");
            route_blocks.push_str(&format!("name = \"{name}\"\n"));
            route_blocks.push_str(&format!(
                "match = {{ path = \"{}\" }}\n",
                escape_toml(&a.url_path)
            ));
            route_blocks.push_str(&format!("root = \"{}\"\n", escape_toml(&a.fs_path)));
            if vh.expires_active {
                emit_cache_headers_toml(&mut route_blocks, &vh.expires_by_mime);
            }
            route_blocks.push('\n');
        }

        // ProxyPass routes.
        for p in &vh.proxies {
            upstream_i += 1;
            let uname = format!("backend{upstream_i}");
            let rname = format!("vh{}_proxy{}", i + 1, upstream_i);
            route_names.push(rname.clone());
            let path = if p.path_prefix.is_empty() {
                "/".to_string()
            } else {
                p.path_prefix.clone()
            };
            upstream_blocks.push_str("[[upstream]]\n");
            upstream_blocks.push_str(&format!("name = \"{uname}\"\n"));
            upstream_blocks.push_str(&format!("target = \"{}\"\n", escape_toml(&p.target)));
            upstream_blocks.push_str("timeout_ms = 30000\n\n");
            route_blocks.push_str("[[route]]\n");
            route_blocks.push_str(&format!("name = \"{rname}\"\n"));
            route_blocks.push_str(&format!(
                "match = {{ path = \"{}\" }}\n",
                escape_toml(&path)
            ));
            route_blocks.push_str(&format!("upstream = \"{uname}\"\n\n"));
        }

        // PHP FastCGI (primary site route when present).
        if let Some(php) = &vh.php {
            fcgi_i += 1;
            let pname = format!("php{fcgi_i}");
            let rname = format!("vh{}_php", i + 1);
            route_names.push(rname.clone());
            let docroot = vh
                .document_root
                .clone()
                .unwrap_or_else(|| "/var/www/html".into());
            fcgi_blocks.push_str("[[fcgi_pool]]\n");
            fcgi_blocks.push_str(&format!("name = \"{pname}\"\n"));
            fcgi_blocks.push_str(&format!("address = \"{}\"\n", escape_toml(&php.address)));
            fcgi_blocks.push_str(&format!("transport = \"{}\"\n", php.transport));
            fcgi_blocks.push_str(&format!("document_root = \"{}\"\n", escape_toml(&docroot)));
            fcgi_blocks.push_str("max_concurrency = 16\n");
            fcgi_blocks.push_str("idle_timeout_ms = 30000\n");
            fcgi_blocks.push_str("total_timeout_ms = 30000\n");
            fcgi_blocks.push_str("checkout_timeout_ms = 5000\n\n");
            route_blocks.push_str("[[route]]\n");
            route_blocks.push_str(&format!("name = \"{rname}\"\n"));
            route_blocks.push_str("match = { path = \"/\" }\n");
            route_blocks.push_str(&format!("fastcgi = \"{pname}\"\n"));
            emit_response_headers_toml(&mut route_blocks, &vh.response_headers);
            route_blocks.push('\n');
        } else if let Some(root) = &vh.document_root {
            // Static site root when not PHP.
            let has_root_proxy = vh.proxies.iter().any(|p| p.path_prefix == "/");
            if !has_root_proxy {
                let rname = format!("vh{}_site", i + 1);
                route_names.push(rname.clone());
                route_blocks.push_str("[[route]]\n");
                route_blocks.push_str(&format!("name = \"{rname}\"\n"));
                route_blocks.push_str("match = { path = \"/\" }\n");
                route_blocks.push_str(&format!("root = \"{}\"\n", escape_toml(root)));
                if let Some(idx) = &vh.directory_index {
                    route_blocks.push_str(&format!("index = \"{}\"\n", escape_toml(idx)));
                }
                if vh.expires_active {
                    emit_cache_headers_toml(&mut route_blocks, &vh.expires_by_mime);
                }
                emit_response_headers_toml(&mut route_blocks, &vh.response_headers);
                route_blocks.push('\n');
            }
        }

        if route_names.is_empty() {
            bail!(
                "VirtualHost at line {}: no mappable routes (DocumentRoot, ProxyPass, or PHP)",
                vh.open_line
            );
        }

        out.push_str("[[server]]\n");
        out.push_str(&format!("listen = \"{}\"\n", escape_toml(&listen)));
        if names.len() == 1 {
            out.push_str(&format!("server_name = \"{}\"\n", escape_toml(&names[0])));
        } else if names.len() > 1 {
            out.push_str("server_name = [");
            for (j, n) in names.iter().enumerate() {
                if j > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("\"{}\"", escape_toml(n)));
            }
            out.push_str("]\n");
        }
        if vh.tls.engine {
            let cert = vh.tls.cert.as_deref().unwrap();
            let key = vh.tls.key.as_deref().unwrap();
            out.push_str(&format!(
                "tls = {{ cert = \"{}\", key = \"{}\" }}\n",
                escape_toml(cert),
                escape_toml(key)
            ));
        }
        out.push_str("routes = [");
        for (j, n) in route_names.iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("\"{n}\""));
        }
        out.push_str("]\n");
        if !vh.error_pages.is_empty() {
            out.push_str("error_pages = { ");
            for (j, (code, path)) in vh.error_pages.iter().enumerate() {
                if j > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("\"{code}\" = \"{}\"", escape_toml(path)));
            }
            out.push_str(" }\n");
        }
        out.push('\n');
        out.push_str(&route_blocks);
        out.push_str(&upstream_blocks);
        out.push_str(&fcgi_blocks);
    }
    Ok(out)
}

fn reject(
    state: &mut ParseState,
    line: usize,
    directive: impl Into<String>,
    detail: impl Into<String>,
) {
    state.rejected.push(Rejected {
        file: state.current_file.clone(),
        line,
        directive: directive.into(),
        detail: detail.into(),
    });
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    for (i, &b) in line.as_bytes().iter().enumerate() {
        match b {
            b'"' => in_quote = !in_quote,
            b'#' if !in_quote => return &line[..i],
            _ => {}
        }
    }
    line
}

fn split_directive(line: &str) -> (&str, &str) {
    let mut parts = line.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let args = parts.next().unwrap_or("").trim();
    (name, args)
}

fn first_token(args: &str) -> &str {
    unquote(args.split_whitespace().next().unwrap_or(""))
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn normalize_listen(args: &str) -> String {
    let tok = first_token(args);
    if tok.contains(':') {
        if tok.starts_with(':') {
            format!("0.0.0.0{tok}")
        } else {
            tok.to_string()
        }
    } else {
        format!("0.0.0.0:{tok}")
    }
}

fn normalize_vhost_addr(addr: &str) -> String {
    let addr = addr.trim();
    if let Some(port) = addr.strip_prefix("*:") {
        return format!("0.0.0.0:{port}");
    }
    if addr.chars().all(|c| c.is_ascii_digit()) {
        return format!("0.0.0.0:{addr}");
    }
    if !addr.contains(':') {
        return "0.0.0.0:80".to_string();
    }
    addr.to_string()
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn format_rewrite_toml(rw: &RewriteDraft) -> String {
    let mut s = format!(
        "rewrite = {{ pattern = \"{}\", replacement = \"{}\", flag = \"{}\"",
        escape_toml(&rw.pattern),
        escape_toml(&rw.replacement),
        rw.flag
    );
    if !rw.when.is_empty() {
        s.push_str(", when = [");
        for (i, w) in rw.when.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format_when_toml(w));
        }
        s.push(']');
    }
    s.push_str(" }\n");
    s
}

fn format_when_toml(w: &WhenDraft) -> String {
    let mut parts = Vec::new();
    if let Some(path) = &w.path {
        parts.push(format!("path = \"{}\"", escape_toml(path)));
    }
    if let Some(host) = &w.host {
        parts.push(format!("host = \"{}\"", escape_toml(host)));
    }
    if let Some(query) = &w.query {
        parts.push(format!("query = \"{}\"", escape_toml(query)));
    }
    if let Some(header) = &w.header {
        parts.push(format!("header = \"{}\"", escape_toml(header)));
    }
    if let Some(hv) = &w.header_value {
        parts.push(format!("header_value = \"{}\"", escape_toml(hv)));
    }
    format!("{{ {} }}", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_static_vhost() {
        let src = r#"
Listen 8080
<VirtualHost *:8080>
    ServerName example.com
    DocumentRoot "/var/www/html"
    DirectoryIndex index.html
</VirtualHost>
"#;
        let (toml, report) = migrate(src).unwrap();
        assert!(toml.contains("root = \"/var/www/html\""));
        assert!(report.unsupported.is_empty());
    }

    #[test]
    fn migrates_header_set_to_response_headers() {
        let conf = r#"
Listen 80
<VirtualHost *:80>
  ServerName example.test
  DocumentRoot /var/www
  Header set X-Frame-Options SAMEORIGIN
  Header set X-Content-Type-Options nosniff
</VirtualHost>
"#;
        let (toml, report) = migrate(conf).expect("migrate");
        assert!(
            toml.contains("[[route.response_headers]]"),
            "{toml}"
        );
        assert!(toml.contains("name = \"X-Frame-Options\""), "{toml}");
        assert!(toml.contains("always = true"), "{toml}");
        assert!(
            report.notes.iter().any(|n| n.message.contains("ADR-062")),
            "{:?}",
            report.notes
        );
    }

    #[test]
    fn migrates_expires_by_type_to_cache_headers() {
        let src = r#"
Listen 8080
<VirtualHost *:8080>
    ServerName assets.example.com
    DocumentRoot "/var/www/html"
    ExpiresActive On
    ExpiresByType text/css "access plus 1 month"
    ExpiresByType image/jpeg A86400
    ExpiresDefault "access plus 1 hour"
</VirtualHost>
"#;
        let (toml, report) = migrate(src).unwrap();
        assert!(report.unsupported.is_empty(), "{report:?}");
        assert!(
            toml.contains("[route.cache_headers.by_mime.\"text/css\"]"),
            "{toml}"
        );
        assert!(toml.contains("max_age = 2592000"), "{toml}"); // 30*24*3600
        assert!(
            toml.contains("[route.cache_headers.by_mime.\"image/jpeg\"]"),
            "{toml}"
        );
        assert!(toml.contains("max_age = 86400"), "{toml}");
        assert!(toml.contains("[route.cache_headers.by_mime.\"*\"]"), "{toml}");
        assert!(toml.contains("max_age = 3600"), "{toml}");
    }

    #[test]
    fn expires_modification_rejected() {
        let src = r#"
<VirtualHost *:80>
    ServerName x.example
    DocumentRoot /srv
    ExpiresActive On
    ExpiresByType text/html M3600
</VirtualHost>
"#;
        let (res, report) = migrate_with_report("httpd.conf", src);
        assert!(res.is_err());
        assert!(
            report
                .unsupported
                .iter()
                .any(|u| u.contains("ExpiresByType") || u.contains("modification")),
            "{report:?}"
        );
    }

    #[test]
    fn migrates_redirect_tls_proxy_php() {
        let src = r#"
Listen 443
<VirtualHost *:443>
    ServerName app.example.com
    DocumentRoot /var/www/app
    SSLEngine on
    SSLCertificateFile /etc/ssl/cert.pem
    SSLCertificateKeyFile /etc/ssl/key.pem
    Redirect permanent /old /new
    Alias /assets /var/www/assets
    ProxyPass /api http://127.0.0.1:9000/
    RewriteEngine On
    RewriteRule ^/legacy$ /modern [R=301,L]
    <FilesMatch \.php$>
        SetHandler "proxy:unix:/run/php/php-fpm.sock|fcgi://localhost"
    </FilesMatch>
</VirtualHost>
"#;
        let (toml, report) = migrate(src).unwrap();
        assert!(report.unsupported.is_empty(), "{report:?}");
        assert!(toml.contains("tls = {"));
        assert!(toml.contains("redirect = { status = 301"));
        assert!(toml.contains("root = \"/var/www/assets\""));
        assert!(toml.contains("upstream = \"backend1\""));
        assert!(toml.contains("target = \"http://127.0.0.1:9000\""));
        assert!(toml.contains("fastcgi = \"php1\""));
        assert!(toml.contains("transport = \"unix\""));
        assert!(toml.contains("/legacy"));
    }

    #[test]
    fn still_rejects_auth_maps_errordocument() {
        let src = r#"
<VirtualHost *:80>
    ServerName a.test
    DocumentRoot /srv
    ErrorDocument 404 /404.html
    AuthType Basic
</VirtualHost>
"#;
        let (res, report) = migrate_with_report("httpd.conf", src);
        assert!(res.is_err());
        assert!(report.unsupported.iter().any(|u| u.contains("AuthType")));
        // ErrorDocument alone is mapped; AuthType still refuses the file.
        assert!(
            !report
                .unsupported
                .iter()
                .any(|u| u.contains("ErrorDocument")),
            "{report:?}"
        );
    }

    #[test]
    fn migrates_errordocument_404() {
        let src = r#"
<VirtualHost *:80>
    ServerName err.example
    DocumentRoot /var/www
    ErrorDocument 404 /404.html
    ErrorDocument 500 /50x.html
</VirtualHost>
"#;
        let (toml, report) = migrate(src).unwrap();
        assert!(report.unsupported.is_empty(), "{report:?}");
        assert!(
            toml.contains("error_pages")
                && toml.contains("\"404\" = \"/404.html\"")
                && toml.contains("\"500\" = \"/50x.html\""),
            "{toml}"
        );
    }

    #[test]
    fn rejects_errordocument_url() {
        let src = r#"
<VirtualHost *:80>
    ServerName a.test
    DocumentRoot /srv
    ErrorDocument 404 http://example.com/404
</VirtualHost>
"#;
        let (res, report) = migrate_with_report("httpd.conf", src);
        assert!(res.is_err());
        assert!(report
            .unsupported
            .iter()
            .any(|u| u.contains("ErrorDocument")));
    }

    #[test]
    fn migrates_rewrite_captures_and_cond() {
        let src = r#"
<VirtualHost *:80>
    ServerName blog.example.com
    DocumentRoot /var/www/blog
    RewriteEngine On
    RewriteCond %{HTTP_HOST} ^blog\.example\.com$ [NC]
    RewriteCond %{REQUEST_URI} !^/index\.php
    RewriteRule ^/blog/(.*)$ /index.php?q=$1 [L]
</VirtualHost>
"#;
        // Negation cond must fail-closed.
        let (res, report) = migrate_with_report("blog.conf", src);
        assert!(res.is_err(), "negation RewriteCond must refuse");
        assert!(
            report.unsupported.iter().any(|u| u.contains("RewriteCond")),
            "{report:?}"
        );

        let src_ok = r#"
<VirtualHost *:80>
    ServerName blog.example.com
    DocumentRoot /var/www/blog
    RewriteEngine On
    RewriteCond %{HTTP_HOST} =blog.example.com
    RewriteCond %{QUERY_STRING} ^utm_
    RewriteRule ^/blog/(.*)$ /index.php?q=$1 [L]
    RewriteRule ^/old/(.*)$ /new/$1 [R=301,L]
</VirtualHost>
"#;
        let (toml, report) = migrate(src_ok).unwrap();
        assert!(report.unsupported.is_empty(), "{report:?}");
        assert!(
            toml.contains("match = { regex = \"^/blog/(.*)$\" }"),
            "expected regex route: {toml}"
        );
        assert!(
            toml.contains("flag = \"last\""),
            "expected last flag: {toml}"
        );
        assert!(
            toml.contains("replacement = \"/index.php?q=$1\""),
            "expected capture replacement: {toml}"
        );
        assert!(
            toml.contains("host = \"blog.example.com\""),
            "expected when.host: {toml}"
        );
        assert!(
            toml.contains("query = \"^utm_\""),
            "expected when.query: {toml}"
        );
        assert!(
            toml.contains("flag = \"permanent\""),
            "expected permanent redirect rewrite: {toml}"
        );
        assert!(
            toml.contains("match = { regex = \"^/old/(.*)$\" }"),
            "expected capture redirect route: {toml}"
        );
    }

    #[test]
    fn dump_ng02_apache_toml_for_evidence() {
        let Some(path) = std::env::var_os("EXYONQ_DUMP_APACHE_TOML") else {
            return;
        };
        let src = r#"
Listen 80
<VirtualHost *:80>
    ServerName blog.example.com
    DocumentRoot /var/www/blog
    RewriteEngine On
    RewriteCond %{HTTP_HOST} =blog.example.com
    RewriteCond %{QUERY_STRING} ^utm_
    RewriteRule ^/blog/(.*)$ /index.php?q=$1 [L]
    RewriteRule ^/old/(.*)$ /new/$1 [R=301,L]
    RewriteRule ^/legacy$ /modern [R=301,L]
</VirtualHost>
"#;
        let (toml, report) = migrate(src).expect("migrate");
        assert!(report.unsupported.is_empty(), "{report:?}");
        std::fs::write(&path, &toml).expect("write dump");
    }

    #[test]
    fn rejects_rewrite_cond_without_rule() {
        let src = r#"
<VirtualHost *:80>
    ServerName a.test
    DocumentRoot /srv
    RewriteEngine On
    RewriteCond %{REQUEST_URI} ^/admin
</VirtualHost>
"#;
        let (res, report) = migrate_with_report("orphan.conf", src);
        assert!(res.is_err());
        assert!(
            report
                .unsupported
                .iter()
                .any(|u| u.contains("without a following RewriteRule")),
            "{report:?}"
        );
    }

    #[test]
    fn rejects_proxypass_with_upstream_path() {
        let src = r#"
<VirtualHost *:80>
    ServerName a.test
    DocumentRoot /srv
    ProxyPass /api http://127.0.0.1:9000/v1/
</VirtualHost>
"#;
        let (res, report) = migrate_with_report("x.conf", src);
        assert!(res.is_err());
        assert!(report.unsupported.iter().any(|u| u.contains("ProxyPass")));
    }

    #[test]
    fn string_include_rejects_exy_0001() {
        let src = r#"
Include conf.d/extra.conf
<VirtualHost *:80>
    ServerName a.test
    DocumentRoot /srv
</VirtualHost>
"#;
        let (res, report) = migrate_with_report("httpd.conf", src);
        assert!(res.is_err());
        assert!(
            report
                .unsupported
                .iter()
                .any(|u| u.contains(EXY_AP_INCLUDE_0001) && u.contains("Include")),
            "{report:?}"
        );
    }

    #[test]
    fn path_include_nested_migrates() {
        let root = std::env::temp_dir().join(format!("exyonq-ap07-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("conf.d")).unwrap();
        std::fs::write(
            root.join("httpd.conf"),
            r#"
Listen 8080
Include conf.d/vhost.conf
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("conf.d/vhost.conf"),
            r#"
<VirtualHost *:8080>
    ServerName nest.example
    DocumentRoot /var/www/nest
</VirtualHost>
"#,
        )
        .unwrap();
        let (toml, report) = migrate_path(&root.join("httpd.conf")).expect("migrate");
        assert!(report.unsupported.is_empty(), "{report:?}");
        assert!(toml.contains("nest.example"), "{toml}");
        assert!(toml.contains("/var/www/nest"), "{toml}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn path_include_missing_rejects_0002() {
        let root = std::env::temp_dir().join(format!("exyonq-ap07-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("httpd.conf"),
            "Include missing.conf\nListen 80\n",
        )
        .unwrap();
        let (res, report) = migrate_path_with_report(&root.join("httpd.conf"));
        assert!(res.is_err());
        assert!(
            report
                .unsupported
                .iter()
                .any(|u| u.contains("EXY-AP-INCLUDE-0002")),
            "{report:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn path_include_optional_missing_notes() {
        let root = std::env::temp_dir().join(format!("exyonq-ap07-opt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("httpd.conf"),
            r#"
IncludeOptional missing-optional.conf
Listen 8080
<VirtualHost *:8080>
    ServerName opt.example
    DocumentRoot /srv
</VirtualHost>
"#,
        )
        .unwrap();
        let (toml, report) = migrate_path(&root.join("httpd.conf")).expect("migrate");
        assert!(report.unsupported.is_empty(), "{report:?}");
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.message.contains("EXY-AP-INCLUDE-0006")),
            "{report:?}"
        );
        assert!(toml.contains("opt.example"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn path_include_cycle_rejects_0003() {
        let root = std::env::temp_dir().join(format!("exyonq-ap07-cycle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.conf"), "Include b.conf\n").unwrap();
        std::fs::write(root.join("b.conf"), "Include a.conf\n").unwrap();
        let (res, report) = migrate_path_with_report(&root.join("a.conf"));
        assert!(res.is_err());
        assert!(
            report
                .unsupported
                .iter()
                .any(|u| u.contains("EXY-AP-INCLUDE-0003")),
            "{report:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
