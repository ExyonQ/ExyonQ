//! Fail-closed FastCGI/PHP MVP profile for NGINX → ExyonQ import.
//!
//! Opt-in only. Does not alter Full, `static-mvp`, or `reverse-proxy-mvp` behavior.

use crate::ast::{AnalyzedConfig, Config, Directive, LocationKind};
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};

const SERVER_ALLOWED: &[&str] = &["listen", "server_name", "root", "location"];
const LOCATION_ALLOWED: &[&str] = &["fastcgi_pass", "fastcgi_param"];

const HARD_REJECT: &[&str] = &[
    "index",
    "fastcgi_index",
    "alias",
    "try_files",
    "proxy_pass",
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
    "fastcgi_split_path_info",
    "fastcgi_intercept_errors",
    "fastcgi_buffers",
    "fastcgi_buffer_size",
    "fastcgi_busy_buffers_size",
    "fastcgi_cache",
    "fastcgi_read_timeout",
    "fastcgi_connect_timeout",
    "fastcgi_send_timeout",
    "fastcgi_keep_conn",
];

/// Apply fastcgi-php-mvp allowlist / reject rules onto `report`.
pub fn enforce(config: &Config, analyzed: &AnalyzedConfig, report: &mut CompatibilityReport) {
    for directive in &config.directives {
        walk_top(directive, report);
    }

    if analyzed.servers.is_empty() {
        report.push(
            &SourceLoc::unlocated("fastcgi-php-mvp"),
            "server",
            CompatStatus::Error,
            "fastcgi-php-mvp requires at least one server block",
            None,
        );
        return;
    }

    for server in &analyzed.servers {
        let mut has_listen = false;
        let mut listen_count = 0usize;
        let mut has_root = false;
        let mut root_count = 0usize;
        for d in &server.directives {
            match d.name.as_str() {
                "listen" => {
                    listen_count += 1;
                    has_listen = true;
                    validate_listen(d, report);
                }
                "server_name" => validate_server_name(d, report),
                "root" => {
                    root_count += 1;
                    has_root = true;
                    validate_root(d, report);
                }
                other => {
                    if !SERVER_ALLOWED.contains(&other) {
                        reject(
                            &d.loc,
                            other,
                            format!("fastcgi-php-mvp rejects server directive `{other}`"),
                            report,
                        );
                    }
                }
            }
            if HARD_REJECT.contains(&d.name.as_str()) {
                reject(
                    &d.loc,
                    &d.name,
                    format!("fastcgi-php-mvp rejects `{}`", d.name),
                    report,
                );
            }
        }
        if listen_count > 1 {
            reject(
                &server.loc,
                "listen",
                "fastcgi-php-mvp allows exactly one `listen` per server",
                report,
            );
        }
        if !has_listen {
            reject(
                &server.loc,
                "listen",
                "fastcgi-php-mvp requires `listen` in each server",
                report,
            );
        }
        if root_count > 1 {
            reject(
                &server.loc,
                "root",
                "fastcgi-php-mvp allows exactly one `root` per server",
                report,
            );
        }
        if !has_root {
            reject(
                &server.loc,
                "root",
                "fastcgi-php-mvp requires `root` (maps to fcgi_pool.document_root)",
                report,
            );
        }

        if server.locations.is_empty() {
            reject(
                &server.loc,
                "location",
                "fastcgi-php-mvp requires exactly one `location` with `fastcgi_pass`",
                report,
            );
            continue;
        }
        if server.locations.len() > 1 {
            reject(
                &server.loc,
                "location",
                "fastcgi-php-mvp allows exactly one location block",
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
                            "fastcgi-php-mvp only allows simple prefix location (got {} `{}`)",
                            other.as_str(),
                            loc.raw_pattern
                        ),
                        report,
                    );
                }
            }
            if loc.raw_pattern.starts_with('@')
                || loc.raw_pattern.starts_with('~')
                || loc.raw_pattern.starts_with('=')
                || loc.raw_pattern.starts_with("^~")
            {
                reject(
                    &loc.loc,
                    "location",
                    format!(
                        "fastcgi-php-mvp rejects non-literal location `{}`",
                        loc.raw_pattern
                    ),
                    report,
                );
            }
            if !is_explicit_php_path(&loc.raw_pattern) {
                reject(
                    &loc.loc,
                    "location",
                    format!(
                        "fastcgi-php-mvp requires an explicit PHP script path (got `{}`)",
                        loc.raw_pattern
                    ),
                    report,
                );
            }

            let mut pass_count = 0usize;
            let mut param_count = 0usize;
            for d in &loc.directives {
                if HARD_REJECT.contains(&d.name.as_str()) {
                    reject(
                        &d.loc,
                        &d.name,
                        format!("fastcgi-php-mvp rejects `{}`", d.name),
                        report,
                    );
                }
                if !LOCATION_ALLOWED.contains(&d.name.as_str()) {
                    reject(
                        &d.loc,
                        &d.name,
                        format!(
                            "fastcgi-php-mvp rejects directive `{}` inside location",
                            d.name
                        ),
                        report,
                    );
                }
                match d.name.as_str() {
                    "fastcgi_pass" => {
                        pass_count += 1;
                        validate_fastcgi_pass(d, report);
                    }
                    "fastcgi_param" => {
                        param_count += 1;
                        validate_fastcgi_param(d, report);
                    }
                    _ => {}
                }
            }
            if pass_count == 0 {
                reject(
                    &loc.loc,
                    "fastcgi_pass",
                    "fastcgi-php-mvp location requires `fastcgi_pass IP:port`",
                    report,
                );
            }
            if pass_count > 1 {
                reject(
                    &loc.loc,
                    "fastcgi_pass",
                    "fastcgi-php-mvp allows exactly one `fastcgi_pass` per location",
                    report,
                );
            }
            if param_count > 1 {
                reject(
                    &loc.loc,
                    "fastcgi_param",
                    "fastcgi-php-mvp allows at most one `fastcgi_param` (exact SCRIPT_FILENAME form)",
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
                format!("fastcgi-php-mvp rejects upstream block `{}`", u.name),
                report,
            );
        }
    }
}

