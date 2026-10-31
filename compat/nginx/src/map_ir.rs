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
//! Map analyzed NGINX config to ExyonQ IR and render TOML.

use crate::ast::{AnalyzedConfig, LocationKind, ParsedLocation, ParsedServer};
use crate::location::{is_runtime_mappable, route_match_from_location, route_path};
use crate::report::{CompatStatus, CompatibilityReport};
use crate::rewrite::{is_safe_literal_redirect, map_rewrite_to_ir, parse_rewrite};
use crate::upstream::{
    map_fcgi_pools_from_upstreams, map_http_upstreams, resolve_fastcgi_upstream_ref,
    resolve_proxy_upstream_ref, NginxUpstreamIndex,
};
use exyonq_config_ir::{
    deterministic_endpoint_id, AdminEndpointState, AppConfig, Endpoint, EndpointFailoverPolicy,
    EndpointSelectionPolicy, FcgiPoolConfig, MapConfig, MapInputKind, ModulesConfig,
    ProxyProtocolConfig, RedirectConfig, RouteConfig, RouteMatch, ServerConfig, ServerNames,
    UpstreamConfig, CONFIG_VERSION_V1,
};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct MappedConfig {
    #[allow(dead_code)]
    pub app: AppConfig,
    pub toml: String,
}

/// Inject static-mvp IR defaults (index.html) without changing Full-profile mapping.
pub(crate) fn apply_static_mvp_defaults(mapped: &mut MappedConfig) {
    let mut changed = false;
    for route in &mut mapped.app.routes {
        if route.root.is_some() && route.index.is_none() {
            route.index = Some("index.html".into());
            changed = true;
        }
    }
    if changed {
        mapped.toml = render_toml(&mapped.app, &mapped.app.pools_fcgi);
    }
}

/// Normalize reverse-proxy-mvp upstream targets (strip authority trailing slash).
pub(crate) fn normalize_proxy_mvp_upstreams(mapped: &mut MappedConfig) {
    let mut changed = false;
    for upstream in mapped.app.upstreams.values_mut() {
        if upstream.target.starts_with("http://") && upstream.target.ends_with('/') {
            let trimmed = upstream.target.trim_end_matches('/');
            if trimmed.len() > "http://".len() {
                upstream.target = trimmed.to_string();
                changed = true;
            }
        }
    }
    if changed {
        mapped.toml = render_toml(&mapped.app, &mapped.app.pools_fcgi);
    }
}

/// FastCGI/PHP MVP: never emit static `index` on FastCGI routes.
pub(crate) fn apply_fastcgi_php_mvp_defaults(mapped: &mut MappedConfig) {
    let mut changed = false;
    for route in &mut mapped.app.routes {
        if route.fastcgi.is_some() && route.index.is_some() {
            route.index = None;
            changed = true;
        }
    }
    if changed {
        mapped.toml = render_toml(&mapped.app, &mapped.app.pools_fcgi);
    }
}

