//! Map analyzed NGINX config to ExyonQ IR and render TOML.

use crate::ast::{AnalyzedConfig, LocationKind, ParsedLocation, ParsedServer};
use crate::location::{is_runtime_mappable, route_path};
use crate::report::{CompatStatus, CompatibilityReport};
use crate::rewrite::{is_safe_literal_redirect, parse_rewrite};
use crate::upstream::{
    map_fcgi_pools_from_upstreams, map_http_upstreams, resolve_fastcgi_upstream_ref,
    resolve_proxy_upstream_ref, NginxUpstreamIndex,
};
use exyonq_config_ir::{
    AppConfig, FcgiPoolConfig, ModulesConfig, RedirectConfig, RouteConfig, RouteMatch,
    ServerConfig, ServerNames, UpstreamConfig, CONFIG_VERSION_V1,
};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct MappedConfig {
    #[allow(dead_code)]
    pub app: AppConfig,
    pub toml: String,
}

pub fn map_to_ir(analyzed: &AnalyzedConfig, report: &mut CompatibilityReport) -> MappedConfig {
    let mut servers = Vec::new();
    let mut routes = Vec::new();

    let upstream_index = match NginxUpstreamIndex::build(&analyzed.upstreams, report) {
        Ok(idx) => idx,
        Err(()) => NginxUpstreamIndex {
            blocks: HashMap::new(),
        },
    };
    let mut upstreams = map_http_upstreams(&upstream_index, report);
    let mut pools_fcgi = map_fcgi_pools_from_upstreams(&upstream_index, report);

    for (server_index, server) in analyzed.servers.iter().enumerate() {
        let listen = map_listen(server, report);
        let server_names = map_server_names(server, report);
        let mut route_names = Vec::new();

        if let Some(route) = map_server_return(server_index, server, report) {
            route_names.push(route.name.clone());
            routes.push(route);
        } else {
            for (loc_index, location) in server.locations.iter().enumerate() {
                let route_name = format!("srv{}_loc{}", server_index + 1, loc_index + 1);
                if let Some(route) = map_location(
                    &route_name,
                    location,
                    server,
                    &upstream_index,
                    &mut upstreams,
                    &mut pools_fcgi,
                    report,
                ) {
                    route_names.push(route.name.clone());
                    routes.push(route);
                }
            }

            if route_names.is_empty() {
                if let Some(route) = server_default_route(server_index, server, report) {
                    route_names.push(route.name.clone());
                    routes.push(route);
                }
            }
        }

        servers.push(ServerConfig {
            listen,
            server_name: server_names,
            routes: route_names,
            tls: None,
            http3_listen: None,
        });
    }

    let app = AppConfig {
        config_version: CONFIG_VERSION_V1,
        includes: Vec::new(),
        servers,
        routes,
        upstreams,
        pools_fcgi,
        cache_policies: HashMap::new(),
        modules: ModulesConfig::default(),
        static_section: Default::default(),
        full_page_cache: Default::default(),
        http3: Default::default(),
    };

    report.summary.servers_parsed = analyzed.servers.len();
    report.summary.listeners_generated = app.servers.len();
    report.summary.sites_generated = app
        .servers
        .iter()
        .filter(|s| !s.server_name.is_empty())
        .count();
    report.summary.routes_generated = app.routes.len();
    report.summary.upstreams_generated = app.upstreams.len();
    report.summary.fcgi_pools_generated = app.pools_fcgi.len();

    let toml = render_toml(&app, &app.pools_fcgi);
    MappedConfig { app, toml }
}

fn map_listen(server: &ParsedServer, report: &mut CompatibilityReport) -> String {
    let mut listen_value = None;
    for d in &server.directives {
        if d.name != "listen" {
            continue;
        }
        let raw = d.args.join(" ");
        let mut parts: Vec<&str> = raw.split_whitespace().collect();
        let addr_port = parts.first().copied().unwrap_or("80");
        for flag in parts.drain(1..) {
            report.push(
                &d.loc,
                "listen",
                CompatStatus::Partial,
                format!("listen flag `{flag}` not represented in IR"),
                None,
            );
            let _ = flag;
        }
        listen_value = Some(normalize_listen_addr(addr_port));
    }
    listen_value.unwrap_or_else(|| "0.0.0.0:80".into())
}

