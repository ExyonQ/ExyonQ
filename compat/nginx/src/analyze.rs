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
    // NG-18 Phase C: http-scope sub_filter defaults.
    for directive in &config.directives {
        if directive.name == "http" {
            if let Some(block) = &directive.block {
                out.sub_filter_http = parse_sub_filter(&block.directives, report);
                out.mirror_http = parse_mirror(&block.directives, report);
                out.slice_http = parse_slice(&block.directives, report);
                out.auth_http = parse_auth(&block.directives, report);
                out.limit_zones_http = parse_limit_req_zone(&block.directives, report);
                out.limit_req_http = parse_limit_req(&block.directives, report);
            }
        }
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
        "gunzip" => match parse_on_off(directive) {
            Ok(v) => {
                out.gunzip_http = Some(v);
                report.push(
                    &directive.loc,
                    "gunzip",
                    CompatStatus::Supported,
                    format!(
                        "http gunzip {} → modules.gunzip.enabled (Phase B)",
                        if v { "on" } else { "off" }
                    ),
                    Some(format!("[modules.gunzip] enabled = {v}")),
                );
            }
            Err(msg) => report.push(&directive.loc, "gunzip", CompatStatus::Error, msg, None),
        },
        "gunzip_buffers" => match parse_gunzip_buffers(directive) {
            Ok(v) => {
                out.gunzip_buffers_http = Some(v);
                report.push(
                    &directive.loc,
                    "gunzip_buffers",
                    CompatStatus::Supported,
                    format!("http gunzip_buffers → modules.gunzip.buffer_bytes = {v}"),
                    Some(format!("[modules.gunzip] buffer_bytes = {v}")),
                );
            }
            Err(msg) => report.push(
                &directive.loc,
                "gunzip_buffers",
                CompatStatus::Error,
                msg,
                None,
            ),
        },
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
        gunzip: None,
        gunzip_buffers: None,
        sub_filter: None,
        mirror: None,
        slice: None,
        auth: None,
        limit_req: None,
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
                match child.name.as_str() {
                    "gunzip" => match parse_on_off(child) {
                        Ok(v) => server.gunzip = Some(v),
                        Err(msg) => {
                            report.push(&child.loc, "gunzip", CompatStatus::Error, msg, None)
                        }
                    },
                    "gunzip_buffers" => match parse_gunzip_buffers(child) {
                        Ok(v) => server.gunzip_buffers = Some(v),
                        Err(msg) => report.push(
                            &child.loc,
                            "gunzip_buffers",
                            CompatStatus::Error,
                            msg,
                            None,
                        ),
                    },
                    _ => {}
                }
                if matches!(
                    child.name.as_str(),
                    "sub_filter"
                        | "sub_filter_once"
                        | "sub_filter_types"
                        | "sub_filter_last_modified"
                ) {
                    server.sub_filter = parse_sub_filter(&server.directives, report);
                }
                if matches!(child.name.as_str(), "mirror" | "mirror_request_body") {
                    server.mirror = parse_mirror(&server.directives, report);
                }
                if child.name == "slice" {
                    server.slice = parse_slice(&server.directives, report);
                }
                if matches!(child.name.as_str(), "auth_basic" | "auth_basic_user_file") {
                    server.auth = parse_auth(&server.directives, report);
                }
                if child.name == "limit_req" {
                    server.limit_req = parse_limit_req(&server.directives, report);
                }
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
        // NG-17: mapped (and reported) by map_ir; inheritance handled there.
        "add_header" | "expires" => {}
        // NG-18 / ADR-064: parsed by parse_server; status reported there.
        "gunzip" | "gunzip_buffers" => {}
        // NG-18: remaining body transforms are still open (ADR-064).
        "sub_filter" | "sub_filter_once" | "sub_filter_types" | "sub_filter_last_modified" => {}
        "mirror" | "mirror_request_body" => {}
        "slice" => {}
        "auth_basic" | "auth_basic_user_file" => {}
        "limit_req" | "limit_req_zone" => {}
        "sub_filter_unsupported_placeholder" => {
            report.push(
                &directive.loc,
                &directive.name,
                CompatStatus::Unsupported,
                "body-transform directive is not implemented (NG-18 / ADR-064)",
                None,
            );
        }
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

/// NG-18: `on|off` with exactly one argument.
pub(crate) fn parse_on_off(directive: &Directive) -> Result<bool, String> {
    match directive.args.as_slice() {
        [value] if value.eq_ignore_ascii_case("on") => Ok(true),
        [value] if value.eq_ignore_ascii_case("off") => Ok(false),
        _ => Err(format!(
            "`{}` expects exactly `on` or `off`",
            directive.name
        )),
    }
}

/// NG-18: `gunzip_buffers <count> <size>` → total bytes.
pub(crate) fn parse_gunzip_buffers(directive: &Directive) -> Result<usize, String> {
    let [count, size] = directive.args.as_slice() else {
        return Err("`gunzip_buffers` expects `<count> <size>`".into());
    };
    let count: usize = count
        .parse()
        .map_err(|_| format!("`gunzip_buffers` count `{count}` is not a number"))?;
    let size = parse_nginx_size(size)
        .ok_or_else(|| format!("`gunzip_buffers` size `{size}` is not a valid size"))?;
    count
        .checked_mul(size)
        .filter(|total| (1024..=1_048_576).contains(total))
        .ok_or_else(|| "`gunzip_buffers` total must be 1024..=1048576 bytes".into())
}

pub(crate) fn parse_nginx_size(value: &str) -> Option<usize> {
    let lower = value.to_ascii_lowercase();
    let (digits, mult) = match lower.chars().next_back()? {
        'k' => (&lower[..lower.len() - 1], 1024usize),
        'm' => (&lower[..lower.len() - 1], 1024 * 1024),
        'g' => (&lower[..lower.len() - 1], 1024 * 1024 * 1024),
        c if c.is_ascii_digit() => (lower.as_str(), 1),
        _ => return None,
    };
    digits.parse::<usize>().ok()?.checked_mul(mult)
}

/// NG-18 Phase C: parse `sub_filter*` directives in one scope.
pub(crate) fn parse_sub_filter(
    directives: &[Directive],
    report: &mut CompatibilityReport,
) -> Option<crate::ast::ParsedSubFilter> {
    let mut parsed = crate::ast::ParsedSubFilter::default();
    let mut seen = false;
    for d in directives {
        match d.name.as_str() {
            "sub_filter" => {
                seen = true;
                if d.args.len() != 2 {
                    report.push(
                        &d.loc,
                        "sub_filter",
                        CompatStatus::Error,
                        "sub_filter expects <pattern> <replacement>",
                        None,
                    );
                    continue;
                }
                let pattern = strip_quotes(&d.args[0]);
                let replacement = strip_quotes(&d.args[1]);
                if pattern.is_empty() {
                    report.push(
                        &d.loc,
                        "sub_filter",
                        CompatStatus::Error,
                        "sub_filter pattern must not be empty",
                        None,
                    );
                    continue;
                }
                if replacement.contains('$') {
                    // nginx interpolates $variables in the replacement; ExyonQ
                    // emits it literally, so a variable-bearing replacement is
                    // not equivalent.
                    report.push(
                        &d.loc,
                        "sub_filter",
                        CompatStatus::Unsupported,
                        format!(
                            "sub_filter `{pattern}` → `{replacement}` (nginx interpolates $variables; literal-only here)"
                        ),
                        None,
                    );
                    continue;
                }
                parsed.rules.push(exyonq_config_ir::SubFilterRule {
                    pattern: pattern.to_string(),
                    replacement: replacement.to_string(),
                });
                report.push(
                    &d.loc,
                    "sub_filter",
                    CompatStatus::Supported,
                    format!("sub_filter `{pattern}` → `{replacement}`"),
                    Some("[[route.sub_filter]] / modules.sub_filter".into()),
                );
            }
            "sub_filter_once" => {
                seen = true;
                match parse_on_off(d) {
                    Ok(v) => {
                        parsed.once = Some(v);
                        report.push(
                            &d.loc,
                            "sub_filter_once",
                            CompatStatus::Supported,
                            format!("sub_filter_once {}", if v { "on" } else { "off" }),
                            Some("modules.sub_filter.routes.<route>.once".into()),
                        );
                    }
                    Err(msg) => {
                        report.push(&d.loc, "sub_filter_once", CompatStatus::Error, msg, None)
                    }
                }
            }
            "sub_filter_types" => {
                seen = true;
                if d.args.is_empty() {
                    report.push(
                        &d.loc,
                        "sub_filter_types",
                        CompatStatus::Error,
                        "sub_filter_types expects at least one MIME type",
                        None,
                    );
                } else {
                    parsed.types = Some(d.args.clone());
                    report.push(
                        &d.loc,
                        "sub_filter_types",
                        CompatStatus::Supported,
                        format!("sub_filter_types {}", d.args.join(" ")),
                        Some("modules.sub_filter.routes.<route>.types".into()),
                    );
                }
            }
            "sub_filter_last_modified" => {
                seen = true;
                match parse_on_off(d) {
                    Ok(v) => parsed.last_modified = Some(v),
                    Err(msg) => report.push(
                        &d.loc,
                        "sub_filter_last_modified",
                        CompatStatus::Error,
                        msg,
                        None,
                    ),
                }
            }
            _ => {}
        }
    }
    seen.then_some(parsed)
}

/// NG-18 Phase D: parse `mirror` / `mirror_request_body` in one scope.
pub(crate) fn parse_mirror(
    directives: &[Directive],
    report: &mut CompatibilityReport,
) -> Option<crate::ast::ParsedMirror> {
    let mut parsed = crate::ast::ParsedMirror::default();
    let mut seen = false;
    for d in directives {
        match d.name.as_str() {
            "mirror" => {
                seen = true;
                if d.args.len() == 1 && d.args[0] == "off" {
                    parsed.off = true;
                    parsed.destinations.clear();
                    report.push(
                        &d.loc,
                        "mirror",
                        CompatStatus::Supported,
                        "mirror off → disables inherited mirrors".to_string(),
                        Some("modules.mirror.routes.<route>".into()),
                    );
                    continue;
                }
                if d.args.len() != 1 || !d.args[0].starts_with('/') {
                    report.push(
                        &d.loc,
                        "mirror",
                        CompatStatus::Error,
                        "mirror expects one internal path (starting with `/`) or `off`",
                        None,
                    );
                    continue;
                }
                parsed.off = false;
                parsed.destinations.push(d.args[0].clone());
                report.push(
                    &d.loc,
                    "mirror",
                    CompatStatus::Supported,
                    format!("mirror `{}` → modules.mirror", d.args[0]),
                    Some("modules.mirror.routes.<route>".into()),
                );
            }
            "mirror_request_body" => {
                seen = true;
                match parse_on_off(d) {
                    Ok(v) => {
                        parsed.request_body = Some(v);
                        report.push(
                            &d.loc,
                            "mirror_request_body",
                            CompatStatus::Supported,
                            format!(
                                "mirror_request_body {} → modules.mirror",
                                if v { "on" } else { "off" }
                            ),
                            Some("modules.mirror.routes.<route>.request_body".into()),
                        );
                    }
                    Err(msg) => report.push(
                        &d.loc,
                        "mirror_request_body",
                        CompatStatus::Error,
                        msg,
                        None,
                    ),
                }
            }
            _ => {}
        }
    }
    seen.then_some(parsed)
}

/// NG-18 Phase E: `slice <size>`.
pub(crate) fn parse_slice(
    directives: &[Directive],
    report: &mut CompatibilityReport,
) -> Option<crate::ast::ParsedSlice> {
    for d in directives {
        if d.name != "slice" {
            continue;
        }
        if d.args.len() != 1 {
            report.push(
                &d.loc,
                "slice",
                CompatStatus::Error,
                "slice expects one size argument",
                None,
            );
            continue;
        }
        let Some(size) = parse_nginx_size(&d.args[0]) else {
            report.push(
                &d.loc,
                "slice",
                CompatStatus::Error,
                format!("slice `{}` is not a valid size", d.args[0]),
                None,
            );
            continue;
        };
        if !(1024..=1_073_741_824).contains(&size) {
            report.push(
                &d.loc,
                "slice",
                CompatStatus::Error,
                format!("slice size {size} is outside 1024..=1073741824"),
                None,
            );
            continue;
        }
        report.push(
            &d.loc,
            "slice",
            CompatStatus::Supported,
            format!("slice {size} → modules.slice.routes"),
            Some("modules.slice.routes.<route>".into()),
        );
        return Some(crate::ast::ParsedSlice { size });
    }
    None
}

/// 0.5: `auth_basic` + `auth_basic_user_file`.
pub(crate) fn parse_auth(
    directives: &[Directive],
    report: &mut CompatibilityReport,
) -> Option<crate::ast::ParsedAuth> {
    let mut parsed = crate::ast::ParsedAuth::default();
    let mut seen = false;
    for d in directives {
        match d.name.as_str() {
            "auth_basic" => {
                seen = true;
                if d.args.len() != 1 {
                    report.push(
                        &d.loc,
                        "auth_basic",
                        CompatStatus::Error,
                        "auth_basic expects one argument",
                        None,
                    );
                    continue;
                }
                if d.args[0].eq_ignore_ascii_case("off") {
                    parsed.off = true;
                    parsed.realm = None;
                } else {
                    parsed.realm = Some(strip_quotes(&d.args[0]).to_string());
                }
                report.push(
                    &d.loc,
                    "auth_basic",
                    CompatStatus::Supported,
                    "auth_basic → modules.auth",
                    Some("modules.auth.routes.<route>".into()),
                );
            }
            "auth_basic_user_file" => {
                seen = true;
                if d.args.len() != 1 {
                    report.push(
                        &d.loc,
                        "auth_basic_user_file",
                        CompatStatus::Error,
                        "auth_basic_user_file expects one path",
                        None,
                    );
                    continue;
                }
                parsed.user_file = Some(d.args[0].clone());
                report.push(
                    &d.loc,
                    "auth_basic_user_file",
                    CompatStatus::Supported,
                    "auth_basic_user_file → modules.auth",
                    Some("modules.auth.routes.<route>".into()),
                );
            }
            _ => {}
        }
    }
    seen.then_some(parsed)
}

/// 0.5: `limit_req_zone $key zone=name:size rate=Nr/s`.
pub(crate) fn parse_limit_req_zone(
    directives: &[Directive],
    report: &mut CompatibilityReport,
) -> std::collections::BTreeMap<String, crate::ast::ParsedLimitZone> {
    let mut out = std::collections::BTreeMap::new();
    for d in directives {
        if d.name != "limit_req_zone" {
            continue;
        }
        let mut key = None;
        let mut zone = None;
        let mut rps = None;
        for arg in &d.args {
            if let Some(rest) = arg.strip_prefix("zone=") {
                zone = rest.split(':').next().map(str::to_string);
            } else if let Some(rest) = arg.strip_prefix("rate=") {
                if let Some(rate) = rest.strip_suffix("r/s") {
                    rps = rate.parse::<u32>().ok();
                } else if let Some(rate) = rest.strip_suffix("r/m") {
                    rps = rate.parse::<u32>().ok().map(|v| (v / 60).max(1));
                }
            } else if arg.starts_with('$') {
                key = Some(match arg.as_str() {
                    "$binary_remote_addr" | "$remote_addr" => "ip".to_string(),
                    "$host" => "host".to_string(),
                    _ if arg.starts_with("$http_") => {
                        format!("header:{}", arg["$http_".len()..].replace('_', "-"))
                    }
                    _ => String::new(),
                });
            }
        }
        let (Some(key), Some(zone), Some(rps)) = (key, zone, rps) else {
            report.push(
                &d.loc,
                "limit_req_zone",
                CompatStatus::Error,
                "limit_req_zone expects $key zone=name:size rate=Nr/s",
                None,
            );
            continue;
        };
        if key.is_empty() {
            report.push(
                &d.loc,
                "limit_req_zone",
                CompatStatus::Partial,
                "limit_req_zone key is not supported (ip/host/$http_*)",
                None,
            );
            continue;
        }
        report.push(
            &d.loc,
            "limit_req_zone",
            CompatStatus::Supported,
            format!("limit_req_zone `{zone}` → modules.ratelimit.zones"),
            Some("modules.ratelimit.zones".into()),
        );
        out.insert(
            zone,
            crate::ast::ParsedLimitZone {
                key,
                requests_per_second: rps,
            },
        );
    }
    out
}

/// 0.5: `limit_req zone=name burst=N nodelay`.
pub(crate) fn parse_limit_req(
    directives: &[Directive],
    report: &mut CompatibilityReport,
) -> Option<crate::ast::ParsedLimitReq> {
    for d in directives {
        if d.name != "limit_req" {
            continue;
        }
        let mut zone = None;
        let mut burst = 1;
        for arg in &d.args {
            if let Some(rest) = arg.strip_prefix("zone=") {
                zone = Some(rest.to_string());
            } else if let Some(rest) = arg.strip_prefix("burst=") {
                burst = rest.parse::<u32>().unwrap_or(1);
            }
        }
        let Some(zone) = zone else {
            report.push(
                &d.loc,
                "limit_req",
                CompatStatus::Error,
                "limit_req expects zone=name",
                None,
            );
            continue;
        };
        report.push(
            &d.loc,
            "limit_req",
            CompatStatus::Supported,
            format!("limit_req `{zone}` → modules.ratelimit.routes"),
            Some("modules.ratelimit.routes".into()),
        );
        return Some(crate::ast::ParsedLimitReq { zone, burst });
    }
    None
}

fn strip_quotes(value: &str) -> &str {
    let v = value.trim();
    if v.len() >= 2 {
        let bytes = v.as_bytes();
        if (bytes[0] == b'\'' && bytes[v.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[v.len() - 1] == b'"')
        {
            return &v[1..v.len() - 1];
        }
    }
    v
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