pub fn map_to_ir(
    analyzed: &AnalyzedConfig,
    report: &mut CompatibilityReport,
    allow_tcp_fastcgi: bool,
) -> MappedConfig {
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
    let mut sub_filters: std::collections::BTreeMap<
        String,
        exyonq_config_ir::SubFilterRouteConfig,
    > = std::collections::BTreeMap::new();
    let mut mirror_routes: std::collections::BTreeMap<String, exyonq_config_ir::MirrorRouteConfig> =
        std::collections::BTreeMap::new();
    let mut slice_routes: std::collections::BTreeMap<String, exyonq_config_ir::SliceRouteConfig> =
        std::collections::BTreeMap::new();
    let mut auth_routes: std::collections::BTreeMap<String, exyonq_config_ir::AuthRouteConfig> =
        std::collections::BTreeMap::new();
    let mut rate_zones: std::collections::BTreeMap<String, exyonq_config_ir::RateLimitZoneConfig> =
        analyzed
            .limit_zones_http
            .iter()
            .map(|(name, zone)| {
                (
                    name.clone(),
                    exyonq_config_ir::RateLimitZoneConfig {
                        key: zone.key.clone(),
                        requests_per_second: zone.requests_per_second,
                        burst: 1,
                    },
                )
            })
            .collect();
    let mut rate_routes: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();

    for (server_index, server) in analyzed.servers.iter().enumerate() {
        let (listen, proxy_protocol_flag) = map_listen(server, report);
        let server_names = map_server_names(server, report);
        let mut route_names = Vec::new();
        // NG-17: nginx inherits `add_header` / `expires` into a location only
        // when that location declares neither. Map the server scope once.
        let server_headers = map_add_header_directives(&server.directives, report);
        let server_expires = map_expires_directive(&server.directives, report).flatten();
        let server_sub_filter = crate::analyze::parse_sub_filter(&server.directives, report);
        let server_mirror = crate::analyze::parse_mirror(&server.directives, report);
        let server_slice = crate::analyze::parse_slice(&server.directives, report);
        let server_auth = crate::analyze::parse_auth(&server.directives, report);
        if let Some(v) = server.gunzip {
            report.push(
                &server.loc,
                "gunzip",
                CompatStatus::Supported,
                format!(
                    "server gunzip {} → per-route [[route]] gunzip (Phase B)",
                    if v { "on" } else { "off" }
                ),
                Some(format!("[[route]] gunzip = {v}")),
            );
        }
        if let Some(total) = server.gunzip_buffers {
            report.push(
                &server.loc,
                "gunzip_buffers",
                CompatStatus::Supported,
                format!("server gunzip_buffers {total} is process-global in ExyonQ"),
                None,
            );
        }

        if let Some(mut route) = map_server_return(server_index, server, report) {
            route.gunzip = server.gunzip;
            route_names.push(route.name.clone());
            routes.push(route);
        } else {
            for (loc_index, location) in server.locations.iter().enumerate() {
                let route_name = format!("srv{}_loc{}", server_index + 1, loc_index + 1);
                if let Some(mut route) = map_location(
                    &route_name,
                    location,
                    server,
                    &upstream_index,
                    &mut upstreams,
                    &mut pools_fcgi,
                    report,
                    allow_tcp_fastcgi,
                ) {
                    // nginx inherits only when the location declares no `add_header`,
                    // even if every declared header was rejected by the denylist.
                    let declares_headers =
                        location.directives.iter().any(|d| d.name == "add_header");
                    let location_headers = map_add_header_directives(&location.directives, report);
                    route.response_headers = if declares_headers {
                        location_headers
                    } else {
                        server_headers.clone()
                    };
                    // NG-17: nginx `expires` → route cache_headers (`*` MIME).
                    // `Some(None)` (off/unsupported) overrides inheritance.
                    route.cache_headers = match map_expires_directive(&location.directives, report)
                    {
                        Some(setting) => setting,
                        None => server_expires.clone(),
                    };
                    // NG-18 / ADR-064: location gunzip overrides server/http.
                    let (location_gunzip, _) = map_gunzip_directive(&location.directives, report);
                    route.gunzip = location_gunzip.or(server.gunzip);
                    let location_sub_filter =
                        crate::analyze::parse_sub_filter(&location.directives, report);
                    if let Some(cfg) = merge_sub_filter(
                        location_sub_filter,
                        server_sub_filter.clone(),
                        analyzed.sub_filter_http.clone(),
                    ) {
                        sub_filters.insert(route.name.clone(), cfg);
                    }
                    let location_mirror =
                        crate::analyze::parse_mirror(&location.directives, report);
                    if let Some(cfg) = merge_mirror(
                        location_mirror,
                        server_mirror.clone(),
                        analyzed.mirror_http.clone(),
                    ) {
                        mirror_routes.insert(route.name.clone(), cfg);
                    }
                    let location_slice = crate::analyze::parse_slice(&location.directives, report);
                    if let Some(size) = location_slice
                        .or(server_slice)
                        .or(analyzed.slice_http)
                        .map(|s| s.size)
                    {
                        slice_routes.insert(
                            route.name.clone(),
                            exyonq_config_ir::SliceRouteConfig { size },
                        );
                    }
                    let location_auth = crate::analyze::parse_auth(&location.directives, report);
                    if let Some(cfg) = merge_auth(
                        location_auth,
                        server_auth.clone(),
                        analyzed.auth_http.clone(),
                    ) {
                        auth_routes.insert(route.name.clone(), cfg);
                    }
                    let location_limit =
                        crate::analyze::parse_limit_req(&location.directives, report);
                    if let Some(req) = location_limit
                        .or(server.limit_req.clone())
                        .or(analyzed.limit_req_http.clone())
                    {
                        if let Some(zone) = rate_zones.get_mut(&req.zone) {
                            zone.burst = req.burst;
                            rate_routes.insert(route.name.clone(), req.zone);
                        }
                    }
                    route_names.push(route.name.clone());
                    routes.push(route);
                }
            }

            if route_names.is_empty() {
                if let Some(mut route) = server_default_route(server_index, server, report) {
                    route.response_headers = server_headers.clone();
                    route.cache_headers = server_expires.clone();
                    route.gunzip = server.gunzip;
                    if let Some(cfg) = merge_sub_filter(
                        None,
                        server_sub_filter.clone(),
                        analyzed.sub_filter_http.clone(),
                    ) {
                        sub_filters.insert(route.name.clone(), cfg);
                    }
                    if let Some(cfg) =
                        merge_mirror(None, server_mirror.clone(), analyzed.mirror_http.clone())
                    {
                        mirror_routes.insert(route.name.clone(), cfg);
                    }
                    route_names.push(route.name.clone());
                    routes.push(route);
                }
            }
        }

        let error_pages = map_server_error_pages(server, report);
        servers.push(ServerConfig {
            listen,
            server_name: server_names,
            routes: route_names,
            tls: None,
            http3_listen: None,
            error_pages,
            proxy_protocol: proxy_protocol_flag.then(|| ProxyProtocolConfig {
                enabled: true,
                trusted: Vec::new(),
            }),
        });
    }

    let maps = map_nginx_maps(&analyzed.maps, report);
    let mut app = AppConfig {
        config_version: CONFIG_VERSION_V1,
        includes: Vec::new(),
        servers,
        routes,
        upstreams,
        pools_fcgi,
        cache_policies: HashMap::new(),
        maps,
        modules: ModulesConfig {
            // NG-11: accumulated set_real_ip_from / real_ip_header.
            real_ip: exyonq_config_ir::RealIpConfig {
                enabled: !analyzed.real_ip_trusted.is_empty(),
                trusted: analyzed.real_ip_trusted.clone(),
                header: analyzed
                    .real_ip_header
                    .clone()
                    .unwrap_or_else(|| "x-forwarded-for".to_string()),
            },
            gunzip: exyonq_config_ir::GunzipConfig {
                enabled: analyzed.gunzip_http.unwrap_or(false),
                buffer_bytes: analyzed.gunzip_buffers_http.unwrap_or(32 * 1024),
            },
            ..ModulesConfig::default()
        },
        static_section: Default::default(),
        full_page_cache: Default::default(),
        http3: Default::default(),
        waf: Default::default(),
        logging: Default::default(),
    };

    app.modules.sub_filter.routes = sub_filters;
    app.modules.mirror.routes = mirror_routes;
    app.modules.slice.routes = slice_routes;
    app.modules.auth.routes = auth_routes;
    if !rate_zones.is_empty() {
        app.modules.ratelimit.enabled = true;
        app.modules.ratelimit.zones = rate_zones;
        app.modules.ratelimit.routes = rate_routes;
    }
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

/// NG-05 / ADR-051: nginx `map $src $dst` → `[[map]]`.
fn map_nginx_maps(
    parsed: &[crate::ast::ParsedMap],
    report: &mut CompatibilityReport,
) -> HashMap<String, MapConfig> {
    let mut maps = HashMap::new();
    for m in parsed {
        let (input, header) = match map_source_to_input(&m.source_var) {
            Some(v) => v,
            None => {
                report.push(
                    &m.loc,
                    "map",
                    CompatStatus::Unsupported,
                    format!(
                        "map source `{}` not in ADR-051 allowlist ($host/$remote_addr/$request_uri/$scheme/$http_upgrade)",
                        m.source_var
                    ),
                    None,
                );
                continue;
            }
        };
        let cfg = MapConfig {
            name: m.dest_var.clone(),
            input,
            header,
            default: m.default.clone(),
            values: m.values.clone(),
        };
        report.push(
            &m.loc,
            "map",
            CompatStatus::Supported,
            format!("map → [[map]] name=`{}`", cfg.name),
            Some(format!("map.{}", cfg.name)),
        );
        maps.insert(cfg.name.clone(), cfg);
    }
    maps
}

fn map_source_to_input(src: &str) -> Option<(MapInputKind, Option<String>)> {
    match src.trim() {
        "$host" => Some((MapInputKind::Host, None)),
        "$remote_addr" => Some((MapInputKind::RemoteAddr, None)),
        "$request_uri" => Some((MapInputKind::RequestUri, None)),
        "$scheme" => Some((MapInputKind::Scheme, None)),
        "$http_upgrade" => Some((MapInputKind::Header, Some("Upgrade".into()))),
        other if other.starts_with("$http_") => {
            let raw = other.trim_start_matches("$http_");
            if raw.is_empty() {
                return None;
            }
            // nginx $http_foo_bar → Foo-Bar
            let mut name = String::new();
            for (i, part) in raw.split('_').enumerate() {
                if i > 0 {
                    name.push('-');
                }
                let mut chars = part.chars();
                if let Some(first) = chars.next() {
                    name.extend(first.to_uppercase());
                    name.extend(chars);
                }
            }
            Some((MapInputKind::Header, Some(name)))
        }
        _ => None,
    }
}

/// NG-04 / ADR-050: `error_page 404 /404.html` → server `error_pages`.
fn map_server_error_pages(
    server: &ParsedServer,
    report: &mut CompatibilityReport,
) -> HashMap<String, String> {
    let mut pages = HashMap::new();
    for d in &server.directives {
        if d.name != "error_page" {
            continue;
        }
        match parse_error_page_args(&d.args) {
            Ok(mapped) => {
                for (code, path) in mapped {
                    pages.insert(code.clone(), path.clone());
                    report.push(
                        &d.loc,
                        "error_page",
                        CompatStatus::Supported,
                        format!("error_page {code} → error_pages[\"{code}\"] = `{path}`"),
                        Some(format!("error_pages[\"{code}\"] = {path:?}")),
                    );
                }
            }
            Err(msg) => {
                report.push(&d.loc, "error_page", CompatStatus::Unsupported, msg, None);
            }
        }
    }
    // Location-scoped error_page is residual (server map only in v0.5).
    for loc in &server.locations {
        for d in &loc.directives {
            if d.name == "error_page" {
                report.push(
                    &d.loc,
                    "error_page",
                    CompatStatus::Unsupported,
                    "location-scoped error_page not mapped (ADR-050: server-scoped only)",
                    None,
                );
            }
        }
    }
    pages
}

fn parse_error_page_args(args: &[String]) -> Result<Vec<(String, String)>, String> {
    if args.len() < 2 {
        return Err("error_page requires status code(s) and a URI".into());
    }
    let uri = args.last().expect("len>=2").as_str();
    if uri.starts_with('@') {
        return Err(format!(
            "error_page named location `{uri}` not supported (ADR-050)"
        ));
    }
    if uri.starts_with("http://") || uri.starts_with("https://") || uri.starts_with("//") {
        return Err(format!(
            "error_page URL redirect `{uri}` not supported (ADR-050)"
        ));
    }
    if uri.starts_with('=') {
        return Err(format!(
            "error_page status rewrite `{uri}` not supported (ADR-050)"
        ));
    }
    if !uri.starts_with('/') || uri.contains('?') || uri.contains('#') {
        return Err(format!(
            "error_page URI `{uri}` must be an absolute path without query/fragment"
        ));
    }
    let mut codes = Vec::new();
    for arg in &args[..args.len() - 1] {
        if arg.starts_with('=') {
            return Err(format!(
                "error_page `{arg}` status rewrite not supported (ADR-050)"
            ));
        }
        let status: u16 = arg
            .parse()
            .map_err(|_| format!("error_page status `{arg}` must be an integer in 400..=599"))?;
        if !(400..=599).contains(&status) {
            return Err(format!("error_page status {status} must be in 400..=599"));
        }
        codes.push(status.to_string());
    }
    if codes.is_empty() {
        return Err("error_page requires at least one status code".into());
    }
    Ok(codes.into_iter().map(|c| (c, uri.to_string())).collect())
}

/// Returns `(listen_addr, proxy_protocol_flag)`.
fn map_listen(server: &ParsedServer, report: &mut CompatibilityReport) -> (String, bool) {
    let mut listen_value = None;
    let mut proxy_protocol = false;
    for d in &server.directives {
        if d.name != "listen" {
            continue;
        }
        let raw = d.args.join(" ");
        let mut parts: Vec<&str> = raw.split_whitespace().collect();
        let addr_port = parts.first().copied().unwrap_or("80");
        for flag in parts.drain(1..) {
            if flag == "proxy_protocol" {
                proxy_protocol = true;
                report.push(
                    &d.loc,
                    "listen",
                    CompatStatus::Supported,
                    "listen flag `proxy_protocol` mapped to servers.proxy_protocol.enabled",
                    None,
                );
                continue;
            }
            report.push(
                &d.loc,
                "listen",
                CompatStatus::Partial,
                format!("listen flag `{flag}` not represented in IR"),
                None,
            );
        }
        listen_value = Some(normalize_listen_addr(addr_port));
    }
    (
        listen_value.unwrap_or_else(|| "0.0.0.0:80".into()),
        proxy_protocol,
    )
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

#[allow(clippy::too_many_arguments)]
fn map_location(
    route_name: &str,
    location: &ParsedLocation,
    server: &ParsedServer,
    upstream_index: &NginxUpstreamIndex,
    upstreams: &mut HashMap<String, UpstreamConfig>,
    pools_fcgi: &mut HashMap<String, FcgiPoolConfig>,
    report: &mut CompatibilityReport,
    allow_tcp_fastcgi: bool,
) -> Option<RouteConfig> {
    if !is_runtime_mappable(location.kind) {
        return None;
    }

    let path = route_path(location);
    let route_match = route_match_from_location(location);
    if matches!(
        location.kind,
        LocationKind::Exact | LocationKind::PreferentialPrefix
    ) {
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
    } else if matches!(
        location.kind,
        LocationKind::RegexCaseSensitive | LocationKind::RegexCaseInsensitive
    ) {
        let hint = match (&route_match.suffix, &route_match.regex) {
            (Some(s), _) => format!("route match.suffix={s}"),
            (_, Some(r)) => format!("route match.regex={r}"),
            _ => format!("route match.path={path}"),
        };
        report.push_migration(
            &location.loc,
            "location",
            CompatStatus::Supported,
            format!(
                "{} location `{}` mapped to ExyonQ match (ADR-047)",
                location.kind.as_str(),
                location.raw_pattern
            ),
            Some(hint),
            None,
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
            return map_return(route_name, &route_match, d, report);
        }
    }

    for d in &location.directives {
        if d.name == "rewrite" {
            if let Ok(rule) = parse_rewrite(&d.loc, &d.args) {
                if let Some((status, _from, target)) = is_safe_literal_redirect(&rule) {
                    return Some(RouteConfig {
                        gunzip: None,
                        cache_tags: Vec::new(),
                        name: route_name.to_string(),
                        r#match: route_match.clone(),
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
                        fallback_rewrite: None,
                        try_uri: None,
                        try_uri_dir: None,
                        proxy_set_headers: Vec::new(),
                        cache_headers: None,
                        response_headers: Vec::new(),
                        location_rewrite: Vec::new(),
                    });
                }
                if let Some(rw) = map_rewrite_to_ir(&rule) {
                    return Some(RouteConfig {
                        gunzip: None,
                        cache_tags: Vec::new(),
                        name: route_name.to_string(),
                        r#match: route_match.clone(),
                        upstream: None,
                        root: None,
                        index: None,
                        redirect: None,
                        rewrite: Some(rw),
                        fastcgi: None,
                        htaccess: Default::default(),
                        cache: None,
                        fallback_rewrite: None,
                        try_uri: None,
                        try_uri_dir: None,
                        proxy_set_headers: Vec::new(),
                        cache_headers: None,
                        response_headers: Vec::new(),
                        location_rewrite: Vec::new(),
                    });
                }
            }
        }
    }

    for d in &location.directives {
        if d.name == "fastcgi_pass" {
            return map_fastcgi(
                route_name,
                &route_match,
                d,
                location,
                server,
                upstream_index,
                pools_fcgi,
                report,
                allow_tcp_fastcgi,
            );
        }
    }

    for d in &location.directives {
        if d.name == "proxy_pass" {
            return map_proxy(
                route_name,
                location,
                &route_match,
                d,
                upstream_index,
                upstreams,
                report,
            );
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
        let (fallback_rewrite, try_uri, try_uri_dir) =
            map_location_try_files(location).unwrap_or((None, None, None));
        return Some(RouteConfig {
            gunzip: None,
            cache_tags: Vec::new(),
            name: route_name.to_string(),
            r#match: route_match,
            upstream: None,
            root: Some(PathBuf::from(root)),
            index,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
            fallback_rewrite,
            try_uri,
            try_uri_dir,
            proxy_set_headers: Vec::new(),
            cache_headers: None,
            response_headers: Vec::new(),
            location_rewrite: Vec::new(),
        });
    }

    None
}

fn map_location_try_files(
    location: &ParsedLocation,
) -> Option<(Option<String>, Option<bool>, Option<bool>)> {
    use crate::try_files::{map_try_files_to_fallback, parse_try_files};
    for d in &location.directives {
        if d.name != "try_files" {
            continue;
        }
        let Ok(tf) = parse_try_files(&d.loc, &d.args) else {
            continue;
        };
        let mapped = map_try_files_to_fallback(&tf)?;
        return Some((
            Some(mapped.fallback_rewrite),
            Some(mapped.try_uri),
            Some(mapped.try_uri_dir),
        ));
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
    location: &ParsedLocation,
    route_match: &RouteMatch,
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
                UpstreamConfig::legacy(
                    name.clone(),
                    http_target,
                    exyonq_config_ir::DEFAULT_UPSTREAM_TIMEOUT_MS,
                ),
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

    let proxy_set_headers = map_location_proxy_set_headers(location, report);
    let location_rewrite = map_location_proxy_redirects(location, target, route_match, report);
    Some(RouteConfig {
        gunzip: None,
        cache_tags: Vec::new(),
        name: route_name.to_string(),
        r#match: route_match.clone(),
        upstream: Some(upstream_name),
        root: None,
        index: None,
        redirect: None,
        rewrite: None,
        fastcgi: None,
        htaccess: Default::default(),
        cache: None,
        fallback_rewrite: None,
        try_uri: None,
        try_uri_dir: None,
        proxy_set_headers,
        cache_headers: None,
        response_headers: Vec::new(),
        location_rewrite,
    })
}

/// NG-18 / 0.7: allowed variables in `proxy_redirect` `to`.
fn has_unsupported_redirect_var(value: &str, allow_captures: bool) -> bool {
    let mut rest = value;
    while let Some(idx) = rest.find('$') {
        rest = &rest[idx + 1..];
        let len = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .count();
        let var = &rest[..len];
        rest = &rest[len..];
        let ok = matches!(var, "scheme" | "host" | "server_port")
            || (allow_captures
                && matches!(var, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
        if !ok {
            return true;
        }
    }
    false
}

/// F3-R20: `proxy_redirect` → `route.location_rewrite` (`Location`,
/// `Content-Location`, `URI` rewrite on upstream responses).
fn map_location_proxy_redirects(
    location: &ParsedLocation,
    proxy_pass_target: &str,
    route_match: &RouteMatch,
    report: &mut CompatibilityReport,
) -> Vec<exyonq_config_ir::LocationRewriteConfig> {
    let mut out = Vec::new();
    for d in &location.directives {
        if d.name != "proxy_redirect" {
            continue;
        }
        let args: Vec<&str> = d.args.iter().map(String::as_str).collect();
        match args.as_slice() {
            ["off"] => {
                report.push(
                    &d.loc,
                    "proxy_redirect",
                    CompatStatus::Supported,
                    "proxy_redirect off → no location rewriting (ExyonQ default)",
                    None,
                );
            }
            ["default"] => {
                // nginx default: from = proxy_pass value, to = location path.
                let rule = exyonq_config_ir::LocationRewriteConfig {
                    from: format!("{}/", proxy_pass_target.trim_end_matches('/')),
                    to: format!("{}/", route_match.path.trim_end_matches('/')),
                };
                report.push(
                    &d.loc,
                    "proxy_redirect",
                    CompatStatus::Supported,
                    format!(
                        "proxy_redirect default → location_rewrite {} → {}",
                        rule.from, rule.to
                    ),
                    Some(format!("route.location_rewrite from={}", rule.from)),
                );
                out.push(rule);
            }
            [from, to] => {
                if let Some(re) = from.strip_prefix('~') {
                    if regex::Regex::new(re).is_err() {
                        report.push(
                            &d.loc,
                            "proxy_redirect",
                            CompatStatus::Error,
                            format!("proxy_redirect regex `{from}` is invalid"),
                            None,
                        );
                        continue;
                    }
                    if has_unsupported_redirect_var(to, true) {
                        report.push(
                            &d.loc,
                            "proxy_redirect",
                            CompatStatus::Partial,
                            format!(
                                "proxy_redirect `{from}` `{to}`: only $1..$9 and $scheme/$host/$server_port in `to`"
                            ),
                            None,
                        );
                        continue;
                    }
                    report.push(
                        &d.loc,
                        "proxy_redirect",
                        CompatStatus::Supported,
                        format!("proxy_redirect regex `{from}` `{to}` → location_rewrite"),
                        Some("route.location_rewrite".into()),
                    );
                    out.push(exyonq_config_ir::LocationRewriteConfig {
                        from: from.to_string(),
                        to: to.to_string(),
                    });
                    continue;
                }
                if from.contains('$')
                    || to.starts_with('~')
                    || has_unsupported_redirect_var(to, false)
                {
                    report.push(
                        &d.loc,
                        "proxy_redirect",
                        CompatStatus::Partial,
                        format!(
                            "proxy_redirect `{from}` `{to}`: only $scheme/$host/$server_port vars in `to` are supported"
                        ),
                        None,
                    );
                    continue;
                }
                if !(from.starts_with("http://") || from.starts_with("https://")) {
                    report.push(
                        &d.loc,
                        "proxy_redirect",
                        CompatStatus::Partial,
                        format!("proxy_redirect `{from}`: relative `from` not supported"),
                        None,
                    );
                    continue;
                }
                report.push(
                    &d.loc,
                    "proxy_redirect",
                    CompatStatus::Supported,
                    format!("proxy_redirect `{from}` `{to}` → location_rewrite"),
                    Some("route.location_rewrite".to_string()),
                );
                out.push(exyonq_config_ir::LocationRewriteConfig {
                    from: from.to_string(),
                    to: to.to_string(),
                });
            }
            _ => {
                report.push(
                    &d.loc,
                    "proxy_redirect",
                    CompatStatus::Error,
                    "proxy_redirect requires `off`, `default`, or `from to`",
                    None,
                );
            }
        }
    }
    out
}

/// NG-17: `expires <time|max|off|epoch>` → `cache_headers.by_mime."*"`.
///
/// ADR-061 only carries a positive `max_age` and applies to static 200s, so
/// `epoch`/`0`/`modified`/`@time` are reported instead of silently dropped.
fn map_expires_directive(
    directives: &[crate::ast::Directive],
    report: &mut CompatibilityReport,
) -> Option<Option<exyonq_config_ir::CacheHeadersConfig>> {
    let mut found: Option<&crate::ast::Directive> = None;
    for d in directives {
        if d.name != "expires" {
            continue;
        }
        if found.is_some() {
            report.push(
                &d.loc,
                "expires",
                CompatStatus::Error,
                "duplicate `expires` in the same scope",
                None,
            );
            return Some(None);
        }
        found = Some(d);
    }
    let d = found?;
    let Some(first) = d.args.first() else {
        report.push(
            &d.loc,
            "expires",
            CompatStatus::Error,
            "expires requires a time argument",
            None,
        );
        return Some(None);
    };
    let lower = first.to_ascii_lowercase();
    if lower == "modified" {
        report.push(
            &d.loc,
            "expires",
            CompatStatus::Unsupported,
            "expires modified depends on Last-Modified and is not mapped",
            None,
        );
        return Some(None);
    }
    if d.args.len() != 1 {
        report.push(
            &d.loc,
            "expires",
            CompatStatus::Error,
            "expires requires exactly one time argument",
            None,
        );
        return Some(None);
    }
    let arg = first.as_str();
    if lower == "off" {
        report.push(
            &d.loc,
            "expires",
            CompatStatus::Supported,
            "expires off → no Cache-Control",
            None,
        );
        return Some(None);
    }
    let max_age = match lower.as_str() {
        "max" => 315_360_000,
        "epoch" | "0" => {
            report.push(
                &d.loc,
                "expires",
                CompatStatus::Unsupported,
                "expires epoch/0 needs `Cache-Control: no-cache`; ADR-061 only carries a positive max_age",
                Some("add_header Cache-Control \"no-cache\"".into()),
            );
            return Some(None);
        }
        _ if lower.starts_with('@') || lower.starts_with('+') => {
            report.push(
                &d.loc,
                "expires",
                CompatStatus::Unsupported,
                format!("expires `{arg}` depends on the clock and is not mapped"),
                None,
            );
            return Some(None);
        }
        _ => match parse_nginx_expires_seconds(arg) {
            Some(secs) if (1..=315_360_000).contains(&secs) => secs,
            Some(_) => {
                report.push(
                    &d.loc,
                    "expires",
                    CompatStatus::Unsupported,
                    format!("expires `{arg}` is outside 1..=315360000s"),
                    None,
                );
                return Some(None);
            }
            None => {
                report.push(
                    &d.loc,
                    "expires",
                    CompatStatus::Error,
                    format!("expires `{arg}` is not a supported nginx time"),
                    None,
                );
                return Some(None);
            }
        },
    };
    report.push(
        &d.loc,
        "expires",
        CompatStatus::Partial,
        format!(
            "expires `{arg}` → cache_headers.by_mime.\"*\" max_age={max_age} (static responses only, ADR-061)"
        ),
        Some(format!(
            "[route.cache_headers.by_mime.\"*\"] max_age = {max_age}"
        )),
    );
    let mut by_mime = std::collections::BTreeMap::new();
    by_mime.insert(
        "*".to_string(),
        exyonq_config_ir::MimeCacheHeader {
            max_age,
            immutable: false,
        },
    );
    Some(Some(exyonq_config_ir::CacheHeadersConfig { by_mime }))
}

/// nginx time: `s|m|h|d|w|M|y` or bare seconds. `M` is 30 days, `y` 365.
fn parse_nginx_expires_seconds(s: &str) -> Option<u64> {
    let last = s.chars().next_back()?;
    let (digits, mult) = match last {
        's' => (&s[..s.len() - 1], 1u64),
        'm' => (&s[..s.len() - 1], 60),
        'h' => (&s[..s.len() - 1], 3_600),
        'd' => (&s[..s.len() - 1], 86_400),
        'w' => (&s[..s.len() - 1], 604_800),
        'M' => (&s[..s.len() - 1], 2_592_000),
        'y' => (&s[..s.len() - 1], 31_536_000),
        c if c.is_ascii_digit() => (s, 1),
        _ => return None,
    };
    let n: u64 = digits.parse().ok()?;
    n.checked_mul(mult)
}

/// ADR-062 / F1-R19: `add_header Name Value [always];` → `response_headers`.
fn map_add_header_directives(
    directives: &[crate::ast::Directive],
    report: &mut CompatibilityReport,
) -> Vec<exyonq_config_ir::ResponseHeaderConfig> {
    let mut out = Vec::new();
    for d in directives {
        if d.name != "add_header" {
            continue;
        }
        if d.args.len() < 2 {
            report.push(
                &d.loc,
                "add_header",
                CompatStatus::Error,
                "add_header requires name and value",
                None,
            );
            continue;
        }
        let name = d.args[0].clone();
        let mut always = false;
        let value_parts: Vec<&str> = if d.args.last().map(String::as_str) == Some("always") {
            always = true;
            d.args[1..d.args.len() - 1]
                .iter()
                .map(String::as_str)
                .collect()
        } else {
            d.args[1..].iter().map(String::as_str).collect()
        };
        let value = value_parts.join(" ");
        if is_response_header_denied_name(&name) {
            report.push(
                &d.loc,
                "add_header",
                CompatStatus::Unsupported,
                format!("add_header `{name}` denied by ADR-062 denylist"),
                None,
            );
            continue;
        }
        if value.contains('$') {
            let mut rest = value.as_str();
            let mut vars_ok = true;
            while let Some(idx) = rest.find('$') {
                rest = &rest[idx + 1..];
                if rest.starts_with('$') {
                    rest = &rest[1..];
                    continue;
                }
                let len = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .count();
                let var = &rest[..len];
                rest = &rest[len..];
                if !matches!(var, "host" | "uri") {
                    vars_ok = false;
                    break;
                }
            }
            if !vars_ok {
                report.push(
                    &d.loc,
                    "add_header",
                    CompatStatus::Unsupported,
                    format!(
                        "add_header value `{value}` uses vars outside ADR-062 allowlist ($host/$uri)"
                    ),
                    None,
                );
                continue;
            }
        }
        if out
            .iter()
            .any(|e: &exyonq_config_ir::ResponseHeaderConfig| e.name.eq_ignore_ascii_case(&name))
        {
            report.push(
                &d.loc,
                "add_header",
                CompatStatus::Unsupported,
                format!("duplicate add_header `{name}` in same scope (ADR-062)"),
                None,
            );
            continue;
        }
        report.push(
            &d.loc,
            "add_header",
            CompatStatus::Supported,
            format!("add_header `{name}` → response_headers (always={always})"),
            Some(format!(
                "response_headers {{ name={name:?} value={value:?} always={always} }}"
            )),
        );
        out.push(exyonq_config_ir::ResponseHeaderConfig {
            name,
            value,
            always,
        });
    }
    out
}

fn is_response_header_denied_name(name: &str) -> bool {
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

/// Hop-by-hop names IR rejects on `proxy_set_headers` (ADR-051).
/// `Connection` is intentionally absent — allowed when map-driven.
fn is_proxy_set_hop_by_hop_denied(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// NG-05 / ADR-051: `proxy_set_header Name $var|literal`.
fn map_location_proxy_set_headers(
    location: &ParsedLocation,
    report: &mut CompatibilityReport,
) -> Vec<exyonq_config_ir::ProxySetHeaderConfig> {
    let mut out = Vec::new();
    for d in &location.directives {
        if d.name != "proxy_set_header" {
            continue;
        }
        if d.args.len() < 2 {
            report.push(
                &d.loc,
                "proxy_set_header",
                CompatStatus::Error,
                "proxy_set_header requires name and value",
                None,
            );
            continue;
        }
        let name = d.args[0].clone();
        let value = d.args[1..].join(" ");
        if is_proxy_set_hop_by_hop_denied(&name) {
            report.push(
                &d.loc,
                "proxy_set_header",
                CompatStatus::Unsupported,
                format!(
                    "proxy_set_header `{name}` is hop-by-hop (ADR-051); use [[map]] on Upgrade + Connection $map"
                ),
                None,
            );
            continue;
        }
        // No `$http_*` (F1 residual / ADR-051 allowlist).
        if value.contains("$http_") {
            report.push(
                &d.loc,
                "proxy_set_header",
                CompatStatus::Unsupported,
                format!("proxy_set_header value `{value}` uses $http_* (ADR-051 allowlist)"),
                None,
            );
            continue;
        }
        // Allowlisted templates only (builtins + $map refs validated later at IR load).
        let ok = value.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '$' | '_' | '-' | '.' | '/' | ':' | ' ')
        }) && !value.contains("${");
        if !ok {
            report.push(
                &d.loc,
                "proxy_set_header",
                CompatStatus::Unsupported,
                format!("proxy_set_header value `{value}` not representable (ADR-051)"),
                None,
            );
            continue;
        }
        report.push(
            &d.loc,
            "proxy_set_header",
            CompatStatus::Supported,
            format!("proxy_set_header `{name}` → proxy_set_headers"),
            Some(format!(
                "proxy_set_headers {{ name={name:?} value={value:?} }}"
            )),
        );
        out.push(exyonq_config_ir::ProxySetHeaderConfig { name, value });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn map_fastcgi(
    route_name: &str,
    route_match: &RouteMatch,
    d: &crate::ast::Directive,
    location: &ParsedLocation,
    server: &ParsedServer,
    upstream_index: &NginxUpstreamIndex,
    pools_fcgi: &mut HashMap<String, FcgiPoolConfig>,
    report: &mut CompatibilityReport,
    allow_tcp_fastcgi: bool,
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
            gunzip: None,
            cache_tags: Vec::new(),
            name: route_name.to_string(),
            r#match: route_match.clone(),
            upstream: None,
            root: None,
            index: location_index(location, server, report),
            redirect: None,
            rewrite: None,
            fastcgi: Some(pool_name),
            htaccess: Default::default(),
            cache: None,
            fallback_rewrite: None,
            try_uri: None,
            try_uri_dir: None,
            proxy_set_headers: Vec::new(),
            cache_headers: None,
            response_headers: Vec::new(),
            location_rewrite: Vec::new(),
        });
    }

    if target.contains(':') && !target.starts_with("http") {
        if allow_tcp_fastcgi && target.parse::<std::net::SocketAddr>().is_ok() {
            let pool_name = format!("{route_name}_fcgi");
            pools_fcgi.insert(
                pool_name.clone(),
                FcgiPoolConfig {
                    name: pool_name.clone(),
                    address: target.clone(),
                    document_root: root_path.clone(),
                    max_concurrency: exyonq_config_ir::DEFAULT_FCGI_MAX_CONCURRENCY,
                    max_connections: None,
                    transport: "tcp".to_string(),
                    idle_timeout_ms: 30_000,
                    total_timeout_ms: 30_000,
                    checkout_timeout_ms: 5_000,
                },
            );
            report.push(
                &d.loc,
                "fastcgi_pass",
                CompatStatus::Supported,
                format!("tcp fastcgi_pass `{target}` → fcgi_pool `{pool_name}`"),
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
                gunzip: None,
                cache_tags: Vec::new(),
                name: route_name.to_string(),
                r#match: route_match.clone(),
                upstream: None,
                root: None,
                index: None,
                redirect: None,
                rewrite: None,
                fastcgi: Some(pool_name),
                htaccess: Default::default(),
                cache: None,
                fallback_rewrite: None,
                try_uri: None,
                try_uri_dir: None,
                proxy_set_headers: Vec::new(),
                cache_headers: None,
                response_headers: Vec::new(),
                location_rewrite: Vec::new(),
            });
        }
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
            gunzip: None,
            cache_tags: Vec::new(),
            name: route_name.to_string(),
            r#match: route_match.clone(),
            upstream: None,
            root: None,
            index: location_index(location, server, report),
            redirect: None,
            rewrite: None,
            fastcgi: Some(pool_name),
            htaccess: Default::default(),
            cache: None,
            fallback_rewrite: None,
            try_uri: None,
            try_uri_dir: None,
            proxy_set_headers: Vec::new(),
            cache_headers: None,
            response_headers: Vec::new(),
            location_rewrite: Vec::new(),
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
            return map_return(
                &format!("srv{}_return", server_index + 1),
                &RouteMatch::prefix("/"),
                d,
                report,
            );
        }
    }
    None
}

fn map_return(
    route_name: &str,
    route_match: &RouteMatch,
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
                gunzip: None,
                cache_tags: Vec::new(),
                name: route_name.to_string(),
                r#match: route_match.clone(),
                upstream: None,
                root: None,
                index: None,
                redirect: Some(RedirectConfig { status, location }),
                rewrite: None,
                fastcgi: None,
                htaccess: Default::default(),
                cache: None,
                fallback_rewrite: None,
                try_uri: None,
                try_uri_dir: None,
                proxy_set_headers: Vec::new(),
                cache_headers: None,
                response_headers: Vec::new(),
                location_rewrite: Vec::new(),
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
        gunzip: None,
        cache_tags: Vec::new(),
        name: format!("srv{}_root", server_index + 1),
        r#match: RouteMatch {
            path: "/".into(),
            host: None,

            ..Default::default()
        },
        upstream: None,
        root: Some(PathBuf::from(root)),
        index: server_index_files(server),
        redirect: None,
        rewrite: None,
        fastcgi: None,
        htaccess: Default::default(),
        cache: None,
        fallback_rewrite: None,
        try_uri: None,
        try_uri_dir: None,
        proxy_set_headers: Vec::new(),
        cache_headers: None,
        response_headers: Vec::new(),
        location_rewrite: Vec::new(),
    })
}

