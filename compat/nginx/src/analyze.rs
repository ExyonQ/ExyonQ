//! Walk AST and extract server/location/upstream structures.

use crate::ast::{AnalyzedConfig, Config, Directive, NginxLbMethod, ParsedServer, ParsedUpstream};
use crate::limits::{MAX_SERVERS_PER_UPSTREAM, MAX_UPSTREAM_BLOCKS};
use crate::location::{
    analyze_location_directives, analyze_precedence, named_location_set, parse_location_directive,
};
use crate::report::{CompatStatus, CompatibilityReport};
use crate::upstream::classify_endpoint;

pub fn analyze(config: &Config, report: &mut CompatibilityReport) -> AnalyzedConfig {
    let mut out = AnalyzedConfig::default();
    for directive in &config.directives {
        walk_top(directive, &mut out, report);
    }
    for server in &out.servers {
        analyze_precedence(server, report);
        let named = named_location_set(server);
        for loc in &server.locations {
            analyze_location_directives(loc, &named, report);
        }
    }
    out
}

fn walk_top(directive: &Directive, out: &mut AnalyzedConfig, report: &mut CompatibilityReport) {
    match directive.name.as_str() {
        "http" | "main" => {
            if let Some(block) = &directive.block {
                for child in &block.directives {
                    walk_http(child, out, report);
                }
            }
        }
        "events" => {
            report.push(
                &directive.loc,
                &directive.name,
                CompatStatus::Ignored,
                "events block ignored for IR import",
                None,
            );
        }
        "upstream" => parse_upstream(directive, out, report),
        "server" => parse_server(directive, out, report),
        "include" => {}
        _ => report_unknown(directive, "main", report),
    }
}

fn walk_http(directive: &Directive, out: &mut AnalyzedConfig, report: &mut CompatibilityReport) {
    match directive.name.as_str() {
        "server" => parse_server(directive, out, report),
        "upstream" => parse_upstream(directive, out, report),
        "include" => {}
        "map" => parse_map(directive, out, report),
        "set_real_ip_from" | "real_ip_header" | "real_ip_recursive" => {
            analyze_real_ip_directive(directive, out, report, false);
        }
        "if" => {
            report.push(
                &directive.loc,
                "if",
                CompatStatus::Error,
                "nginx `if` is not supported (EXY-NG-IF / ADR-051); use match.header / map / explicit routes",
                None,
            );
        }
        _ => report_unknown(directive, "http", report),
    }
}

/// NG-11: nginx real_ip module → `modules.real_ip`. `set_real_ip_from`
/// accumulates trusted CIDRs; `real_ip_header` picks the chain header.
/// `real_ip_recursive on` matches our right-to-left trusted-skip walk;
/// `off` is reported Partial because we never take the untrusted-blind
/// last-entry semantics nginx uses when off.
fn analyze_real_ip_directive(
    directive: &Directive,
    out: &mut AnalyzedConfig,
    report: &mut CompatibilityReport,
    server_scoped: bool,
) {
    match directive.name.as_str() {
        "set_real_ip_from" => {
            let Some(arg) = directive.args.first() else {
                report.push(
                    &directive.loc,
                    "set_real_ip_from",
                    CompatStatus::Error,
                    "set_real_ip_from requires a CIDR or address",
                    None,
                );
                return;
            };
            if !valid_cidr_or_ip(arg) {
                report.push(
                    &directive.loc,
                    "set_real_ip_from",
                    CompatStatus::Partial,
                    format!("set_real_ip_from `{arg}` is not a CIDR/IP — skipped"),
                    None,
                );
                return;
            }
            out.real_ip_trusted.push(arg.clone());
            if server_scoped {
                report.push(
                    &directive.loc,
                    "set_real_ip_from",
                    CompatStatus::Partial,
                    format!(
                        "server-scoped set_real_ip_from {arg} mapped globally (ExyonQ real_ip is global) — applies to all servers"
                    ),
                    Some(format!("modules.real_ip.trusted += {arg}")),
                );
            } else {
                report.push(
                    &directive.loc,
                    "set_real_ip_from",
                    CompatStatus::Supported,
                    format!("set_real_ip_from {arg} → modules.real_ip.trusted"),
                    Some(format!("modules.real_ip.trusted += {arg}")),
                );
            }
        }
        "real_ip_header" => match directive.args.first() {
            Some(h)
                if !h.is_empty()
                    && h.len() <= 128
                    && h.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) =>
            {
                out.real_ip_header = Some(h.clone());
                report.push(
                    &directive.loc,
                    "real_ip_header",
                    CompatStatus::Supported,
                    format!("real_ip_header {h} → modules.real_ip.header"),
                    Some(format!("modules.real_ip.header={h}")),
                );
            }
            _ => report.push(
                &directive.loc,
                "real_ip_header",
                CompatStatus::Partial,
                "real_ip_header value is not a valid header name — skipped",
                None,
            ),
        },
        "real_ip_recursive" => {
            let on = directive.args.first().is_some_and(|a| a == "on");
            if on {
                report.push(
                    &directive.loc,
                    "real_ip_recursive",
                    CompatStatus::Supported,
                    "real_ip_recursive on matches ExyonQ trusted-chain walk",
                    None,
                );
            } else {
                report.push(
                    &directive.loc,
                    "real_ip_recursive",
                    CompatStatus::Partial,
                    "real_ip_recursive off: ExyonQ always resolves the chain right-to-left skipping trusted hops (recursive semantics)",
                    None,
                );
            }
        }
        _ => {}
    }
}