/// True when the profile must refuse to emit IR.
pub fn must_refuse(report: &CompatibilityReport) -> bool {
    report.has_errors() || report.has_partial_or_unsupported()
}

fn is_explicit_php_path(raw: &str) -> bool {
    if !raw.starts_with('/') || raw.contains('$') || raw.contains('*') {
        return false;
    }
    let lower = raw.to_ascii_lowercase();
    lower.ends_with(".php")
        && lower.len() > 4
        && !lower.contains("..")
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~'))
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
            "fastcgi-php-mvp rejects `events` (out of accept set)",
            report,
        ),
        "upstream" => reject(
            &directive.loc,
            "upstream",
            "fastcgi-php-mvp rejects `upstream`",
            report,
        ),
        "include" => reject(
            &directive.loc,
            "include",
            "fastcgi-php-mvp rejects `include`",
            report,
        ),
        other => {
            if HARD_REJECT.contains(&other) || !matches!(other, "http" | "main" | "server") {
                reject(
                    &directive.loc,
                    other,
                    format!("fastcgi-php-mvp rejects top-level `{other}`"),
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
            "fastcgi-php-mvp rejects `upstream`",
            report,
        ),
        "include" => reject(
            &directive.loc,
            "include",
            "fastcgi-php-mvp rejects `include`",
            report,
        ),
        other => reject(
            &directive.loc,
            other,
            format!("fastcgi-php-mvp rejects http-context `{other}`"),
            report,
        ),
    }
}

