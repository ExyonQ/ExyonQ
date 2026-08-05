//! Walk AST and extract server/location/upstream structures.

use crate::ast::{AnalyzedConfig, Config, Directive, ParsedServer, ParsedUpstream};
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
        _ => report_unknown(directive, "http", report),
    }
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
                    servers.push(target.clone());
                    if child.args.len() > 1 {
                        report.push(
                            &child.loc,
                            "server",
                            CompatStatus::Partial,
                            format!(
                                "upstream `{name}` server flags `{}` not represented in IR",
                                child.args[1..].join(" ")
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
                "least_conn" | "ip_hash" | "hash" => {
                    report.push(
                        &child.loc,
                        &child.name,
                        CompatStatus::Partial,
                        format!(
                            "upstream load method `{}` not represented; ExyonQ default balancing applies",
                            child.name
                        ),
                        None,
                    );
                }
                other => {
                    let status = if matches!(
                        other,
                        "weight" | "max_fails" | "fail_timeout" | "backup" | "down"
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
                classify_server_directive(child, report);
            }
        }
    }
    out.servers.push(server);
}

fn classify_server_directive(directive: &Directive, report: &mut CompatibilityReport) {
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
        "server_name" => classify_server_name_directive(directive, report),
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