fn valid_cidr_or_ip(arg: &str) -> bool {
    let arg = arg.trim();
    if arg.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let Some((addr, prefix)) = arg.split_once('/') else {
        return false;
    };
    let Ok(ip) = addr.parse::<std::net::IpAddr>() else {
        return false;
    };
    let Ok(bits) = prefix.parse::<u8>() else {
        return false;
    };
    bits <= if ip.is_ipv4() { 32 } else { 128 }
}

fn parse_map(directive: &Directive, out: &mut AnalyzedConfig, report: &mut CompatibilityReport) {
    use crate::ast::ParsedMap;
    if directive.args.len() < 2 {
        report.push(
            &directive.loc,
            "map",
            CompatStatus::Error,
            "map requires source and destination variables",
            None,
        );
        return;
    }
    let source_var = directive.args[0].clone();
    let dest_var = directive.args[1].trim_start_matches('$').to_string();
    if !dest_var
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase())
        || !dest_var
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        report.push(
            &directive.loc,
            "map",
            CompatStatus::Unsupported,
            format!("map destination `${dest_var}` not a valid IR map name (ADR-051)"),
            None,
        );
        return;
    }
    let Some(block) = &directive.block else {
        report.push(
            &directive.loc,
            "map",
            CompatStatus::Error,
            "map requires a block body",
            None,
        );
        return;
    };
    let mut default = String::new();
    let mut values = std::collections::HashMap::new();
    for child in &block.directives {
        // Inside map, "directives" are `key value;` — name=key, args=[value]
        let key = child.name.as_str();
        let val = child.args.first().cloned().unwrap_or_default();
        if key == "default" {
            default = val;
            continue;
        }
        if key.starts_with('~') || key.starts_with("~*") {
            report.push(
                &child.loc,
                "map",
                CompatStatus::Unsupported,
                format!("regex map key `{key}` not supported (ADR-051)"),
                None,
            );
            return;
        }
        values.insert(key.to_string(), val);
    }
    out.maps.push(ParsedMap {
        loc: directive.loc.clone(),
        source_var,
        dest_var,
        default,
        values,
    });
    report.push(
        &directive.loc,
        "map",
        CompatStatus::Supported,
        "map block accepted for IR mapping (ADR-051)",
        None,
    );
}

