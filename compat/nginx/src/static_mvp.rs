//! Fail-closed static-MVP profile gate for NGINX → ExyonQ import.
//!
//! Opt-in only. Default Tier1/2 migrate behavior is unchanged.

use crate::ast::{AnalyzedConfig, Config, Directive, LocationKind};
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};

/// Directives allowed at server (or inside `location /`) for static-mvp.
const SERVER_ALLOWED: &[&str] = &["listen", "server_name", "root", "index", "location"];

/// Directives allowed inside `location /` only.
const LOCATION_ALLOWED: &[&str] = &["root", "index"];

/// Hard-reject names (always Error under static-mvp when present anywhere).
const HARD_REJECT: &[&str] = &[
    "proxy_pass",
    "fastcgi_pass",
    "uwsgi_pass",
    "grpc_pass",
    "rewrite",
    "return",
    "if",
    "map",
    "upstream",
    "split_clients",
    "include",
    "ssl_certificate",
    "ssl_certificate_key",
    "listen_http2",
    "http2",
    "quic",
    "limit_req",
    "auth_basic",
    "add_header",
    "try_files",
    "set",
    "lua",
    "content_by_lua",
    "content_by_lua_block",
];

/// Apply static-mvp allowlist / reject rules onto `report`.
///
/// Call after `analyze`. Any violation is recorded as [`CompatStatus::Error`].
pub fn enforce(config: &Config, analyzed: &AnalyzedConfig, report: &mut CompatibilityReport) {
    for directive in &config.directives {
        walk_top(directive, report);
    }

    if analyzed.servers.is_empty() {
        report.push(
            &SourceLoc::unlocated("static-mvp"),
            "server",
            CompatStatus::Error,
            "static-mvp requires at least one server block",
            None,
        );
        return;
    }

    for server in &analyzed.servers {
        let mut has_listen = false;
        let mut has_root = false;
        let mut listen_count = 0usize;
        let mut root_count = 0usize;
        let mut index_count = 0usize;
        for d in &server.directives {
            match d.name.as_str() {
                "listen" => {
                    listen_count += 1;
                    has_listen = true;
                    validate_listen(d, report);
                }
                "root" => {
                    root_count += 1;
                    has_root = true;
                    validate_single_path_arg(d, "root", report);
                }
                "server_name" => validate_server_name(d, report),
                "index" => {
                    index_count += 1;
                    validate_index(d, report);
                }
                other => {
                    if !SERVER_ALLOWED.contains(&other) {
                        reject(
                            &d.loc,
                            other,
                            format!("static-mvp rejects server directive `{other}`"),
                            report,
                        );
                    }
                }
            }
        }
        if listen_count > 1 {
            reject(
                &server.loc,
                "listen",
                "static-mvp allows exactly one `listen` per server",
                report,
            );
        }
        if root_count > 1 {
            reject(
                &server.loc,
                "root",
                "static-mvp allows at most one `root` at server level",
                report,
            );
        }
        if index_count > 1 {
            reject(
                &server.loc,
                "index",
                "static-mvp allows at most one `index` at server level",
                report,
            );
        }

        if !server.locations.is_empty() {
            if server.locations.len() > 1 {
                reject(
                    &server.loc,
                    "location",
                    "static-mvp allows at most one location block",
                    report,
                );
            }
            for loc in &server.locations {
                let is_root_prefix = matches!(loc.kind, LocationKind::Prefix)
                    && (loc.raw_pattern == "/"
                        || loc.normalized_literal_path.as_deref() == Some("/"));
                if !is_root_prefix {
                    reject(
                        &loc.loc,
                        "location",
                        format!(
                            "static-mvp only allows `location /` (got {} `{}`)",
                            loc.kind.as_str(),
                            loc.raw_pattern
                        ),
                        report,
                    );
                }
                let mut loc_root = 0usize;
                let mut loc_index = 0usize;
                for d in &loc.directives {
                    if d.name == "root" {
                        loc_root += 1;
                        has_root = true;
                        validate_single_path_arg(d, "root", report);
                    }
                    if d.name == "index" {
                        loc_index += 1;
                        validate_index(d, report);
                    }
                    if !LOCATION_ALLOWED.contains(&d.name.as_str()) {
                        reject(
                            &d.loc,
                            &d.name,
                            format!("static-mvp rejects directive `{}` inside location", d.name),
                            report,
                        );
                    }
                    if HARD_REJECT.contains(&d.name.as_str()) {
                        reject(
                            &d.loc,
                            &d.name,
                            format!("static-mvp rejects `{}`", d.name),
                            report,
                        );
                    }
                }
                if loc_root > 1 {
                    reject(
                        &loc.loc,
                        "root",
                        "static-mvp allows at most one `root` inside location /",
                        report,
                    );
                }
                if loc_index > 1 {
                    reject(
                        &loc.loc,
                        "index",
                        "static-mvp allows at most one `index` inside location /",
                        report,
                    );
                }
            }
        }

        if !has_listen {
            reject(
                &server.loc,
                "listen",
                "static-mvp requires `listen` in each server",
                report,
            );
        }
        if !has_root {
            reject(
                &server.loc,
                "root",
                "static-mvp requires `root` (server or location /)",
                report,
            );
        }
    }

    if !analyzed.upstreams.is_empty() {
        for u in &analyzed.upstreams {
            reject(
                &u.loc,
                "upstream",
                format!("static-mvp rejects upstream `{}`", u.name),
                report,
            );
        }
    }
}