fn normalize_listen_addr(raw: &str) -> String {
    if raw.starts_with('[') {
        if let Some(end) = raw.find(']') {
            let host = &raw[..=end];
            let rest = raw[end + 1..].trim_start_matches(':');
            if rest.is_empty() {
                return format!("{host}:80");
            }
            return format!("{host}:{rest}");
        }
    }
    if let Some((host, port)) = raw.rsplit_once(':') {
        if host.contains('.') || host == "*" || host.parse::<std::net::IpAddr>().is_ok() {
            let host = if host == "*" { "0.0.0.0" } else { host };
            return format!("{host}:{port}");
        }
    }
    if raw.chars().all(|c| c.is_ascii_digit()) {
        return format!("0.0.0.0:{raw}");
    }
    "0.0.0.0:80".to_string()
}

fn map_server_names(server: &ParsedServer, _report: &mut CompatibilityReport) -> ServerNames {
    let mut names = Vec::new();
    for d in &server.directives {
        if d.name != "server_name" {
            continue;
        }
        for arg in &d.args {
            if arg == "_" {
                continue;
            }
            if arg.starts_with('~') || arg.starts_with("~*") || arg.contains('*') {
                continue;
            }
            if !arg.is_empty() {
                names.push(arg.clone());
            }
        }
    }
    match names.len() {
        0 => ServerNames::None,
        1 => ServerNames::One(names.pop().unwrap()),
        _ => ServerNames::Many(names),
    }
}

fn server_root(server: &ParsedServer) -> Option<String> {
    server
        .directives
        .iter()
        .find(|d| d.name == "root")
        .and_then(|d| d.args.first())
        .cloned()
}

fn server_index_files(server: &ParsedServer) -> Option<String> {
    server
        .directives
        .iter()
        .find(|d| d.name == "index")
        .and_then(|d| d.args.first())
        .cloned()
}

fn location_root(
    location: &ParsedLocation,
    server: &ParsedServer,
    report: &mut CompatibilityReport,
) -> Option<String> {
    if let Some(local) = location
        .directives
        .iter()
        .find(|d| d.name == "root")
        .and_then(|d| d.args.first())
    {
        return Some(local.clone());
    }
    if let Some(inherited) = server_root(server) {
        report.summary.inherited_roots += 1;
        report.push(
            &location.loc,
            "root",
            CompatStatus::Supported,
            format!("inherited root `{inherited}` from server block"),
            Some(format!("route.document_root={inherited} (inherited)")),
        );
        return Some(inherited);
    }
    None
}

fn location_index(
    location: &ParsedLocation,
    server: &ParsedServer,
    report: &mut CompatibilityReport,
) -> Option<String> {
    if let Some(local) = location
        .directives
        .iter()
        .find(|d| d.name == "index")
        .and_then(|d| d.args.first())
    {
        d_extra_index_warnings(location, report);
        return Some(local.clone());
    }
    if let Some(inherited) = server_index_files(server) {
        report.summary.inherited_indexes += 1;
        report.push(
            &location.loc,
            "index",
            CompatStatus::Supported,
            format!("inherited index `{inherited}` from server block"),
            Some(format!("route.index={inherited} (inherited)")),
        );
        if let Some(d) = server.directives.iter().find(|d| d.name == "index") {
            for extra in d.args.iter().skip(1) {
                report.push(
                    &d.loc,
                    "index",
                    CompatStatus::Partial,
                    format!("extra index `{extra}` not represented; first index used"),
                    None,
                );
            }
        }
        return Some(inherited);
    }
    None
}

fn d_extra_index_warnings(location: &ParsedLocation, report: &mut CompatibilityReport) {
    if let Some(d) = location.directives.iter().find(|d| d.name == "index") {
        for extra in d.args.iter().skip(1) {
            report.push(
                &d.loc,
                "index",
                CompatStatus::Partial,
                format!("extra index `{extra}` not represented; first index used"),
                None,
            );
        }
    }
}