fn parse_upstream(
    directive: &Directive,
    out: &mut AnalyzedConfig,
    report: &mut CompatibilityReport,
) {
    if out.upstreams.len() >= MAX_UPSTREAM_BLOCKS {
        report.push(
            &directive.loc,
            "upstream",
            CompatStatus::Error,
            format!("upstream block count exceeds limit ({MAX_UPSTREAM_BLOCKS})"),
            None,
        );
        return;
    }
    let name = directive
        .args
        .first()
        .cloned()
        .unwrap_or_else(|| "upstream".into());
    if out.upstreams.iter().any(|u| u.name == name) {
        report.push(
            &directive.loc,
            "upstream",
            CompatStatus::Error,
            format!("duplicate upstream block `{name}`"),
            None,
        );
        return;
    }
    let mut servers = Vec::new();
    let mut method = None;
    let mut sticky_cookie = None;
    let mut dns_refresh_ms = None;
    if let Some(block) = &directive.block {
        for child in &block.directives {
            match child.name.as_str() {
                "server" => {
                    if servers.len() >= MAX_SERVERS_PER_UPSTREAM {
                        report.push(
                            &child.loc,
                            "server",
                            CompatStatus::Error,
                            format!(
                                "upstream `{name}` exceeds max servers ({MAX_SERVERS_PER_UPSTREAM})"
                            ),
                            None,
                        );
                        continue;
                    }
                    let Some(target) = child.args.first() else {
                        report.push(
                            &child.loc,
                            "server",
                            CompatStatus::Error,
                            format!("upstream `{name}` server directive missing endpoint"),
                            None,
                        );
                        continue;
                    };
                    let endpoint = target.as_str();
                    let kind = classify_endpoint(endpoint);
                    if kind == crate::upstream::EndpointKind::Invalid {
                        report.push(
                            &child.loc,
                            "server",
                            CompatStatus::Error,
                            format!("upstream `{name}` invalid endpoint `{endpoint}`"),
                            None,
                        );
                        continue;
                    }
                    // Keep the full line so downstream mapping can read flags
                    // (`weight=`/`down`/`backup` mapped in NG-07;
                    // `max_fails=`/`fail_timeout=`/`slow_start=` in NG-08; the
                    // rest report Partial).
                    servers.push(child.args.join(" "));
                    let unknown: Vec<_> = child.args[1..]
                        .iter()
                        .filter(|a| {
                            !a.starts_with("weight=")
                                && !a.starts_with("max_fails=")
                                && !a.starts_with("fail_timeout=")
                                && !a.starts_with("slow_start=")
                                && a.as_str() != "down"
                                && a.as_str() != "backup"
                                // bare forms are handled as Error by the
                                // upstream mapper — not "unknown" flags.
                                && a.as_str() != "max_fails"
                                && a.as_str() != "fail_timeout"
                                && a.as_str() != "slow_start"
                        })
                        .collect();
                    if !unknown.is_empty() {
                        report.push(
                            &child.loc,
                            "server",
                            CompatStatus::Partial,
                            format!(
                                "upstream `{name}` server flags `{}` not represented in IR",
                                unknown
                                    .iter()
                                    .map(|s| s.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            ),
                            None,
                        );
                    }
                    report.push(
                        &child.loc,
                        "server",
                        CompatStatus::Supported,
                        format!("upstream `{name}` member `{endpoint}`"),
                        Some(format!("upstream.{name}")),
                    );
                }
                "keepalive" | "zone" => {
                    report.push(
                        &child.loc,
                        &child.name,
                        CompatStatus::Unsupported,
                        format!(
                            "upstream `{}` directive not supported in Tier 2A",
                            child.name
                        ),
                        None,
                    );
                }
                // NG-09: `resolver <addrs> valid=T` → periodic re-resolve.
                // Resolver addresses are ignored (system DNS); `valid=`
                // absent follows nginx TTL semantics → default 30s.
                "resolver" => {
                    let raw_valid = child.args.iter().find_map(|a| a.strip_prefix("valid="));
                    let ms = match raw_valid {
                        None => Some(30_000),
                        Some(v) => crate::upstream::parse_nginx_time_ms(v),
                    };
                    // A parseable-but-over-limit `valid=` degrades like a
                    // malformed one — Partial, not an upstream-killing error.
                    let ms = ms.filter(|m| *m <= exyonq_config_ir::MAX_DNS_REFRESH_MS);
                    match ms {
                        Some(ms) => {
                            dns_refresh_ms = Some(ms);
                            report.push(
                                &child.loc,
                                "resolver",
                                CompatStatus::Supported,
                                format!(
                                    "upstream `{name}` `resolver` → dns_refresh_ms={ms} (system DNS, addresses ignored)"
                                ),
                                Some(format!("upstream.{name}.dns_refresh_ms={ms}")),
                            );
                        }
                        None => report.push(
                            &child.loc,
                            "resolver",
                            CompatStatus::Partial,
                            format!(
                                "upstream `{name}` `resolver valid={}` invalid — re-resolve not configured",
                                raw_valid.unwrap_or("")
                            ),
                            None,
                        ),
                    }
                }
                // NG-07: nginx LB methods → IR selection_policy.
                "least_conn" | "ip_hash" | "hash" | "sticky" => {
                    let mapped = match child.name.as_str() {
                        "least_conn" => Some((NginxLbMethod::LeastConn, "least_conn", None)),
                        "ip_hash" => Some((NginxLbMethod::IpHash, "source_affinity", None)),
                        "hash" => {
                            let consistent = child.args.last().is_some_and(|a| a == "consistent");
                            let key = child.args.first().map(String::as_str);
                            if consistent && matches!(key, Some("$uri" | "$request_uri")) {
                                Some((NginxLbMethod::ConsistentHash, "consistent_hash", None))
                            } else {
                                report.push(
                                    &child.loc,
                                    "hash",
                                    CompatStatus::Partial,
                                    if consistent {
                                        format!(
                                            "upstream `{name}` `hash {} consistent` uses a non-URI key; ExyonQ consistent_hash keys on path+query",
                                            key.unwrap_or("$uri")
                                        )
                                    } else {
                                        "upstream `hash` without `consistent` not represented (nginx non-consistent hash semantics)".to_string()
                                    },
                                    None,
                                );
                                None
                            }
                        }
                        _ => match child.args.as_slice() {
                            [flag, cookie_name, extra @ ..] if flag == "cookie" => {
                                if !extra.is_empty() {
                                    report.push(
                                        &child.loc,
                                        "sticky",
                                        CompatStatus::Partial,
                                        format!(
                                            "upstream `{name}` `sticky cookie` attributes `{}` not mapped (ExyonQ mints `Path=/; HttpOnly`)",
                                            extra.join(" ")
                                        ),
                                        None,
                                    );
                                }
                                Some((
                                    NginxLbMethod::StickyCookie,
                                    "cookie_affinity",
                                    Some(cookie_name.clone()),
                                ))
                            }
                            _ => {
                                report.push(
                                    &child.loc,
                                    "sticky",
                                    CompatStatus::Unsupported,
                                    "upstream `sticky` forms other than `sticky cookie <name>` not supported".to_string(),
                                    None,
                                );
                                None
                            }
                        },
                    };
                    if let Some((m, ir_policy, cookie)) = mapped {
                        if method.replace(m).is_some() {
                            report.push(
                                &child.loc,
                                &child.name,
                                CompatStatus::Error,
                                format!("upstream `{name}` declares more than one load method"),
                                None,
                            );
                        } else {
                            report.push(
                                &child.loc,
                                &child.name,
                                CompatStatus::Supported,
                                format!(
                                    "upstream `{name}` `{}` → selection_policy={ir_policy}",
                                    child.name
                                ),
                                Some(format!("upstream.{name}.selection_policy={ir_policy}")),
                            );
                            if let Some(cookie) = cookie {
                                sticky_cookie = Some(cookie);
                            }
                        }
                    }
                }
                other => {
                    let status = if matches!(
                        other,
                        "weight" | "max_fails" | "fail_timeout" | "slow_start" | "backup" | "down"
                    ) {
                        CompatStatus::Partial
                    } else {
                        CompatStatus::Unsupported
                    };
                    report.push(
                        &child.loc,
                        other,
                        status,
                        format!("upstream `{name}` directive `{other}` not in Tier 2A subset"),
                        None,
                    );
                }
            }
        }
    }
    if servers.is_empty() {
        report.push(
            &directive.loc,
            "upstream",
            CompatStatus::Error,
            format!("upstream `{name}` has no server members"),
            None,
        );
    }
    out.upstreams.push(ParsedUpstream {
        name,
        servers,
        method,
        sticky_cookie,
        dns_refresh_ms,
        loc: directive.loc.clone(),
    });
}

fn parse_server(directive: &Directive, out: &mut AnalyzedConfig, report: &mut CompatibilityReport) {
    let mut server = ParsedServer {
        directives: Vec::new(),
        locations: Vec::new(),
        loc: directive.loc.clone(),
    };
    let mut loc_order = 0usize;
    if let Some(block) = &directive.block {
        for child in &block.directives {
            if child.name == "location" {
                if let Some(loc) = parse_location_directive(child, loc_order, report) {
                    loc_order += 1;
                    server.locations.push(loc);
                }
            } else {
                server.directives.push(child.clone());
                classify_server_directive(child, out, report);
            }
        }
    }
    out.servers.push(server);
}

fn classify_server_directive(
    directive: &Directive,
    out: &mut AnalyzedConfig,
    report: &mut CompatibilityReport,
) {
    match directive.name.as_str() {
        "listen" | "root" | "index" | "return" => {
            report.push(
                &directive.loc,
                &directive.name,
                CompatStatus::Supported,
                "server-level directive accepted for mapping",
                None,
            );
        }
        "error_page" => {
            // Mapping / Supported|Unsupported decided in map_server_error_pages.
        }
        "if" => {
            report.push(
                &directive.loc,
                "if",
                CompatStatus::Error,
                "nginx `if` is not supported (EXY-NG-IF / ADR-051); use match.header / map / explicit routes",
                None,
            );
        }
        "server_name" => classify_server_name_directive(directive, report),
        "set_real_ip_from" | "real_ip_header" | "real_ip_recursive" => {
            analyze_real_ip_directive(directive, out, report, true);
        }
        "include" => {}
        "ssl_certificate" | "ssl_certificate_key" => {
            report.push(
                &directive.loc,
                &directive.name,
                CompatStatus::Partial,
                "TLS paths noted; manual review recommended",
                None,
            );
        }
        _ => report_unknown(directive, "server", report),
    }
}

fn classify_server_name_directive(directive: &Directive, report: &mut CompatibilityReport) {
    for arg in &directive.args {
        if arg == "_" {
            report.push(
                &directive.loc,
                "server_name",
                CompatStatus::Ignored,
                "catch-all server_name `_` ignored",
                None,
            );
            continue;
        }
        if arg.starts_with('~') || arg.starts_with("~*") {
            report.push(
                &directive.loc,
                "server_name",
                CompatStatus::Unsupported,
                format!("regex server_name `{arg}` not supported"),
                None,
            );
        } else if arg.contains('*') {
            report.push(
                &directive.loc,
                "server_name",
                CompatStatus::Partial,
                format!("wildcard server_name `{arg}` not represented in IR"),
                None,
            );
        } else if !arg.is_empty() {
            report.push(
                &directive.loc,
                "server_name",
                CompatStatus::Supported,
                format!("server_name `{arg}` mapped"),
                None,
            );
        }
    }
}

fn report_unknown(directive: &Directive, ctx: &str, report: &mut CompatibilityReport) {
    let status = if directive.name.starts_with("proxy_") || directive.name.starts_with("fastcgi_") {
        CompatStatus::Partial
    } else {
        CompatStatus::Unsupported
    };
    report.push(
        &directive.loc,
        &directive.name,
        status,
        format!("{ctx} directive `{}` not in Tier 2B subset", directive.name),
        None,
    );
}
