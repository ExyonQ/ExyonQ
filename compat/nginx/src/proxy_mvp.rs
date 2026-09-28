//! Fail-closed reverse-proxy MVP profile for NGINX → ExyonQ import.
//!
//! Opt-in only. Does not alter Full or `static-mvp` behavior.

use crate::ast::{AnalyzedConfig, Config, Directive, LocationKind};
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};

const SERVER_ALLOWED: &[&str] = &["listen", "server_name", "location"];
const LOCATION_ALLOWED: &[&str] = &["proxy_pass"];

const HARD_REJECT: &[&str] = &[
    "root",
    "index",
    "alias",
    "try_files",
    "fastcgi_pass",
    "uwsgi_pass",
    "scgi_pass",
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
    "set",
    "lua",
    "content_by_lua",
    "content_by_lua_block",
    "proxy_set_header",
    "proxy_http_version",
    "proxy_buffering",
    "proxy_cache",
    "proxy_read_timeout",
    "proxy_connect_timeout",
    "proxy_send_timeout",
    "proxy_redirect",
    "proxy_ssl_verify",
];

/// Apply reverse-proxy-mvp allowlist / reject rules onto `report`.
pub fn enforce(config: &Config, analyzed: &AnalyzedConfig, report: &mut CompatibilityReport) {
    for directive in &config.directives {
        walk_top(directive, report);
    }

    if analyzed.servers.is_empty() {
        report.push(
            &SourceLoc::unlocated("reverse-proxy-mvp"),
            "server",
            CompatStatus::Error,
            "reverse-proxy-mvp requires at least one server block",
            None,
        );
        return;
    }

    for server in &analyzed.servers {
        let mut has_listen = false;
        let mut listen_count = 0usize;
        for d in &server.directives {
            match d.name.as_str() {
                "listen" => {
                    listen_count += 1;
                    has_listen = true;
                    validate_listen(d, report);
                }
                "server_name" => validate_server_name(d, report),
                other => {
                    if !SERVER_ALLOWED.contains(&other) {
                        reject(
                            &d.loc,
                            other,
                            format!("reverse-proxy-mvp rejects server directive `{other}`"),
                            report,
                        );
                    }
                }
            }
            if HARD_REJECT.contains(&d.name.as_str()) {
                reject(
                    &d.loc,
                    &d.name,
                    format!("reverse-proxy-mvp rejects `{}`", d.name),
                    report,
                );
            }
        }
        if listen_count > 1 {
            reject(
                &server.loc,
                "listen",
                "reverse-proxy-mvp allows exactly one `listen` per server",
                report,
            );
        }
        if !has_listen {
            reject(
                &server.loc,
                "listen",
                "reverse-proxy-mvp requires `listen` in each server",
                report,
            );
        }

        if server.locations.is_empty() {
            reject(
                &server.loc,
                "location",
                "reverse-proxy-mvp requires exactly one `location` with `proxy_pass`",
                report,
            );
            continue;
        }
        if server.locations.len() > 1 {
            reject(
                &server.loc,
                "location",
                "reverse-proxy-mvp allows exactly one location block",
                report,
            );
        }

        for loc in &server.locations {
            match loc.kind {
                LocationKind::Prefix => {}
                other => {
                    reject(
                        &loc.loc,
                        "location",
                        format!(
                            "reverse-proxy-mvp only allows simple prefix location (got {} `{}`)",
                            other.as_str(),
                            loc.raw_pattern
                        ),
                        report,
                    );
                }
            }
            if loc.raw_pattern.starts_with('@') || loc.raw_pattern.starts_with('~') {
                reject(
                    &loc.loc,
                    "location",
                    format!(
                        "reverse-proxy-mvp rejects non-literal location `{}`",
                        loc.raw_pattern
                    ),
                    report,
                );
            }

            let mut proxy_count = 0usize;
            for d in &loc.directives {
                if HARD_REJECT.contains(&d.name.as_str()) {
                    reject(
                        &d.loc,
                        &d.name,
                        format!("reverse-proxy-mvp rejects `{}`", d.name),
                        report,
                    );
                }
                if !LOCATION_ALLOWED.contains(&d.name.as_str()) {
                    reject(
                        &d.loc,
                        &d.name,
                        format!(
                            "reverse-proxy-mvp rejects directive `{}` inside location",
                            d.name
                        ),
                        report,
                    );
                }
                if d.name == "proxy_pass" {
                    proxy_count += 1;
                    validate_proxy_pass(d, report);
                }
            }
            if proxy_count == 0 {
                reject(
                    &loc.loc,
                    "proxy_pass",
                    "reverse-proxy-mvp location requires `proxy_pass http://host:port`",
                    report,
                );
            }
            if proxy_count > 1 {
                reject(
                    &loc.loc,
                    "proxy_pass",
                    "reverse-proxy-mvp allows exactly one `proxy_pass` per location",
                    report,
                );
            }
        }
    }

    if !analyzed.upstreams.is_empty() {
        for u in &analyzed.upstreams {
            reject(
                &u.loc,
                "upstream",
                format!("reverse-proxy-mvp rejects upstream block `{}`", u.name),
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
            "reverse-proxy-mvp rejects `events` (out of accept set)",
            report,
        ),
        "upstream" => reject(
            &directive.loc,
            "upstream",
            "reverse-proxy-mvp rejects `upstream`",
            report,
        ),
        "include" => reject(
            &directive.loc,
            "include",
            "reverse-proxy-mvp rejects `include`",
            report,
        ),
        other => {
            if HARD_REJECT.contains(&other) || !matches!(other, "http" | "main" | "server") {
                reject(
                    &directive.loc,
                    other,
                    format!("reverse-proxy-mvp rejects top-level `{other}`"),
                    report,
                );
            }
        }
    }
}

fn walk_http(directive: &Directive, report: &mut CompatibilityReport) {
    match directive.name.as_str() {
        "server" => walk_server_block(directive, report),
        "upstream" => reject(
            &directive.loc,
            "upstream",
            "reverse-proxy-mvp rejects `upstream`",
            report,
        ),
        "include" => reject(
            &directive.loc,
            "include",
            "reverse-proxy-mvp rejects `include`",
            report,
        ),
        other => reject(
            &directive.loc,
            other,
            format!("reverse-proxy-mvp rejects http-context `{other}`"),
            report,
        ),
    }
}

fn walk_server_block(directive: &Directive, report: &mut CompatibilityReport) {
    let Some(block) = &directive.block else {
        reject(
            &directive.loc,
            "server",
            "reverse-proxy-mvp requires a non-empty server block",
            report,
        );
        return;
    };
    for child in &block.directives {
        if child.name == "location" {
            if let Some(inner) = &child.block {
                for nested in &inner.directives {
                    if nested.name == "location" {
                        reject(
                            &nested.loc,
                            "location",
                            "reverse-proxy-mvp rejects nested location",
                            report,
                        );
                    }
                }
            }
        }
    }
}

fn validate_listen(directive: &Directive, report: &mut CompatibilityReport) {
    if directive.args.is_empty() {
        reject(
            &directive.loc,
            "listen",
            "reverse-proxy-mvp requires `listen` address/port",
            report,
        );
        return;
    }
    if directive.args.len() > 1 {
        reject(
            &directive.loc,
            "listen",
            format!(
                "reverse-proxy-mvp rejects listen flags/options (`{}`)",
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
                "reverse-proxy-mvp rejects unmappable listen `{raw}` (need port, IPv4/IPv6:port, or *:port)"
            ),
            report,
        );
    }
}

fn listen_arg_mappable(raw: &str) -> bool {
    if raw.is_empty() || raw.starts_with("unix:") {
        return false;
    }
    let lower = raw.to_ascii_lowercase();
    if lower == "ssl" || lower == "http2" || lower == "quic" {
        return false;
    }
    // Bare port → map_ir normalizes to 0.0.0.0:port (faithful).
    if raw.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if let Some((host, port)) = raw.rsplit_once(':') {
        if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        // Only forms normalize_listen_addr maps without silent fallback.
        if host == "*" || host == "0.0.0.0" || host == "127.0.0.1" {
            return true;
        }
        if host.starts_with('[') && host.ends_with(']') {
            let inner = &host[1..host.len() - 1];
            return inner.parse::<std::net::Ipv6Addr>().is_ok();
        }
        if host.parse::<std::net::Ipv4Addr>().is_ok() {
            return true;
        }
    }
    false
}

fn validate_server_name(directive: &Directive, report: &mut CompatibilityReport) {
    if directive.args.is_empty() {
        reject(
            &directive.loc,
            "server_name",
            "reverse-proxy-mvp `server_name` requires at least one literal name",
            report,
        );
        return;
    }
    for name in &directive.args {
        if name.starts_with('~') || name.contains('*') {
            if name == "_" {
                continue;
            }
            reject(
                &directive.loc,
                "server_name",
                format!("reverse-proxy-mvp rejects non-literal server_name `{name}`"),
                report,
            );
        }
    }
}

fn validate_proxy_pass(directive: &Directive, report: &mut CompatibilityReport) {
    if directive.args.len() != 1 {
        reject(
            &directive.loc,
            "proxy_pass",
            "reverse-proxy-mvp requires a single `proxy_pass` argument",
            report,
        );
        return;
    }
    let raw = directive.args[0].as_str();
    if raw.contains('$') {
        reject(
            &directive.loc,
            "proxy_pass",
            "reverse-proxy-mvp rejects variables in `proxy_pass`",
            report,
        );
        return;
    }
    if raw.starts_with("https://") {
        reject(
            &directive.loc,
            "proxy_pass",
            "reverse-proxy-mvp rejects https upstreams",
            report,
        );
        return;
    }
    if raw.contains("unix:") {
        reject(
            &directive.loc,
            "proxy_pass",
            "reverse-proxy-mvp rejects unix socket upstreams",
            report,
        );
        return;
    }
    if !raw.starts_with("http://") {
        reject(
            &directive.loc,
            "proxy_pass",
            format!("reverse-proxy-mvp requires `proxy_pass http://host:port` (got `{raw}`)"),
            report,
        );
        return;
    }

    let authority = raw.trim_start_matches("http://");
    // Allow optional trailing slash on authority only: http://host:port/
    let authority = authority.strip_suffix('/').unwrap_or(authority);
    if authority.contains('/') {
        reject(
            &directive.loc,
            "proxy_pass",
            "reverse-proxy-mvp rejects URI path in `proxy_pass` (authority only)",
            report,
        );
        return;
    }
    if authority.is_empty() {
        reject(
            &directive.loc,
            "proxy_pass",
            "reverse-proxy-mvp rejects empty `proxy_pass` authority",
            report,
        );
        return;
    }
    if is_named_upstream_ref(authority) {
        reject(
            &directive.loc,
            "proxy_pass",
            format!(
                "reverse-proxy-mvp rejects named upstream `{authority}` (inline http://host:port only)"
            ),
            report,
        );
        return;
    }
    if !authority_looks_like_host_port(authority) {
        reject(
            &directive.loc,
            "proxy_pass",
            format!("reverse-proxy-mvp rejects unmappable proxy_pass authority `{authority}`"),
            report,
        );
    }
}

fn is_named_upstream_ref(name: &str) -> bool {
    let name = name.trim_end_matches('/');
    !name.is_empty()
        && !name.starts_with('[')
        && name.parse::<std::net::SocketAddr>().is_err()
        && !name.contains('.')
        && !name
            .rsplit_once(':')
            .is_some_and(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()))
}