fn map_location(
    route_name: &str,
    location: &ParsedLocation,
    server: &ParsedServer,
    upstream_index: &NginxUpstreamIndex,
    upstreams: &mut HashMap<String, UpstreamConfig>,
    pools_fcgi: &mut HashMap<String, FcgiPoolConfig>,
    report: &mut CompatibilityReport,
) -> Option<RouteConfig> {
    if !is_runtime_mappable(location.kind) {
        return None;
    }

    let path = route_path(location);
    if location.kind != LocationKind::Prefix {
        report.push_migration(
            &location.loc,
            "location",
            CompatStatus::Partial,
            format!(
                "{} location `{}` mapped as literal path only",
                location.kind.as_str(),
                location.raw_pattern
            ),
            Some(format!("route match.path={path}")),
            Some("Review exact/preferential-prefix semantics vs NGINX".into()),
            Some(location.kind.as_str()),
            None,
            None,
        );
    }

    if let Some(msg) = conflicting_terminal_actions(location) {
        report.push(&location.loc, "location", CompatStatus::Error, msg, None);
        return None;
    }

    for d in &location.directives {
        if d.name == "return" {
            return map_return(route_name, &path, d, report);
        }
    }

    for d in &location.directives {
        if d.name == "rewrite" {
            if let Ok(rule) = parse_rewrite(&d.loc, &d.args) {
                if let Some((status, _from, target)) = is_safe_literal_redirect(&rule) {
                    return Some(RouteConfig {
                        name: route_name.to_string(),
                        r#match: RouteMatch {
                            path: path.clone(),
                            host: None,
                        },
                        upstream: None,
                        root: None,
                        index: None,
                        redirect: Some(RedirectConfig {
                            status,
                            location: target,
                        }),
                        rewrite: None,
                        fastcgi: None,
                        htaccess: Default::default(),
                        cache: None,
                    });
                }
            }
        }
    }

    for d in &location.directives {
        if d.name == "fastcgi_pass" {
            return map_fastcgi(
                route_name,
                &path,
                d,
                location,
                server,
                upstream_index,
                pools_fcgi,
                report,
            );
        }
    }

    for d in &location.directives {
        if d.name == "proxy_pass" {
            return map_proxy(route_name, &path, d, upstream_index, upstreams, report);
        }
    }

    let root = location_root(location, server, report);
    if let Some(root) = root {
        let index = location_index(location, server, report);
        if location.directives.iter().any(|d| d.name == "root") {
            report.push(
                &location.loc,
                "root",
                CompatStatus::Supported,
                format!("static root `{root}` mapped for `{path}`"),
                Some(format!("route.root={root}")),
            );
        }
        return Some(RouteConfig {
            name: route_name.to_string(),
            r#match: RouteMatch { path, host: None },
            upstream: None,
            root: Some(PathBuf::from(root)),
            index,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        });
    }

    None
}

