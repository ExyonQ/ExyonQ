//! WordPress nginx import.
//!
//! Emits the product config: static `/wp-includes` and `/wp-content`, one Unix
//! PHP-FPM pool, and `/` with `htaccess = "overlay"`. This is not nginx
//! compatibility. A directive that would change routing and is not that front
//! controller is an error, and then no config is emitted.

use crate::ast::{AnalyzedConfig, Directive, LocationKind, ParsedLocation, ParsedServer};
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};
use std::net::SocketAddr;

/// Scope of this profile. Full nginx compatibility stays forbidden.
pub const WORDPRESS_IMPORT_SCOPE: &str = "FRONT_CONTROLLER_ONLY";

const ROUTING_REJECT: &[&str] = &[
    "rewrite",
    "if",
    "proxy_pass",
    "alias",
    "return",
    "uwsgi_pass",
    "scgi_pass",
    "grpc_pass",
    "set",
];

/// Refuse only on errors. Notes about directives that were not copied are expected.
pub fn must_refuse(report: &CompatibilityReport) -> bool {
    report.has_errors()
}

/// Build the WordPress product TOML, or `None` when the file is not that subset.
pub fn emit(analyzed: &AnalyzedConfig, report: &mut CompatibilityReport) -> Option<String> {
    if analyzed.servers.len() != 1 {
        let loc = analyzed
            .servers
            .first()
            .map(|server| server.loc.clone())
            .unwrap_or_else(|| SourceLoc::unlocated("wordpress"));
        report.push(
            &loc,
            "server",
            CompatStatus::Error,
            format!(
                "wordpress import accepts one server block, found {}",
                analyzed.servers.len()
            ),
            None,
        );
        return None;
    }

    let server = &analyzed.servers[0];
    reject_routing_directives(&server.directives, report);
    for location in &server.locations {
        reject_routing_directives(&location.directives, report);
        if location.kind == LocationKind::RegexCaseSensitive
            || location.kind == LocationKind::RegexCaseInsensitive
            || location.kind == LocationKind::Named
        {
            report.push(
                &location.loc,
                "location",
                CompatStatus::Partial,
                format!(
                    "location `{}` is not copied; PHP is the htaccess overlay on `/`",
                    location.raw_pattern
                ),
                None,
            );
        }
    }

    let listen = one_listen(server, report)?;
    let root = one_root(server, report)?;
    let socket = one_socket(server, analyzed, report)?;
    if !has_front_controller(server, report) {
        return None;
    }
    if report.has_errors() {
        return None;
    }

    report.push(
        &server.loc,
        "wordpress",
        CompatStatus::Supported,
        format!(
            "emitted the WordPress product config ({WORDPRESS_IMPORT_SCOPE}). nginx directives are not copied. Put the WordPress .htaccess in the document root; this import does not write it"
        ),
        Some("route.htaccess=overlay".to_string()),
    );
    report.summary.listeners_generated = 1;
    report.summary.sites_generated = 1;
    report.summary.routes_generated = 3;
    report.summary.fcgi_pools_generated = 1;

    let includes = join_root(&root, "/wp-includes");
    let content = join_root(&root, "/wp-content");
    Some(format!(
        r#"# wordpress import: front controller only. nginx compatibility is not claimed.
# /wp-includes and /wp-content are files. / executes PHP through the htaccess overlay.
# The document root still needs the WordPress .htaccess. This file does not write it.

config_version = 2

[logging]
level = "info"
format = "text"

[logging.console]
enabled = true
stream = "stdout"

[logging.access]
enabled = true

[[server]]
listen = "{listen}"
routes = ["wp-includes", "wp-content", "wordpress"]

[[route]]
name = "wp-includes"
match = {{ path = "/wp-includes" }}
root = "{includes}"

[[route]]
name = "wp-content"
match = {{ path = "/wp-content" }}
root = "{content}"

[[route]]
name = "wordpress"
match = {{ path = "/" }}
fastcgi = "php"
htaccess = "overlay"

[[fcgi_pool]]
name = "php"
address = "{socket}"
transport = "unix"
document_root = "{root}"
max_concurrency = 16
idle_timeout_ms = 30000
total_timeout_ms = 120000
checkout_timeout_ms = 5000

[full_page_cache]
enabled = true
default_ttl_seconds = 300
max_ttl_seconds = 3600
max_object_bytes = 4194304
max_total_bytes = 67108864
max_entries = 10000
namespace = 4
"#
    ))
}