/// True when the profile must refuse to emit IR.
pub fn must_refuse(report: &CompatibilityReport) -> bool {
    report.has_errors() || report.has_partial_or_unsupported()
}

fn walk_top(directive: &Directive, report: &mut CompatibilityReport) {
    match directive.name.as_str() {
        "http" | "main" => {
            if let Some(block) = &directive.block {
                for child in &block.directives {
                    walk_http(child, report);
                }
            }
        }
        "server" => walk_server_block(directive, report),
        "events" => reject(
            &directive.loc,
            "events",
            "static-mvp rejects `events` (out of accept set)",
            report,
        ),
        "upstream" => reject(
            &directive.loc,
            "upstream",
            "static-mvp rejects `upstream`",
            report,
        ),
        "include" => reject(
            &directive.loc,
            "include",
            "static-mvp rejects `include`",
            report,
        ),
        other if HARD_REJECT.contains(&other) => {
            reject(
                &directive.loc,
                other,
                format!("static-mvp rejects `{other}`"),
                report,
            );
        }
        other => reject(
            &directive.loc,
            other,
            format!("static-mvp rejects unknown/unsupported top-level `{other}`"),
            report,
        ),
    }
}

fn walk_http(directive: &Directive, report: &mut CompatibilityReport) {
    match directive.name.as_str() {
        "server" => walk_server_block(directive, report),
        "include" => reject(
            &directive.loc,
            "include",
            "static-mvp rejects `include`",
            report,
        ),
        "upstream" => reject(
            &directive.loc,
            "upstream",
            "static-mvp rejects `upstream`",
            report,
        ),
        other if HARD_REJECT.contains(&other) => {
            reject(
                &directive.loc,
                other,
                format!("static-mvp rejects `{other}`"),
                report,
            );
        }
        other => reject(
            &directive.loc,
            other,
            format!("static-mvp rejects http-context `{other}`"),
            report,
        ),
    }
}

fn walk_server_block(directive: &Directive, report: &mut CompatibilityReport) {
    let Some(block) = &directive.block else {
        return;
    };
    for child in &block.directives {
        let name = child.name.as_str();
        if HARD_REJECT.contains(&name) {
            reject(
                &child.loc,
                name,
                format!("static-mvp rejects `{name}`"),
                report,
            );
            continue;
        }
        if name == "location" {
            continue; // validated via analyzed locations
        }
        if !SERVER_ALLOWED.contains(&name) {
            reject(
                &child.loc,
                name,
                format!("static-mvp rejects unknown server directive `{name}`"),
                report,
            );
        }
    }
}

fn validate_server_name(directive: &Directive, report: &mut CompatibilityReport) {
    for arg in &directive.args {
        if arg == "_" {
            continue;
        }
        if arg.starts_with('~') || arg.starts_with("~*") || arg.contains('*') {
            reject(
                &directive.loc,
                "server_name",
                format!("static-mvp rejects non-literal server_name `{arg}`"),
                report,
            );
        }
    }
}

fn validate_listen(directive: &Directive, report: &mut CompatibilityReport) {
    if directive.args.is_empty() {
        reject(
            &directive.loc,
            "listen",
            "static-mvp requires `listen` address/port",
            report,
        );
        return;
    }
    if directive.args.len() > 1 {
        reject(
            &directive.loc,
            "listen",
            format!(
                "static-mvp rejects listen flags/options (`{}`)",
                directive.args[1..].join(" ")
            ),
            report,
        );
        return;
    }
    let raw = directive.args[0].as_str();
    if !listen_arg_mappable(raw) {
        reject(
            &directive.loc,
            "listen",
            format!(
                "static-mvp rejects unmappable listen `{raw}` (need port, IPv4/IPv6:port, or *:port)"
            ),
            report,
        );
    }
}

/// Accept only forms that map unambiguously under `map_ir::normalize_listen_addr`
/// without falling back to `0.0.0.0:80` or failing AppConfig validation.
fn listen_arg_mappable(raw: &str) -> bool {
    if raw.is_empty() || raw.starts_with("unix:") {
        return false;
    }
    let lower = raw.to_ascii_lowercase();
    if lower == "ssl" || lower == "http2" || lower == "quic" {
        return false;
    }
    if raw.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if raw.starts_with('[') {
        let Some(end) = raw.find(']') else {
            return false;
        };
        let inner = &raw[1..end];
        if inner.parse::<std::net::Ipv6Addr>().is_err() {
            return false;
        }
        let rest = raw[end + 1..].trim_start_matches(':');
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    }
    if let Some((host, port)) = raw.rsplit_once(':') {
        if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if host == "*" {
            return true;
        }
        return host.parse::<std::net::IpAddr>().is_ok();
    }
    // Bare IP / hostname without port — refuse (no silent 0.0.0.0:80 remap).
    false
}