fn conflicting_terminal_actions(location: &ParsedLocation) -> Option<String> {
    let mut terminals = Vec::new();
    for d in &location.directives {
        match d.name.as_str() {
            "return" => terminals.push("return"),
            "fastcgi_pass" => terminals.push("fastcgi_pass"),
            "proxy_pass" => terminals.push("proxy_pass"),
            "rewrite"
                if d.args.get(2).map(String::as_str) == Some("permanent")
                    || d.args.get(2).map(String::as_str) == Some("redirect") =>
            {
                terminals.push("rewrite(redirect)");
            }
            _ => {}
        }
    }
    if terminals.len() > 1 {
        Some(format!(
            "incompatible terminal directives in location: {}",
            terminals.join(", ")
        ))
    } else {
        None
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

fn map_proxy(
    route_name: &str,
    path: &str,
    d: &crate::ast::Directive,
    upstream_index: &NginxUpstreamIndex,
    upstreams: &mut HashMap<String, UpstreamConfig>,
    report: &mut CompatibilityReport,
) -> Option<RouteConfig> {
    let target = d.args.first()?;
    let upstream_name = if target.starts_with("http://") || target.starts_with("https://") {
        let scheme_stripped = target
            .trim_end_matches('/')
            .replacen("https://", "", 1)
            .replacen("http://", "", 1);
        if is_named_upstream_ref(&scheme_stripped) {
            if target.starts_with("https://") {
                report.push(
                    &d.loc,
                    "proxy_pass",
                    CompatStatus::Partial,
                    "https upstream group noted; ExyonQ upstream uses http:// scheme",
                    None,
                );
            }
            resolve_proxy_upstream_ref(&scheme_stripped, upstream_index, upstreams, &d.loc, report)?
        } else {
            let name = format!("{route_name}_upstream");
            let http_target = if target.starts_with("https://") {
                report.push(
                    &d.loc,
                    "proxy_pass",
                    CompatStatus::Partial,
                    "https upstream target noted; ExyonQ upstream uses http:// scheme",
                    None,
                );
                target.replacen("https://", "http://", 1)
            } else {
                target.clone()
            };
            upstreams.insert(
                name.clone(),
                UpstreamConfig {
                    name: name.clone(),
                    target: http_target,
                    timeout_ms: exyonq_config_ir::DEFAULT_UPSTREAM_TIMEOUT_MS,
                },
            );
            name
        }
    } else {
        resolve_proxy_upstream_ref(target, upstream_index, upstreams, &d.loc, report)?
    };

    report.push(
        &d.loc,
        "proxy_pass",
        CompatStatus::Supported,
        format!("proxy_pass `{target}` → upstream `{upstream_name}`"),
        Some(format!("route.upstream={upstream_name}")),
    );

    Some(RouteConfig {
        name: route_name.to_string(),
        r#match: RouteMatch {
            path: path.to_string(),
            host: None,
        },
        upstream: Some(upstream_name),
        root: None,
        index: None,
        redirect: None,
        rewrite: None,
        fastcgi: None,
        htaccess: Default::default(),
        cache: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn map_fastcgi(
    route_name: &str,
    path: &str,
    d: &crate::ast::Directive,
    location: &ParsedLocation,
    server: &ParsedServer,
    upstream_index: &NginxUpstreamIndex,
    pools_fcgi: &mut HashMap<String, FcgiPoolConfig>,
    report: &mut CompatibilityReport,
) -> Option<RouteConfig> {
    let target = d.args.first()?;
    let root = location_root(location, server, report);
    let root_path = root.map(PathBuf::from);

    if target.starts_with("unix:") {
        let pool_name = format!("{route_name}_fcgi");
        let address = target.clone();
        pools_fcgi.insert(
            pool_name.clone(),
            FcgiPoolConfig {
                name: pool_name.clone(),
                address,
                document_root: root_path.clone(),
                max_concurrency: exyonq_config_ir::DEFAULT_FCGI_MAX_CONCURRENCY,
                max_connections: None,
                transport: "unix".to_string(),
                idle_timeout_ms: 30_000,
                total_timeout_ms: 30_000,
                checkout_timeout_ms: 5_000,
            },
        );
        report.push(
            &d.loc,
            "fastcgi_pass",
            CompatStatus::Supported,
            format!("unix fastcgi_pass `{target}` → fcgi_pool `{pool_name}`"),
            Some(format!("fcgi_pool.{pool_name}")),
        );
        if let Some(ref r) = root_path {
            report.push(
                &location.loc,
                "root",
                CompatStatus::Supported,
                format!(
                    "document_root `{}` bound to fcgi_pool `{pool_name}`",
                    r.display()
                ),
                Some(format!(
                    "fcgi_pool.{pool_name}.document_root={}",
                    r.display()
                )),
            );
        } else {
            report.push(
                &d.loc,
                "fastcgi_pass",
                CompatStatus::Partial,
                "no document root for FastCGI route; set fcgi_pool.document_root in generated config",
                None,
            );
        }
        return Some(RouteConfig {
            name: route_name.to_string(),
            r#match: RouteMatch {
                path: path.to_string(),
                host: None,
            },
            upstream: None,
            root: None,
            index: location_index(location, server, report),
            redirect: None,
            rewrite: None,
            fastcgi: Some(pool_name),
            htaccess: Default::default(),
            cache: None,
        });
    }

    if target.contains(':') && !target.starts_with("http") {
        report.push(
            &d.loc,
            "fastcgi_pass",
            CompatStatus::Unsupported,
            format!("TCP fastcgi_pass `{target}` blocked (TCP productivo not enabled)"),
            None,
        );
        return None;
    }

    if upstream_index.blocks.contains_key(target) {
        let pool_name = resolve_fastcgi_upstream_ref(
            target,
            upstream_index,
            pools_fcgi,
            root_path,
            &d.loc,
            report,
        )?;
        return Some(RouteConfig {
            name: route_name.to_string(),
            r#match: RouteMatch {
                path: path.to_string(),
                host: None,
            },
            upstream: None,
            root: None,
            index: location_index(location, server, report),
            redirect: None,
            rewrite: None,
            fastcgi: Some(pool_name),
            htaccess: Default::default(),
            cache: None,
        });
    }

    report.push(
        &d.loc,
        "fastcgi_pass",
        CompatStatus::Unsupported,
        format!("fastcgi_pass target `{target}` not recognized"),
        None,
    );
    None
}

fn map_server_return(
    server_index: usize,
    server: &ParsedServer,
    report: &mut CompatibilityReport,
) -> Option<RouteConfig> {
    for d in &server.directives {
        if d.name == "return" {
            return map_return(&format!("srv{}_return", server_index + 1), "/", d, report);
        }
    }
    None
}

fn map_return(
    route_name: &str,
    path: &str,
    d: &crate::ast::Directive,
    report: &mut CompatibilityReport,
) -> Option<RouteConfig> {
    let status: u16 = match d.args.first().and_then(|v| v.parse().ok()) {
        Some(code) if (100..600).contains(&code) => code,
        _ => {
            report.push_migration(
                &d.loc,
                "return",
                CompatStatus::Error,
                format!(
                    "invalid return status `{}`",
                    d.args.first().unwrap_or(&String::new())
                ),
                None,
                None,
                None,
                None,
                None,
            );
            return None;
        }
    };
    let target = d.args.get(1).cloned();

    if status == 444 {
        report.push_migration(
            &d.loc,
            "return",
            CompatStatus::Unsupported,
            "return 444 (NGINX connection close) not representable in ExyonQ",
            None,
            Some("Map manually or use supported status code".into()),
            None,
            Some("444".into()),
            None,
        );
        return None;
    }

    if status == 204 && target.is_none() {
        report.push_migration(
            &d.loc,
            "return",
            CompatStatus::Partial,
            "return 204 has no dedicated IR empty-body response",
            None,
            Some("Requires static/empty response support in IR".into()),
            None,
            Some("204".into()),
            None,
        );
        return None;
    }

    match status {
        301 | 302 | 307 | 308 if target.is_some() => {
            let location = target.unwrap();
            let vars = crate::variables::scan_variables(&location);
            let var_names: Vec<_> = vars.iter().map(|v| v.name.clone()).collect();
            let has_complex_vars = location.contains('$')
                && !location.contains("$request_uri")
                && location.replace("$request_uri", "").contains('$');
            let compat = if has_complex_vars || location.contains('$') {
                CompatStatus::Partial
            } else {
                CompatStatus::Supported
            };
            let action = if compat == CompatStatus::Supported {
                "Safe to replace with HTTP redirect".into()
            } else {
                "Review NGINX variables; interpolation may differ".into()
            };
            report.push_migration(
                &d.loc,
                "return",
                compat,
                format!("return {status} `{location}`"),
                Some(format!("route.redirect.status={status}")),
                Some(action),
                None,
                Some(format!("{status} {location}")),
                if var_names.is_empty() {
                    None
                } else {
                    Some(var_names)
                },
            );
            Some(RouteConfig {
                name: route_name.to_string(),
                r#match: RouteMatch {
                    path: path.to_string(),
                    host: None,
                },
                upstream: None,
                root: None,
                index: None,
                redirect: Some(RedirectConfig { status, location }),
                rewrite: None,
                fastcgi: None,
                htaccess: Default::default(),
                cache: None,
            })
        }
        404 => {
            report.push_migration(
                &d.loc,
                "return",
                CompatStatus::Partial,
                "return 404 has no dedicated IR static response",
                None,
                Some("Map manually to not-found handling".into()),
                None,
                Some("404".into()),
                None,
            );
            None
        }
        200 | 403 if target.is_some() => {
            report.push_migration(
                &d.loc,
                "return",
                CompatStatus::Partial,
                format!(
                    "return {status} with body `{}` not supported in IR",
                    target.unwrap_or_default()
                ),
                None,
                Some("Requires static response body support in IR".into()),
                None,
                Some(format!(
                    "{status} {}",
                    d.args.get(1).unwrap_or(&String::new())
                )),
                None,
            );
            None
        }
        200 | 403 | 204 => {
            report.push_migration(
                &d.loc,
                "return",
                CompatStatus::Partial,
                format!("return {status} without representable IR action"),
                None,
                Some("Requires static/empty response support in IR".into()),
                None,
                Some(status.to_string()),
                None,
            );
            None
        }
        other => {
            report.push_migration(
                &d.loc,
                "return",
                CompatStatus::Unsupported,
                format!("return {other} not supported in Tier 2B"),
                None,
                Some("Use supported redirect or manual mapping".into()),
                None,
                Some(other.to_string()),
                None,
            );
            None
        }
    }
}

fn server_default_route(
    server_index: usize,
    server: &ParsedServer,
    report: &mut CompatibilityReport,
) -> Option<RouteConfig> {
    let root = server_root(server)?;
    report.push(
        &server.loc,
        "root",
        CompatStatus::Supported,
        format!("server root `{root}` mapped to default `/` route"),
        Some("route.root".into()),
    );
    Some(RouteConfig {
        name: format!("srv{}_root", server_index + 1),
        r#match: RouteMatch {
            path: "/".into(),
            host: None,
        },
        upstream: None,
        root: Some(PathBuf::from(root)),
        index: server_index_files(server),
        redirect: None,
        rewrite: None,
        fastcgi: None,
        htaccess: Default::default(),
        cache: None,
    })
}