fn reject_routing_directives(directives: &[Directive], report: &mut CompatibilityReport) {
    for directive in directives {
        if ROUTING_REJECT.contains(&directive.name.as_str()) {
            report.push(
                &directive.loc,
                &directive.name,
                CompatStatus::Error,
                format!(
                    "wordpress import does not copy `{}`; refusing so the product config does not pretend to",
                    directive.name
                ),
                None,
            );
        }
    }
}

fn one_listen(server: &ParsedServer, report: &mut CompatibilityReport) -> Option<String> {
    let mut chosen: Option<(String, u16)> = None;
    for directive in server
        .directives
        .iter()
        .filter(|directive| directive.name == "listen")
    {
        match parse_listen(&directive.args) {
            Ok((addr, port, tls)) => {
                if tls {
                    report.push(
                        &directive.loc,
                        "listen",
                        CompatStatus::Partial,
                        "tls flags on listen are not imported; set cert and key on the server",
                        None,
                    );
                }
                match &chosen {
                    None => chosen = Some((addr, port)),
                    Some((prev, prev_port)) if *prev_port == port => {
                        report.push(
                            &directive.loc,
                            "listen",
                            CompatStatus::Partial,
                            format!("extra listen `{addr}` is not copied; emitted `{prev}`"),
                            None,
                        );
                    }
                    Some((_, prev_port)) => {
                        report.push(
                            &directive.loc,
                            "listen",
                            CompatStatus::Error,
                            format!(
                                "wordpress import emits one listen; ports {prev_port} and {port} differ"
                            ),
                            None,
                        );
                    }
                }
            }
            Err(message) => {
                report.push(&directive.loc, "listen", CompatStatus::Error, message, None);
            }
        }
    }
    if chosen.is_none() && !report.has_errors() {
        report.push(
            &server.loc,
            "listen",
            CompatStatus::Error,
            "wordpress import requires one listen",
            None,
        );
    }
    chosen.map(|(addr, _)| addr)
}

fn parse_listen(args: &[String]) -> Result<(String, u16, bool), String> {
    let Some(first) = args.first() else {
        return Err("listen has no address".to_string());
    };
    if first.starts_with("unix:") {
        return Err("a unix listen is not imported".to_string());
    }
    let tls = args.iter().any(|arg| arg == "ssl" || arg == "quic");
    if first.chars().all(|c| c.is_ascii_digit()) {
        let port: u16 = first
            .parse()
            .map_err(|_| format!("listen `{first}` is not a port"))?;
        if port == 0 {
            return Err("listen port 0 is not imported".to_string());
        }
        return Ok((format!("0.0.0.0:{port}"), port, tls));
    }
    let sock: SocketAddr = first
        .parse()
        .map_err(|_| format!("listen `{first}` is not an IP:port or a port"))?;
    if sock.port() == 0 {
        return Err("listen port 0 is not imported".to_string());
    }
    Ok((first.clone(), sock.port(), tls))
}

fn one_root(server: &ParsedServer, report: &mut CompatibilityReport) -> Option<String> {
    let mut root: Option<String> = None;
    for directive in server
        .directives
        .iter()
        .filter(|directive| directive.name == "root")
    {
        match plain_absolute(
            "root",
            directive.args.first().map(String::as_str).unwrap_or(""),
        ) {
            Ok(path) => {
                if root.is_some() {
                    report.push(
                        &directive.loc,
                        "root",
                        CompatStatus::Error,
                        "wordpress import accepts one document root",
                        None,
                    );
                } else {
                    root = Some(path);
                }
            }
            Err(message) => {
                report.push(&directive.loc, "root", CompatStatus::Error, message, None);
            }
        }
    }
    for location in &server.locations {
        for directive in location
            .directives
            .iter()
            .filter(|directive| directive.name == "root")
        {
            let path = directive.args.first().map(String::as_str).unwrap_or("");
            if root.as_deref() != Some(path) {
                report.push(
                    &directive.loc,
                    "root",
                    CompatStatus::Error,
                    "a location root different from the server root is not imported",
                    None,
                );
            }
        }
    }
    if root.is_none() && !report.has_errors() {
        report.push(
            &server.loc,
            "root",
            CompatStatus::Error,
            "wordpress import requires a document root",
            None,
        );
    }
    root
}