fn validate_index(directive: &Directive, report: &mut CompatibilityReport) {
    if directive.args.is_empty() {
        reject(
            &directive.loc,
            "index",
            "static-mvp `index` requires one filename",
            report,
        );
        return;
    }
    if directive.args.len() > 1 {
        reject(
            &directive.loc,
            "index",
            "static-mvp allows a single `index` filename only",
            report,
        );
    }
}

fn validate_single_path_arg(directive: &Directive, name: &str, report: &mut CompatibilityReport) {
    if directive.args.len() != 1 || directive.args[0].is_empty() {
        reject(
            &directive.loc,
            name,
            format!("static-mvp requires a single non-empty `{name}` path"),
            report,
        );
    }
}

fn reject(
    loc: &SourceLoc,
    directive: &str,
    message: impl Into<String>,
    report: &mut CompatibilityReport,
) {
    report.push(loc, directive, CompatStatus::Error, message, None);
}

#[cfg(test)]
mod tests {
    use crate::{migrate_source, MigrateOptions, MigrateProfile};

    fn opts() -> MigrateOptions {
        MigrateOptions {
            profile: MigrateProfile::StaticMvp,
            ..MigrateOptions::default()
        }
    }

    #[test]
    fn accepts_minimal_static_server() {
        let src = r#"
server {
    listen 8080;
    server_name example.com;
    root /srv/www;
    index index.html;
}
"#;
        let out = migrate_source("accept.conf", src, &opts()).expect("migrate");
        assert!(!out.config.trim().is_empty(), "expected IR");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            0
        );
        assert!(out.config.contains("root"));
        assert!(out.config.contains("/srv/www"));
    }

    #[test]
    fn defaults_index_html_when_absent() {
        let src = r#"
server {
    listen 8080;
    root /srv/www;
}
"#;
        let out = migrate_source("no-index.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            0
        );
        assert!(
            out.config.contains("index = \"index.html\""),
            "expected default index.html in IR: {}",
            out.config
        );
    }

    #[test]
    fn refuses_unmappable_listen_ip_without_port() {
        let src = r#"
server {
    listen 127.0.0.1;
    root /srv/www;
}
"#;
        let out = migrate_source("listen-ip.conf", src, &opts()).expect("migrate");
        assert!(out.config.trim().is_empty());
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            1
        );
    }

    #[test]
    fn refuses_unix_listen() {
        let src = r#"
server {
    listen unix:/tmp/nginx.sock;
    root /srv/www;
}
"#;
        let out = migrate_source("unix.conf", src, &opts()).expect("migrate");
        assert!(out.config.trim().is_empty());
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            1
        );
    }

    #[test]
    fn refuses_hostname_listen() {
        let src = r#"
server {
    listen example.com:8080;
    root /srv/www;
}
"#;
        let out = migrate_source("host.conf", src, &opts()).expect("migrate");
        assert!(out.config.trim().is_empty());
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            1
        );
    }

    #[test]
    fn refuses_duplicate_root_in_location() {
        let src = r#"
server {
    listen 80;
    location / {
        root /a;
        root /b;
    }
}
"#;
        let out = migrate_source("dup-loc.conf", src, &opts()).expect("migrate");
        assert!(out.config.trim().is_empty());
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            1
        );
    }

    #[test]
    fn refuses_proxy_pass() {
        let src = r#"
server {
    listen 80;
    root /srv/www;
    location / {
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
        let out = migrate_source("proxy.conf", src, &opts()).expect("migrate");
        assert!(out.config.trim().is_empty());
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            1
        );
        assert!(out.report.has_errors());
    }

    #[test]
    fn refuses_fastcgi_pass() {
        let src = r#"
server {
    listen 80;
    root /srv/php;
    location / {
        fastcgi_pass unix:/run/php/php-fpm.sock;
    }
}
"#;
        let out = migrate_source("fcgi.conf", src, &opts()).expect("migrate");
        assert!(out.config.trim().is_empty());
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            1
        );
    }

    #[test]
    fn refuses_include() {
        let src = r#"
server {
    listen 80;
    root /srv/www;
    include mime.types;
}
"#;
        let out = migrate_source("inc.conf", src, &opts()).expect("migrate");
        assert!(out.config.trim().is_empty());
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            1
        );
    }

    #[test]
    fn refuses_unknown_directive() {
        let src = r#"
server {
    listen 80;
    root /srv/www;
    gzip on;
}
"#;
        let out = migrate_source("unknown.conf", src, &opts()).expect("migrate");
        assert!(out.config.trim().is_empty());
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            1
        );
    }
}