fn render_toml(app: &AppConfig, pools: &HashMap<String, FcgiPoolConfig>) -> String {
    let mut out = String::new();
    out.push_str(&format!("config_version = {}\n\n", app.config_version));

    for server in &app.servers {
        out.push_str("[[server]]\n");
        out.push_str(&format!("listen = {:?}\n", server.listen));
        let names = server.server_name.to_vec();
        if names.len() == 1 {
            out.push_str(&format!("server_name = {:?}\n", names[0]));
        } else if names.len() > 1 {
            out.push_str("server_name = [");
            for (i, n) in names.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("{n:?}"));
            }
            out.push_str("]\n");
        }
        if !server.routes.is_empty() {
            out.push_str("routes = [");
            for (i, r) in server.routes.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("{r:?}"));
            }
            out.push_str("]\n");
        }
        out.push('\n');
    }

    for route in &app.routes {
        out.push_str("[[route]]\n");
        out.push_str(&format!("name = {:?}\n", route.name));
        out.push_str(&format!("match = {{ path = {:?}", route.r#match.path));
        if let Some(host) = &route.r#match.host {
            out.push_str(&format!(", host = {host:?}"));
        }
        out.push_str(" }\n");
        if let Some(upstream) = &route.upstream {
            out.push_str(&format!("upstream = {upstream:?}\n"));
        }
        if let Some(root) = &route.root {
            out.push_str(&format!("root = {:?}\n", root.display()));
        }
        if let Some(index) = &route.index {
            out.push_str(&format!("index = {index:?}\n"));
        }
        if let Some(redirect) = &route.redirect {
            out.push_str(&format!(
                "redirect = {{ status = {}, location = {:?} }}\n",
                redirect.status, redirect.location
            ));
        }
        if let Some(fastcgi) = &route.fastcgi {
            out.push_str(&format!("fastcgi = {fastcgi:?}\n"));
        }
        out.push('\n');
    }

    for upstream in app.upstreams.values() {
        out.push_str("[[upstream]]\n");
        out.push_str(&format!("name = {:?}\n", upstream.name));
        out.push_str(&format!("target = {:?}\n", upstream.target));
        out.push_str(&format!("timeout_ms = {}\n\n", upstream.timeout_ms));
    }

    let mut pool_names: Vec<_> = pools.keys().collect();
    pool_names.sort();
    for name in pool_names {
        let pool = &pools[name];
        out.push_str("[[fcgi_pool]]\n");
        out.push_str(&format!("name = {:?}\n", pool.name));
        out.push_str(&format!("address = {:?}\n", pool.address));
        if let Some(root) = &pool.document_root {
            out.push_str(&format!("document_root = {:?}\n", root.display()));
        }
        out.push_str(&format!("max_concurrency = {}\n\n", pool.max_concurrency));
    }

    out
}

pub fn validate_mapped(mapped: &MappedConfig) -> Result<(), exyonq_config_ir::ConfigError> {
    exyonq_config_ir::AppConfig::parse_str(&mapped.toml)?;
    Ok(())
}
