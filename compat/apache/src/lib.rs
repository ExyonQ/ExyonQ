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

use anyhow::{bail, Result};
use exyonq_compat_common::{CompatLevel, CompatNote, MigrationReport};

pub const IMPORTER_NAME: &str = "apache";
pub const IMPORTER_VERSION: &str = "0.4.13";

#[derive(Debug, Clone)]
struct Rejected {
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
    aliases: Vec<AliasDraft>,
    proxies: Vec<ProxyDraft>,
    php: Option<PhpDraft>,
    rewrite_engine: bool,
    /// Inside `<FilesMatch \.php$>` (or similar).
    in_php_files_match: bool,
}

#[derive(Debug, Default)]
struct ParseState {
    global_listen: Option<String>,
    vhosts: Vec<VirtualHostDraft>,
    current: Option<VirtualHostDraft>,
    rejected: Vec<Rejected>,
    notes: Vec<CompatNote>,
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

/// Always returns a report (including on refusal).
pub fn migrate_with_report(label: &str, input: &str) -> (Result<String>, MigrationReport) {
    let mut state = ParseState::default();
    if let Err(e) = parse_httpd(input, &mut state) {
        return (
            Err(e),
            MigrationReport {
                source: IMPORTER_NAME.to_string(),
                notes: state.notes,
                unsupported: rejected_list(label, &state.rejected),
            },
        );
    }

    let unsupported = rejected_list(label, &state.rejected);
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

fn rejected_list(label: &str, rejected: &[Rejected]) -> Vec<String> {
    rejected
        .iter()
        .map(|r| {
            format!(
                "REJECTED_UNSUPPORTED {}:{} `{}` — {}",
                label, r.line, r.directive, r.detail
            )
        })
        .collect()
}

fn parse_httpd(input: &str, state: &mut ParseState) -> Result<()> {
    for (idx, raw_line) in input.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
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

    if state.current.is_some() {
        bail!("unclosed <VirtualHost> at end of file");
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
    if draft.document_root.is_none() && draft.php.is_none() && !has_root_proxy {
        reject(
            state,
            draft.open_line,
            "DocumentRoot",
            "required unless PHP-FPM or ProxyPass / is configured",
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
        match parse_safe_rewrite_redirect(args) {
            Ok(r) => {
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
            Err(msg) => reject(state, line, name, msg),
        }
        return;
    }
    if eq_ci(name, "RewriteCond") {
        reject(
            state,
            line,
            name,
            "RewriteCond not mapped; use ExyonQ htaccess overlay for front-controller conditions",
        );
        return;
    }
    if eq_ci(name, "ErrorDocument") {
        reject(state, line, name, "no ExyonQ IR for custom error pages yet");
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

fn parse_safe_rewrite_redirect(args: &str) -> Result<RedirectDraft, String> {
    // RewriteRule ^/old$ /new [R=301,L]
    let mut parts = args.split_whitespace();
    let pattern = parts.next().ok_or("RewriteRule requires pattern")?;
    let replacement = parts.next().ok_or("RewriteRule requires substitution")?;
    let flags = parts.next().unwrap_or("");
    let status = parse_r_flag(flags).ok_or_else(|| {
        "only RewriteRule with external redirect flags [R] or [R=301|302,L] and literal ^/path$ are mapped"
            .to_string()
    })?;
    let from = literal_path_from_pattern(pattern).ok_or_else(|| {
        "RewriteRule pattern must be literal ^/path$ without captures".to_string()
    })?;
    if replacement.contains('$') || replacement.contains('%') {
        return Err("RewriteRule substitution captures not supported".into());
    }
    Ok(RedirectDraft {
        status,
        from_path: from,
        location: unquote(replacement).to_string(),
    })
}

fn parse_r_flag(flags: &str) -> Option<u16> {
    let f = flags.trim().trim_start_matches('[').trim_end_matches(']');
    if f.is_empty() {
        return None;
    }
    let mut status = None;
    for part in f.split(',') {
        let p = part.trim();
        if p.eq_ignore_ascii_case("R") || p.eq_ignore_ascii_case("redirect") {
            status = Some(status.unwrap_or(302));
        } else if let Some(code) = p
            .strip_prefix("R=")
            .or_else(|| p.strip_prefix("r="))
            .or_else(|| p.strip_prefix("redirect="))
        {
            status = code.parse().ok();
        } else if p.eq_ignore_ascii_case("L")
            || p.eq_ignore_ascii_case("last")
            || p.eq_ignore_ascii_case("NC")
        {
            continue;
        } else {
            return None;
        }
    }
    match status {
        Some(s) if matches!(s, 301 | 302 | 303 | 307 | 308) => Some(s),
        _ => None,
    }
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
            route_blocks.push_str(&format!("root = \"{}\"\n\n", escape_toml(&a.fs_path)));
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
            route_blocks.push_str(&format!("fastcgi = \"{pname}\"\n\n"));
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
        out.push_str("]\n\n");
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
    fn still_rejects_auth_and_errordocument() {
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
        assert!(report
            .unsupported
            .iter()
            .any(|u| u.contains("ErrorDocument")));
        assert!(report.unsupported.iter().any(|u| u.contains("AuthType")));
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
}