/// TOML spelling of endpoint admin state (serde `snake_case`).
fn admin_state_toml(state: AdminEndpointState) -> &'static str {
    match state {
        AdminEndpointState::Enabled => "enabled",
        AdminEndpointState::Disabled => "disabled",
        AdminEndpointState::DrainRequested => "drain_requested",
    }
}

/// NG-18 / ADR-064: location-scope `gunzip` / `gunzip_buffers`.
fn map_gunzip_directive(
    directives: &[crate::ast::Directive],
    report: &mut CompatibilityReport,
) -> (Option<bool>, Option<usize>) {
    let mut gunzip = None;
    let mut buffers = None;
    for d in directives {
        match d.name.as_str() {
            "gunzip" => match crate::analyze::parse_on_off(d) {
                Ok(v) => {
                    gunzip = Some(v);
                    report.push(
                        &d.loc,
                        "gunzip",
                        CompatStatus::Supported,
                        format!(
                            "location gunzip {} → [[route]] gunzip (Phase B)",
                            if v { "on" } else { "off" }
                        ),
                        Some(format!("[[route]] gunzip = {v}")),
                    );
                }
                Err(msg) => report.push(&d.loc, "gunzip", CompatStatus::Error, msg, None),
            },
            "gunzip_buffers" => match crate::analyze::parse_gunzip_buffers(d) {
                Ok(total) => {
                    buffers = Some(total);
                    report.push(
                        &d.loc,
                        "gunzip_buffers",
                        CompatStatus::Supported,
                        format!("location gunzip_buffers {total} is process-global in ExyonQ"),
                        Some(format!("[modules.gunzip] buffer_bytes = {total}")),
                    );
                }
                Err(msg) => report.push(&d.loc, "gunzip_buffers", CompatStatus::Error, msg, None),
            },
            _ => {}
        }
    }
    (gunzip, buffers)
}