fn one_socket(
    server: &ParsedServer,
    analyzed: &AnalyzedConfig,
    report: &mut CompatibilityReport,
) -> Option<String> {
    let mut socket: Option<String> = None;
    let mut saw_pass = false;
    for location in &server.locations {
        for directive in location
            .directives
            .iter()
            .filter(|directive| directive.name == "fastcgi_pass")
        {
            saw_pass = true;
            let arg = directive.args.first().map(String::as_str).unwrap_or("");
            match resolve_socket(arg, analyzed) {
                Ok(path) => {
                    if let Some(prev) = &socket {
                        if prev != &path {
                            report.push(
                                &directive.loc,
                                "fastcgi_pass",
                                CompatStatus::Error,
                                format!("two php sockets `{prev}` and `{path}`; wordpress import uses one"),
                                None,
                            );
                        }
                    } else {
                        socket = Some(path);
                    }
                }
                Err(message) => {
                    report.push(
                        &directive.loc,
                        "fastcgi_pass",
                        CompatStatus::Error,
                        message,
                        None,
                    );
                }
            }
        }
    }
    if socket.is_none() && !saw_pass {
        report.push(
            &server.loc,
            "fastcgi_pass",
            CompatStatus::Error,
            "wordpress import requires fastcgi_pass to a unix socket in this file; includes are not expanded",
            None,
        );
    }
    socket
}

fn resolve_socket(arg: &str, analyzed: &AnalyzedConfig) -> Result<String, String> {
    if let Some(path) = arg.strip_prefix("unix:") {
        return plain_absolute("fastcgi socket", path);
    }
    if arg.contains(':') || arg.contains('/') {
        return Err(format!("fastcgi_pass `{arg}` is not a unix socket"));
    }
    let Some(upstream) = analyzed.upstreams.iter().find(|item| item.name == arg) else {
        return Err(format!(
            "fastcgi_pass `{arg}` is not a unix socket and not an upstream in this file"
        ));
    };
    if upstream.servers.len() != 1 {
        return Err(format!(
            "upstream `{arg}` must contain exactly one unix server"
        ));
    }
    let Some(path) = upstream.servers[0].strip_prefix("unix:") else {
        return Err(format!(
            "upstream `{arg}` member `{}` is not a unix socket",
            upstream.servers[0]
        ));
    };
    plain_absolute("fastcgi socket", path)
}

fn has_front_controller(server: &ParsedServer, report: &mut CompatibilityReport) -> bool {
    let mut found = false;
    for location in &server.locations {
        classify_try_files(location, report, &mut found);
    }
    if !found {
        report.push(
            &server.loc,
            "try_files",
            CompatStatus::Error,
            "wordpress import requires try_files whose last argument is /index.php",
            None,
        );
    }
    found && !report.has_errors()
}

fn classify_try_files(
    location: &ParsedLocation,
    report: &mut CompatibilityReport,
    found: &mut bool,
) {
    for directive in location
        .directives
        .iter()
        .filter(|directive| directive.name == "try_files")
    {
        let Some(last) = directive.args.last() else {
            report.push(
                &directive.loc,
                "try_files",
                CompatStatus::Error,
                "try_files has no fallback",
                None,
            );
            continue;
        };
        if is_index_php(last) {
            *found = true;
            report.push(
                &directive.loc,
                "try_files",
                CompatStatus::Supported,
                "front controller recognized; PHP runs through the htaccess overlay, not a copied try_files",
                Some("route.htaccess=overlay".to_string()),
            );
        } else if last.starts_with('=') && last[1..].chars().all(|c| c.is_ascii_digit()) {
            report.push(
                &directive.loc,
                "try_files",
                CompatStatus::Partial,
                format!("status fallback `{last}` is not copied; a missing static file is a 404"),
                None,
            );
        } else {
            report.push(
                &directive.loc,
                "try_files",
                CompatStatus::Error,
                format!("try_files fallback `{last}` is not the WordPress front controller"),
                None,
            );
        }
    }
}

fn is_index_php(fallback: &str) -> bool {
    let path = fallback.split(['?', '$']).next().unwrap_or(fallback);
    path == "/index.php"
}

fn plain_absolute(kind: &str, raw: &str) -> Result<String, String> {
    if raw.is_empty() || !raw.starts_with('/') {
        return Err(format!("{kind} `{raw}` must be an absolute path"));
    }
    if raw.contains("..")
        || raw.contains('"')
        || raw.contains('\\')
        || raw.contains('\n')
        || raw.contains('$')
        || raw.contains('\'')
    {
        return Err(format!("{kind} `{raw}` is not a plain absolute path"));
    }
    Ok(raw.to_string())
}

fn join_root(root: &str, suffix: &str) -> String {
    format!("{}{suffix}", root.trim_end_matches('/'))
}