fn walk_server_block(directive: &Directive, report: &mut CompatibilityReport) {
    let Some(block) = &directive.block else {
        reject(
            &directive.loc,
            "server",
            "fastcgi-php-mvp requires a non-empty server block",
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
                            "fastcgi-php-mvp rejects nested location",
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
            "fastcgi-php-mvp requires `listen` address/port",
            report,
        );
        return;
    }
    if directive.args.len() > 1 {
        reject(
            &directive.loc,
            "listen",
            format!(
                "fastcgi-php-mvp rejects listen flags/options (`{}`)",
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
                "fastcgi-php-mvp rejects unmappable listen `{raw}` (need port, IPv4/IPv6:port, or *:port)"
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
    if raw.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if let Some((host, port)) = raw.rsplit_once(':') {
        if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
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
            "fastcgi-php-mvp `server_name` requires at least one literal name",
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
                format!("fastcgi-php-mvp rejects non-literal server_name `{name}`"),
                report,
            );
        }
    }
}

fn validate_root(directive: &Directive, report: &mut CompatibilityReport) {
    if directive.args.len() != 1 {
        reject(
            &directive.loc,
            "root",
            "fastcgi-php-mvp requires a single absolute `root` path",
            report,
        );
        return;
    }
    let path = directive.args[0].as_str();
    if path.contains('$') || !path.starts_with('/') {
        reject(
            &directive.loc,
            "root",
            format!("fastcgi-php-mvp requires absolute filesystem `root` (got `{path}`)"),
            report,
        );
    }
}

fn validate_fastcgi_pass(directive: &Directive, report: &mut CompatibilityReport) {
    if directive.args.len() != 1 {
        reject(
            &directive.loc,
            "fastcgi_pass",
            "fastcgi-php-mvp requires a single `fastcgi_pass` argument",
            report,
        );
        return;
    }
    let raw = directive.args[0].as_str();
    if raw.contains('$') {
        reject(
            &directive.loc,
            "fastcgi_pass",
            "fastcgi-php-mvp rejects variables in `fastcgi_pass`",
            report,
        );
        return;
    }
    if raw.starts_with("unix:") || raw.starts_with('/') {
        reject(
            &directive.loc,
            "fastcgi_pass",
            "fastcgi-php-mvp rejects unix socket `fastcgi_pass` (TCP SocketAddr only)",
            report,
        );
        return;
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        reject(
            &directive.loc,
            "fastcgi_pass",
            "fastcgi-php-mvp rejects http(s) scheme in `fastcgi_pass`",
            report,
        );
        return;
    }
    // Runtime resolves TCP via SocketAddr only (no DNS at request time).
    if raw.parse::<std::net::SocketAddr>().is_err() {
        reject(
            &directive.loc,
            "fastcgi_pass",
            format!("fastcgi-php-mvp requires `fastcgi_pass` as literal SocketAddr (got `{raw}`)"),
            report,
        );
    }
}

fn validate_fastcgi_param(directive: &Directive, report: &mut CompatibilityReport) {
    if directive.args.len() != 2 {
        reject(
            &directive.loc,
            "fastcgi_param",
            "fastcgi-php-mvp requires `fastcgi_param NAME VALUE`",
            report,
        );
        return;
    }
    let name = directive.args[0].as_str();
    let value = directive.args[1].as_str();
    let exact = name == "SCRIPT_FILENAME"
        && (value == "$document_root$fastcgi_script_name"
            || value == "\"$document_root$fastcgi_script_name\"");
    if !exact {
        reject(
            &directive.loc,
            "fastcgi_param",
            format!(
                "fastcgi-php-mvp only accepts `fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name` (got `{name} {value}`)"
            ),
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
            profile: MigrateProfile::FastcgiPhpMvp,
            ..MigrateOptions::default()
        }
    }

    #[test]
    fn accepts_tcp_php_location() {
        let src = r#"
server {
    listen 127.0.0.1:8080;
    root /var/www/html;
    location /index.php {
        fastcgi_pass 127.0.0.1:9000;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
            0
        );
        assert!(out.config.contains("[[fcgi_pool]]"));
        assert!(out.config.contains("transport = \"tcp\""));
        assert!(out.config.contains("address = \"127.0.0.1:9000\""));
        assert!(out.config.contains("document_root = \"/var/www/html\""));
        assert!(out.config.contains("fastcgi ="));
        assert!(!out.config.contains("index ="));
    }

    #[test]
    fn refuses_unix_socket() {
        let src = r#"
server {
    listen 80;
    root /var/www;
    location /index.php {
        fastcgi_pass unix:/run/php/php-fpm.sock;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn refuses_index_directive() {
        let src = r#"
server {
    listen 80;
    root /var/www;
    index index.php;
    location /index.php {
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn refuses_include_fastcgi_params() {
        let src = r#"
server {
            listen 80;
    root /var/www;
    location /index.php {
        include fastcgi_params;
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
        let out = migrate_source("t.conf", src, &opts()).expect("migrate");
        assert_eq!(
            out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
            1
        );
        assert!(out.config.trim().is_empty());
    }

    #[test]
    fn full_still_blocks_tcp() {
        let src = r#"
server {
    listen 80;
    root /var/www;
    location /index.php {
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
        let out = migrate_source(
            "t.conf",
            src,
            &MigrateOptions {
                profile: MigrateProfile::Full,
                ..MigrateOptions::default()
            },
        )
        .expect("migrate");
        assert!(!out.config.contains("transport = \"tcp\""));
    }
}