/// NG-18 Phase C: nginx inheritance. Rules: nearest non-empty scope wins;
/// scalar flags inherit individually.
fn merge_sub_filter(
    location: Option<crate::ast::ParsedSubFilter>,
    server: Option<crate::ast::ParsedSubFilter>,
    http: Option<crate::ast::ParsedSubFilter>,
) -> Option<exyonq_config_ir::SubFilterRouteConfig> {
    if location.is_none() && server.is_none() && http.is_none() {
        return None;
    }
    let rules = location
        .as_ref()
        .filter(|p| !p.rules.is_empty())
        .map(|p| p.rules.clone())
        .or_else(|| {
            server
                .as_ref()
                .filter(|p| !p.rules.is_empty())
                .map(|p| p.rules.clone())
        })
        .or_else(|| {
            http.as_ref()
                .filter(|p| !p.rules.is_empty())
                .map(|p| p.rules.clone())
        })
        .unwrap_or_default();
    let once = location
        .as_ref()
        .and_then(|p| p.once)
        .or_else(|| server.as_ref().and_then(|p| p.once))
        .or_else(|| http.as_ref().and_then(|p| p.once))
        .unwrap_or(true);
    let types = location
        .as_ref()
        .and_then(|p| p.types.clone())
        .or_else(|| server.as_ref().and_then(|p| p.types.clone()))
        .or_else(|| http.as_ref().and_then(|p| p.types.clone()))
        .unwrap_or_else(|| vec!["text/html".to_string()]);
    let last_modified = location
        .as_ref()
        .and_then(|p| p.last_modified)
        .or_else(|| server.as_ref().and_then(|p| p.last_modified))
        .or_else(|| http.as_ref().and_then(|p| p.last_modified))
        .unwrap_or(false);
    Some(exyonq_config_ir::SubFilterRouteConfig {
        rules,
        once,
        types,
        last_modified,
    })
}