fn authority_looks_like_host_port(authority: &str) -> bool {
    if authority.parse::<std::net::SocketAddr>().is_ok() {
        return true;
    }
    if let Some((host, port)) = authority.rsplit_once(':') {
        if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if host.is_empty() {
            return false;
        }
        if host.starts_with('[') && host.ends_with(']') {
            return true;
        }
        if host.parse::<std::net::Ipv4Addr>().is_ok() {
            return true;
        }
        // hostname.tld or localhost
        return host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    }
    false
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
            profile: MigrateProfile::ReverseProxyMvp,
            ..MigrateOptions::default()
        }
    }

    #[test]
    fn accepts_inline_http_proxy() {
        let src = r#"
server {
    listen 127.0.0.1:8080;
    location /api/ {
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
            0
        );
        assert!(out.config.contains("[[upstream]]"));
        assert!(out.config.contains("target = \"http://127.0.0.1:3000\""));
        assert!(out.config.contains("upstream ="));
    }

    #[test]
    fn refuses_https() {
        let src = r#"
server {
    listen 80;
    location / {
        proxy_pass https://127.0.0.1:3000;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn refuses_upstream_block() {
        let src = r#"
upstream app { server 127.0.0.1:3000; }
server {
    listen 80;
    location / {
        proxy_pass http://app;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn refuses_proxy_set_header() {
        let src = r#"
server {
    listen 80;
    location / {
        proxy_set_header Host $host;
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn refuses_include() {
        let src = r#"
server {
    listen 80;
    include mime.types;
    location / {
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn refuses_variable_proxy_pass() {
        let src = r#"
server {
    listen 80;
    location / {
        proxy_pass http://$backend;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn refuses_unknown_directive() {
        let src = r#"
server {
    listen 80;
    gzip on;
    location / {
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn refuses_localhost_listen_false_success() {
        let src = r#"
server {
    listen localhost:8080;
    location / {
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn static_mvp_still_refuses_proxy() {
        let src = r#"
server {
    listen 80;
    root /var/www;
    location / {
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
        let out = migrate_source(
            "t.conf",
            src,
            &MigrateOptions {
                profile: MigrateProfile::StaticMvp,
                ..MigrateOptions::default()
            },
        )
        .expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::StaticMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }
}