/// NG-18 Phase D: nginx mirror inheritance. The nearest scope with `mirror`
/// (destinations or `off`) wins; `mirror_request_body` is a separate scalar
/// inherited from the nearest explicit scope (nginx 1.27 default: on).
fn merge_mirror(
    location: Option<crate::ast::ParsedMirror>,
    server: Option<crate::ast::ParsedMirror>,
    http: Option<crate::ast::ParsedMirror>,
) -> Option<exyonq_config_ir::MirrorRouteConfig> {
    let scopes = [&location, &server, &http];
    let nearest = scopes
        .into_iter()
        .flatten()
        .find(|p| p.off || !p.destinations.is_empty())?;
    if nearest.off {
        return None;
    }
    let request_body = scopes
        .into_iter()
        .flatten()
        .find_map(|p| p.request_body)
        .unwrap_or(true);
    Some(exyonq_config_ir::MirrorRouteConfig {
        destinations: nearest.destinations.clone(),
        request_body,
    })
}

fn merge_auth(
    location: Option<crate::ast::ParsedAuth>,
    server: Option<crate::ast::ParsedAuth>,
    http: Option<crate::ast::ParsedAuth>,
) -> Option<exyonq_config_ir::AuthRouteConfig> {
    let cfg = location.or(server).or(http)?;
    if cfg.off {
        return None;
    }
    Some(exyonq_config_ir::AuthRouteConfig {
        realm: cfg.realm?,
        user_file: cfg.user_file?,
    })
}

fn render_toml(app: &AppConfig, pools: &HashMap<String, FcgiPoolConfig>) -> String {
    let mut out = String::new();
    out.push_str(&format!("config_version = {}\n\n", app.config_version));
    if app.modules.gunzip.enabled || app.modules.gunzip.buffer_bytes != 32 * 1024 {
        out.push_str("[modules.gunzip]\n");
        out.push_str(&format!("enabled = {}\n", app.modules.gunzip.enabled));
        out.push_str(&format!(
            "buffer_bytes = {}\n\n",
            app.modules.gunzip.buffer_bytes
        ));
    }
    for (route, cfg) in &app.modules.sub_filter.routes {
        out.push_str(&format!("[modules.sub_filter.routes.{route:?}]\n"));
        out.push_str(&format!("once = {}\n", cfg.once));
        out.push_str(&format!("last_modified = {}\n", cfg.last_modified));
        out.push_str(&format!(
            "types = [{}]\n",
            cfg.types
                .iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for rule in &cfg.rules {
            out.push_str(&format!("[[modules.sub_filter.routes.{route:?}.rules]]\n"));
            out.push_str(&format!("pattern = {:?}\n", rule.pattern));
            out.push_str(&format!("replacement = {:?}\n", rule.replacement));
        }
        out.push('\n');
    }

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
        if !server.error_pages.is_empty() {
            out.push_str("error_pages = { ");
            let mut keys: Vec<_> = server.error_pages.keys().collect();
            keys.sort();
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let v = &server.error_pages[*k];
                out.push_str(&format!("{k:?} = {v:?}"));
            }
            out.push_str(" }\n");
        }
        if let Some(pp) = &server.proxy_protocol {
            let trusted = pp
                .trusted
                .iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "proxy_protocol = {{ enabled = {}, trusted = [{}] }}\n",
                pp.enabled, trusted
            ));
        }
        out.push('\n');
    }
    for (route, cfg) in &app.modules.mirror.routes {
        out.push_str(&format!("[modules.mirror.routes.{route:?}]\n"));
        out.push_str(&format!(
            "destinations = [{}]\n",
            cfg.destinations
                .iter()
                .map(|d| format!("{d:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str(&format!("request_body = {}\n\n", cfg.request_body));
    }
    for (route, cfg) in &app.modules.slice.routes {
        out.push_str(&format!("[modules.slice.routes.{route:?}]\n"));
        out.push_str(&format!("size = {}\n\n", cfg.size));
    }
    for (route, cfg) in &app.modules.auth.routes {
        out.push_str(&format!("[modules.auth.routes.{route:?}]\n"));
        out.push_str(&format!("realm = {:?}\n", cfg.realm));
        out.push_str(&format!("user_file = {:?}\n\n", cfg.user_file));
    }
    if !app.modules.ratelimit.zones.is_empty() {
        out.push_str("[modules.ratelimit]\nenabled = true\n");
        for (name, zone) in &app.modules.ratelimit.zones {
            out.push_str(&format!("[modules.ratelimit.zones.{name:?}]\n"));
            out.push_str(&format!("key = {:?}\n", zone.key));
            out.push_str(&format!(
                "requests_per_second = {}\n",
                zone.requests_per_second
            ));
            out.push_str(&format!("burst = {}\n", zone.burst));
        }
        if !app.modules.ratelimit.routes.is_empty() {
            out.push_str("[modules.ratelimit.routes]\n");
            for (route, zone) in &app.modules.ratelimit.routes {
                out.push_str(&format!("{route:?} = {zone:?}\n"));
            }
        }
        out.push('\n');
    }

    for route in &app.routes {
        out.push_str("[[route]]\n");
        out.push_str(&format!("name = {:?}\n", route.name));
        out.push_str("match = { ");
        if let Some(suffix) = &route.r#match.suffix {
            out.push_str(&format!("suffix = {suffix:?}"));
        } else if let Some(regex) = &route.r#match.regex {
            out.push_str(&format!("regex = {regex:?}"));
        } else {
            out.push_str(&format!("path = {:?}", route.r#match.path));
        }
        if let Some(host) = &route.r#match.host {
            out.push_str(&format!(", host = {host:?}"));
        }
        if let Some(header) = &route.r#match.header {
            out.push_str(&format!(", header = {header:?}"));
            if let Some(hv) = &route.r#match.header_value {
                out.push_str(&format!(", header_value = {hv:?}"));
            }
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
        if let Some(fb) = &route.fallback_rewrite {
            out.push_str(&format!("fallback_rewrite = {fb:?}\n"));
            if let Some(v) = route.try_uri {
                out.push_str(&format!("try_uri = {}\n", if v { "true" } else { "false" }));
            }
            if let Some(v) = route.try_uri_dir {
                out.push_str(&format!(
                    "try_uri_dir = {}\n",
                    if v { "true" } else { "false" }
                ));
            }
        }
        if let Some(redirect) = &route.redirect {
            out.push_str(&format!(
                "redirect = {{ status = {}, location = {:?} }}\n",
                redirect.status, redirect.location
            ));
        }
        if let Some(rewrite) = &route.rewrite {
            match rewrite {
                exyonq_config_ir::RewriteConfig::Literal(path) => {
                    out.push_str(&format!("rewrite = {path:?}\n"));
                }
                exyonq_config_ir::RewriteConfig::Rule(rule) => {
                    let flag = match rule.flag {
                        exyonq_config_ir::RewriteFlagConfig::Last => "last",
                        exyonq_config_ir::RewriteFlagConfig::Break => "break",
                        exyonq_config_ir::RewriteFlagConfig::Redirect => "redirect",
                        exyonq_config_ir::RewriteFlagConfig::Permanent => "permanent",
                    };
                    out.push_str(&format!(
                        "rewrite = {{ pattern = {:?}, replacement = {:?}, flag = {:?} }}\n",
                        rule.pattern, rule.replacement, flag
                    ));
                }
            }
        }
        if let Some(fastcgi) = &route.fastcgi {
            out.push_str(&format!("fastcgi = {fastcgi:?}\n"));
        }
        if !route.proxy_set_headers.is_empty() {
            out.push_str("proxy_set_headers = [\n");
            for h in &route.proxy_set_headers {
                out.push_str(&format!(
                    "  {{ name = {:?}, value = {:?} }},\n",
                    h.name, h.value
                ));
            }
            out.push_str("]\n");
        }
        if let Some(gunzip) = route.gunzip {
            out.push_str(&format!("gunzip = {gunzip}\n"));
        }
        if let Some(ch) = &route.cache_headers {
            for (mime, h) in &ch.by_mime {
                out.push_str(&format!("[route.cache_headers.by_mime.{mime:?}]\n"));
                out.push_str(&format!("max_age = {}\n", h.max_age));
                if h.immutable {
                    out.push_str("immutable = true\n");
                }
            }
        }
        for h in &route.response_headers {
            out.push_str("[[route.response_headers]]\n");
            out.push_str(&format!("name = {:?}\n", h.name));
            out.push_str(&format!("value = {:?}\n", h.value));
            if h.always {
                out.push_str("always = true\n");
            }
        }
        for lr in &route.location_rewrite {
            out.push_str("[[route.location_rewrite]]\n");
            out.push_str(&format!("from = {:?}\n", lr.from));
            out.push_str(&format!("to = {:?}\n", lr.to));
        }
        out.push('\n');
    }

    let mut map_names: Vec<_> = app.maps.keys().collect();
    map_names.sort();
    for name in map_names {
        let map = &app.maps[name];
        out.push_str("[[map]]\n");
        out.push_str(&format!("name = {:?}\n", map.name));
        let input = match map.input {
            MapInputKind::Header => "header",
            MapInputKind::Host => "host",
            MapInputKind::RemoteAddr => "remote_addr",
            MapInputKind::RequestUri => "request_uri",
            MapInputKind::Scheme => "scheme",
        };
        out.push_str(&format!("input = {input:?}\n"));
        if let Some(h) = &map.header {
            out.push_str(&format!("header = {h:?}\n"));
        }
        out.push_str(&format!("default = {:?}\n", map.default));
        if !map.values.is_empty() {
            out.push_str("values = { ");
            let mut keys: Vec<_> = map.values.keys().collect();
            keys.sort();
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("{k:?} = {:?}", map.values[*k]));
            }
            out.push_str(" }\n");
        }
        out.push('\n');
    }

    for upstream in app.upstreams.values() {
        out.push_str("[[upstream]]\n");
        out.push_str(&format!("name = {:?}\n", upstream.name));
        let eps = upstream.endpoint_set.endpoints();
        // NG-07: `target` shorthand is only valid for the plain single-endpoint
        // case — weight/priority/admin_state must not be silently dropped.
        // NG-08: passive-health params count too — a tuned single endpoint
        // must keep its explicit `endpoints` row.
        let legacy_single = eps.len() == 1
            && eps[0].weight == Endpoint::DEFAULT_WEIGHT
            && eps[0].priority == Endpoint::DEFAULT_PRIORITY
            && eps[0].admin_state == AdminEndpointState::Enabled
            && eps[0].max_fails == Endpoint::DEFAULT_MAX_FAILS
            && eps[0].fail_timeout_ms == Endpoint::DEFAULT_FAIL_TIMEOUT_MS
            && eps[0].slow_start_ms == Endpoint::DEFAULT_SLOW_START_MS
            && eps[0].endpoint_id
                == deterministic_endpoint_id(
                    &eps[0].address,
                    eps[0].port,
                    eps[0].weight,
                    eps[0].priority,
                );
        if eps.len() > 1 || (!eps.is_empty() && !legacy_single) {
            let list: Vec<String> = eps
                .iter()
                .map(|e| {
                    let mut s = format!(
                        "{{ address = {:?}, port = {}",
                        e.address.as_host_str(),
                        e.port
                    );
                    // Endpoint ids are deterministic from address/port/weight/
                    // priority — emit only a custom id that the shorthand
                    // would otherwise drop.
                    if e.endpoint_id
                        != deterministic_endpoint_id(&e.address, e.port, e.weight, e.priority)
                    {
                        s.push_str(&format!(", id = {:?}", e.endpoint_id.as_str()));
                    }
                    if e.weight != Endpoint::DEFAULT_WEIGHT {
                        s.push_str(&format!(", weight = {}", e.weight));
                    }
                    if e.priority != Endpoint::DEFAULT_PRIORITY {
                        s.push_str(&format!(", priority = {}", e.priority));
                    }
                    if e.admin_state != AdminEndpointState::Enabled {
                        s.push_str(&format!(
                            ", admin_state = {:?}",
                            admin_state_toml(e.admin_state)
                        ));
                    }
                    // NG-08: passive-health params (non-default only).
                    if e.max_fails != Endpoint::DEFAULT_MAX_FAILS {
                        s.push_str(&format!(", max_fails = {}", e.max_fails));
                    }
                    if e.fail_timeout_ms != Endpoint::DEFAULT_FAIL_TIMEOUT_MS {
                        s.push_str(&format!(", fail_timeout_ms = {}", e.fail_timeout_ms));
                    }
                    if e.slow_start_ms != Endpoint::DEFAULT_SLOW_START_MS {
                        s.push_str(&format!(", slow_start_ms = {}", e.slow_start_ms));
                    }
                    s.push_str(" }");
                    s
                })
                .collect();
            out.push_str(&format!("endpoints = [{}]\n", list.join(", ")));
        } else {
            out.push_str(&format!("target = {:?}\n", upstream.target));
        }
        if upstream.selection_policy != EndpointSelectionPolicy::default() {
            let policy = match upstream.selection_policy {
                EndpointSelectionPolicy::WeightedRoundRobin => "weighted_round_robin",
                EndpointSelectionPolicy::LeastConn => "least_conn",
                EndpointSelectionPolicy::PowerOfTwo => "power_of_two",
                EndpointSelectionPolicy::ConsistentHash => "consistent_hash",
                EndpointSelectionPolicy::CookieAffinity => "cookie_affinity",
                EndpointSelectionPolicy::SourceAffinity => "source_affinity",
                EndpointSelectionPolicy::HeaderAffinity => "header_affinity",
            };
            out.push_str(&format!("selection_policy = {policy:?}\n"));
        }
        if upstream.failover_policy != EndpointFailoverPolicy::default() {
            let policy = match upstream.failover_policy {
                EndpointFailoverPolicy::PriorityBands => "priority_bands",
                EndpointFailoverPolicy::None => "none",
            };
            out.push_str(&format!("failover_policy = {policy:?}\n"));
        }
        if let Some(cookie) = &upstream.sticky_cookie {
            out.push_str(&format!("sticky_cookie = {cookie:?}\n"));
        }
        if let Some(header) = &upstream.sticky_header {
            out.push_str(&format!("sticky_header = {header:?}\n"));
        }
        if upstream.dns_refresh_ms != 0 {
            out.push_str(&format!("dns_refresh_ms = {}\n", upstream.dns_refresh_ms));
        }
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
        if pool.transport != "unix" {
            out.push_str(&format!("transport = {:?}\n", pool.transport));
        }
        out.push_str(&format!("max_concurrency = {}\n\n", pool.max_concurrency));
    }

    // NG-11: trusted-proxy client-IP resolution.
    let rip = &app.modules.real_ip;
    if rip.enabled || !rip.trusted.is_empty() {
        out.push_str("[modules.real_ip]\n");
        out.push_str(&format!("enabled = {}\n", rip.enabled));
        out.push_str(&format!("trusted = {:?}\n", rip.trusted));
        if rip.header != "x-forwarded-for" {
            out.push_str(&format!("header = {:?}\n", rip.header));
        }
        out.push('\n');
    }

    out
}

pub fn validate_mapped(mapped: &MappedConfig) -> Result<(), exyonq_config_ir::ConfigError> {
    exyonq_config_ir::AppConfig::parse_str(&mapped.toml)?;
    Ok(())
}
