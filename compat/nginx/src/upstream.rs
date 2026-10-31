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
//! NGINX upstream block resolution for Tier 2A offline import.

use crate::ast::{NginxLbMethod, ParsedUpstream};
use crate::limits::{MAX_GENERATED_ENDPOINTS, MAX_SERVERS_PER_UPSTREAM, MAX_UPSTREAM_BLOCKS};
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};
use exyonq_config_ir::{
    AdminEndpointState, Endpoint, EndpointFailoverPolicy, EndpointSelectionPolicy, FcgiPoolConfig,
    RawEndpointConfig, UpstreamConfig, UpstreamHealthCheckConfig, DEFAULT_FCGI_MAX_CONCURRENCY,
    DEFAULT_UPSTREAM_TIMEOUT_MS,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    Http,
    UnixSocket,
    TcpFastcgi,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NginxUpstreamIndex {
    pub blocks: HashMap<String, ParsedUpstream>,
}

impl NginxUpstreamIndex {
    pub fn build(
        upstreams: &[ParsedUpstream],
        report: &mut CompatibilityReport,
    ) -> Result<Self, ()> {
        report.summary.nginx_upstream_blocks_parsed = upstreams.len();
        if upstreams.len() > MAX_UPSTREAM_BLOCKS {
            if let Some(first) = upstreams.first() {
                report.push(
                    &first.loc,
                    "upstream",
                    CompatStatus::Error,
                    format!("upstream block count exceeds limit ({MAX_UPSTREAM_BLOCKS})"),
                    None,
                );
            }
            return Err(());
        }

        let mut blocks = HashMap::new();
        let mut total_endpoints = 0usize;
        for up in upstreams {
            if up.servers.is_empty() {
                report.push(
                    &up.loc,
                    "upstream",
                    CompatStatus::Error,
                    format!("upstream `{}` has no server members", up.name),
                    None,
                );
                continue;
            }
            if up.servers.len() > MAX_SERVERS_PER_UPSTREAM {
                report.push(
                    &up.loc,
                    "upstream",
                    CompatStatus::Error,
                    format!(
                        "upstream `{}` exceeds max servers per block ({MAX_SERVERS_PER_UPSTREAM})",
                        up.name
                    ),
                    None,
                );
                continue;
            }
            total_endpoints = total_endpoints.saturating_add(up.servers.len());
            if total_endpoints > MAX_GENERATED_ENDPOINTS {
                report.push(
                    &up.loc,
                    "upstream",
                    CompatStatus::Error,
                    format!("total upstream endpoints exceed limit ({MAX_GENERATED_ENDPOINTS})"),
                    None,
                );
                return Err(());
            }
            if blocks.insert(up.name.clone(), up.clone()).is_some() {
                report.push(
                    &up.loc,
                    "upstream",
                    CompatStatus::Error,
                    format!("duplicate upstream block `{}`", up.name),
                    None,
                );
            }
        }
        Ok(Self { blocks })
    }
}

pub fn classify_endpoint(raw: &str) -> EndpointKind {
    let endpoint = raw.split_whitespace().next().unwrap_or(raw);
    if endpoint.starts_with("unix:") {
        EndpointKind::UnixSocket
    } else if endpoint.starts_with("http://")
        || endpoint.starts_with("https://")
        || looks_like_host_port(endpoint)
    {
        EndpointKind::Http
    } else if endpoint.contains(':') && !endpoint.starts_with('/') {
        EndpointKind::TcpFastcgi
    } else if !endpoint.is_empty() && !endpoint.starts_with('/') {
        // Bare hostname — nginx `server host;` defaults to port 80.
        EndpointKind::Http
    } else {
        EndpointKind::Invalid
    }
}

fn looks_like_host_port(endpoint: &str) -> bool {
    if endpoint.parse::<std::net::SocketAddr>().is_ok() {
        return true;
    }
    endpoint
        .rsplit_once(':')
        .is_some_and(|(host, port)| !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()))
}

pub fn http_target_url(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.split_whitespace().next().unwrap_or(endpoint);
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        Some(endpoint.to_string())
    } else if looks_like_host_port(endpoint) || !endpoint.contains(':') {
        // host:port → as-is; bare hostname → nginx default port 80 downstream.
        Some(format!("http://{endpoint}"))
    } else {
        None
    }
}

pub fn map_http_upstreams(
    index: &NginxUpstreamIndex,
    report: &mut CompatibilityReport,
) -> HashMap<String, UpstreamConfig> {
    let mut out = HashMap::new();
    let mut names: Vec<_> = index.blocks.keys().cloned().collect();
    names.sort();
    for name in names {
        let block = &index.blocks[&name];
        let kinds: Vec<EndpointKind> = block.servers.iter().map(|s| classify_endpoint(s)).collect();
        if kinds.contains(&EndpointKind::UnixSocket) {
            continue;
        }
        if kinds
            .iter()
            .any(|k| matches!(k, EndpointKind::TcpFastcgi | EndpointKind::Invalid))
        {
            report.push(
                &block.loc,
                "upstream",
                CompatStatus::Unsupported,
                format!("upstream `{name}` has non-HTTP endpoints; not mapped to [[upstream]]"),
                None,
            );
            continue;
        }
        // NG-07: every `server` member maps to an endpoint; `weight=`/`down`/
        // `backup` flags map to weight/admin_state/priority; the block LB
        // method maps to selection_policy.
        let mut raws: Vec<RawEndpointConfig> = Vec::with_capacity(block.servers.len());
        let mut ok = true;
        for server in &block.servers {
            let mut tokens = server.split_whitespace();
            let endpoint_token = tokens.next().unwrap_or(server.as_str());
            let Some(url) = http_target_url(server) else {
                continue;
            };
            if url.starts_with("https://") {
                report.push(
                    &block.loc,
                    "server",
                    CompatStatus::Unsupported,
                    format!(
                        "upstream `{name}` https endpoint `{endpoint_token}` not mapped (IR upstreams are HTTP)"
                    ),
                    None,
                );
                ok = false;
                break;
            }
            let host_port = url.trim_start_matches("http://");
            let (address, port) = match host_port.rsplit_once(':') {
                Some((h, p)) => (h, p.parse::<u16>().unwrap_or(80)),
                None => (host_port, 80),
            };
            let mut weight = Endpoint::DEFAULT_WEIGHT;
            let mut admin_state = AdminEndpointState::default();
            let mut priority = 0u32;
            // NG-08: nginx server-level passive health parameters.
            let mut max_fails = Endpoint::DEFAULT_MAX_FAILS;
            let mut fail_timeout_ms = Endpoint::DEFAULT_FAIL_TIMEOUT_MS;
            let mut slow_start_ms = Endpoint::DEFAULT_SLOW_START_MS;
            for flag in tokens {
                if let Some(w) = flag.strip_prefix("weight=") {
                    match w.parse::<u32>() {
                        Ok(v) => weight = v,
                        Err(_) => {
                            report.push(
                                &block.loc,
                                "server",
                                CompatStatus::Error,
                                format!(
                                    "upstream `{name}` endpoint `{endpoint_token}` has malformed weight `{w}`"
                                ),
                                None,
                            );
                            ok = false;
                        }
                    }
                // NG-08: `max_fails=` bare integer (fail-closed on junk).
                } else if let Some(v) = flag.strip_prefix("max_fails=") {
                    match v.parse::<u32>() {
                        Ok(n) => max_fails = n,
                        Err(_) => {
                            report.push(
                                &block.loc,
                                "server",
                                CompatStatus::Error,
                                format!(
                                    "upstream `{name}` endpoint `{endpoint_token}` has malformed max_fails `{v}`"
                                ),
                                None,
                            );
                            ok = false;
                        }
                    }
                // NG-08: `fail_timeout=`/`slow_start=` accept nginx time
                // syntax (bare seconds, or ms/s/m/h/d/w/M/y suffixes).
                } else if let Some(v) = flag.strip_prefix("fail_timeout=") {
                    match parse_nginx_time_ms(v) {
                        Some(ms) if ms > 0 => fail_timeout_ms = ms,
                        _ => {
                            report.push(
                                &block.loc,
                                "server",
                                CompatStatus::Error,
                                format!(
                                    "upstream `{name}` endpoint `{endpoint_token}` has invalid fail_timeout `{v}`"
                                ),
                                None,
                            );
                            ok = false;
                        }
                    }
                } else if let Some(v) = flag.strip_prefix("slow_start=") {
                    match parse_nginx_time_ms(v) {
                        Some(ms) => slow_start_ms = ms,
                        None => {
                            report.push(
                                &block.loc,
                                "server",
                                CompatStatus::Error,
                                format!(
                                    "upstream `{name}` endpoint `{endpoint_token}` has invalid slow_start `{v}`"
                                ),
                                None,
                            );
                            ok = false;
                        }
                    }
                // NG-07: `down` → disabled endpoint; `backup` → spare band.
                } else if flag == "down" {
                    admin_state = AdminEndpointState::Disabled;
                } else if flag == "backup" {
                    priority = 1;
                // NG-08: bare `max_fails`/`fail_timeout`/`slow_start` (no `=`)
                // — nginx rejects a missing parameter; fail closed.
                } else if matches!(flag, "max_fails" | "fail_timeout" | "slow_start") {
                    report.push(
                        &block.loc,
                        "server",
                        CompatStatus::Error,
                        format!(
                            "upstream `{name}` endpoint `{endpoint_token}` has bare `{flag}` (missing `=value`)"
                        ),
                        None,
                    );
                    ok = false;
                }
            }
            if !ok {
                break;
            }
            // nginx tolerates identical `server` lines — fold duplicates into
            // weight (same effective share without a duplicate endpoint id).
            // NG-08: differing passive-health params are NOT identical rows —
            // keep them separate instead of letting the first silently win.
            if let Some(existing) = raws.iter_mut().find(|e| {
                e.address == address
                    && e.port == port
                    && e.priority == priority
                    && e.admin_state == admin_state
                    && e.max_fails == max_fails
                    && e.fail_timeout_ms == fail_timeout_ms
                    && e.slow_start_ms == slow_start_ms
            }) {
                existing.weight = existing.weight.saturating_add(weight);
                continue;
            }
            raws.push(RawEndpointConfig {
                id: None,
                address: address.to_string(),
                port,
                weight,
                priority,
                admin_state,
                max_fails,
                fail_timeout_ms,
                slow_start_ms,
            });
        }
        if !ok || raws.is_empty() {
            if raws.is_empty() && ok {
                report.push(
                    &block.loc,
                    "upstream",
                    CompatStatus::Error,
                    format!("upstream `{name}` has no valid HTTP endpoint"),
                    None,
                );
            }
            continue;
        }
        let policy = match block.method {
            Some(NginxLbMethod::LeastConn) => EndpointSelectionPolicy::LeastConn,
            Some(NginxLbMethod::IpHash) => EndpointSelectionPolicy::SourceAffinity,
            Some(NginxLbMethod::ConsistentHash) => EndpointSelectionPolicy::ConsistentHash,
            Some(NginxLbMethod::StickyCookie) => EndpointSelectionPolicy::CookieAffinity,
            None => EndpointSelectionPolicy::default(),
        };
        match UpstreamConfig::try_from_parts(
            name.clone(),
            None,
            raws,
            policy,
            EndpointFailoverPolicy::default(),
            DEFAULT_UPSTREAM_TIMEOUT_MS,
            1,
            UpstreamHealthCheckConfig::default(),
            block.sticky_cookie.clone(),
            None,
            block.dns_refresh_ms.unwrap_or(0),
        ) {
            Ok(cfg) => {
                let n = cfg.endpoint_set.endpoints().len();
                report.push(
                    &block.loc,
                    "upstream",
                    CompatStatus::Supported,
                    format!("upstream `{name}` → ExyonQ upstream ({n} endpoint(s))"),
                    Some(format!("upstream.{name}.endpoints={n}")),
                );
                out.insert(name.clone(), cfg);
            }
            Err(e) => {
                report.push(
                    &block.loc,
                    "upstream",
                    CompatStatus::Error,
                    format!("upstream `{name}` rejected by IR: {e}"),
                    None,
                );
            }
        }
    }
    report.summary.http_upstreams_generated = out.len();
    out
}

pub fn map_fcgi_pools_from_upstreams(
    index: &NginxUpstreamIndex,
    report: &mut CompatibilityReport,
) -> HashMap<String, FcgiPoolConfig> {
    let mut out = HashMap::new();
    let mut names: Vec<_> = index.blocks.keys().cloned().collect();
    names.sort();
    for name in names {
        let block = &index.blocks[&name];
        let kinds: Vec<EndpointKind> = block.servers.iter().map(|s| classify_endpoint(s)).collect();
        let unix_count = kinds
            .iter()
            .filter(|k| **k == EndpointKind::UnixSocket)
            .count();
        let http_count = kinds
            .iter()
            .filter(|k| matches!(k, EndpointKind::Http))
            .count();
        let tcp_count = kinds
            .iter()
            .filter(|k| **k == EndpointKind::TcpFastcgi)
            .count();
        if unix_count == 0 {
            continue;
        }
        if http_count > 0 || tcp_count > 0 {
            report.push(
                &block.loc,
                "upstream",
                CompatStatus::Unsupported,
                format!("upstream `{name}` mixes Unix/HTTP/TCP endpoints; not mapped to fcgi_pool"),
                None,
            );
            continue;
        }
        if unix_count > 1 {
            report.push(
                &block.loc,
                "upstream",
                CompatStatus::Unsupported,
                format!(
                    "upstream `{name}` has {unix_count} Unix sockets; multi-address FastCGI not supported"
                ),
                Some(format!("upstream.{name}.endpoints={unix_count}")),
            );
            continue;
        }
        let socket = block
            .servers
            .first()
            .map(|s| s.split_whitespace().next().unwrap_or(s).to_string())
            .expect("unix upstream");
        report.push(
            &block.loc,
            "upstream",
            CompatStatus::Supported,
            format!("upstream `{name}` → fcgi_pool `{name}` (`{socket}`)"),
            Some(format!("fcgi_pool.{name}.address={socket}")),
        );
        out.insert(
            name.clone(),
            FcgiPoolConfig {
                name: name.clone(),
                address: socket,
                document_root: None,
                max_concurrency: DEFAULT_FCGI_MAX_CONCURRENCY,
                max_connections: None,
                transport: "unix".to_string(),
                idle_timeout_ms: 30_000,
                total_timeout_ms: 30_000,
                checkout_timeout_ms: 5_000,
            },
        );
        report.summary.fcgi_pools_from_upstreams += 1;
    }
    out
}

pub fn resolve_proxy_upstream_ref(
    name: &str,
    index: &NginxUpstreamIndex,
    http_upstreams: &HashMap<String, UpstreamConfig>,
    loc: &SourceLoc,
    report: &mut CompatibilityReport,
) -> Option<String> {
    let name = name.trim_end_matches('/');
    if !index.blocks.contains_key(name) {
        report.summary.unresolved_upstream_references += 1;
        report.push(
            loc,
            "proxy_pass",
            CompatStatus::Error,
            format!("named upstream `{name}` not defined"),
            None,
        );
        return None;
    }
    let block = &index.blocks[name];
    if block
        .servers
        .iter()
        .any(|s| classify_endpoint(s) == EndpointKind::UnixSocket)
    {
        report.push(
            loc,
            "proxy_pass",
            CompatStatus::Error,
            format!("upstream `{name}` is Unix/FastCGI; cannot use with proxy_pass"),
            None,
        );
        return None;
    }
    if !http_upstreams.contains_key(name) {
        report.push(
            loc,
            "proxy_pass",
            CompatStatus::Error,
            format!("upstream `{name}` could not be mapped to HTTP upstream"),
            None,
        );
        return None;
    }
    Some(name.to_string())
}

pub fn resolve_fastcgi_upstream_ref(
    name: &str,
    index: &NginxUpstreamIndex,
    pools_fcgi: &mut HashMap<String, FcgiPoolConfig>,
    document_root: Option<PathBuf>,
    loc: &SourceLoc,
    report: &mut CompatibilityReport,
) -> Option<String> {
    let name = name.trim();
    let Some(block) = index.blocks.get(name) else {
        report.summary.unresolved_upstream_references += 1;
        report.push(
            loc,
            "fastcgi_pass",
            CompatStatus::Error,
            format!("named upstream `{name}` not defined"),
            None,
        );
        return None;
    };
    let kinds: Vec<EndpointKind> = block.servers.iter().map(|s| classify_endpoint(s)).collect();
    if kinds.contains(&EndpointKind::Http) {
        report.push(
            loc,
            "fastcgi_pass",
            CompatStatus::Error,
            format!("upstream `{name}` is HTTP; cannot use with fastcgi_pass"),
            None,
        );
        return None;
    }
    if kinds.contains(&EndpointKind::TcpFastcgi) {
        report.push(
            loc,
            "fastcgi_pass",
            CompatStatus::Partial,
            format!("upstream `{name}` TCP endpoint blocked (TCP productivo not enabled)"),
            None,
        );
        return None;
    }
    let unix_count = kinds
        .iter()
        .filter(|k| **k == EndpointKind::UnixSocket)
        .count();
    if unix_count != 1 {
        report.push(
            loc,
            "fastcgi_pass",
            CompatStatus::Unsupported,
            format!("upstream `{name}` must have exactly one Unix socket for fastcgi_pass"),
            None,
        );
        return None;
    }
    if let Some(pool) = pools_fcgi.get_mut(name) {
        if pool.document_root.is_none() {
            pool.document_root = document_root.clone();
        }
    }
    if let Some(ref root) = document_root {
        report.push(
            loc,
            "fastcgi_pass",
            CompatStatus::Supported,
            format!(
                "fastcgi_pass `{name}` → fcgi_pool `{name}` (document_root `{}`)",
                root.display()
            ),
            Some(format!("fcgi_pool.{name}.document_root={}", root.display())),
        );
    } else {
        report.push(
            loc,
            "fastcgi_pass",
            CompatStatus::Partial,
            format!("fastcgi_pass `{name}` mapped; document_root missing"),
            Some(format!("fcgi_pool.{name}")),
        );
    }
    Some(name.to_string())
}

/// NG-08: nginx `time` syntax → milliseconds. Bare number = seconds
/// (nginx semantics); suffixes `ms s m h d w M y`. Returns `None` on
/// malformed input or overflow — callers fail closed.
pub(crate) fn parse_nginx_time_ms(s: &str) -> Option<u64> {
    const SECOND: u64 = 1_000;
    const MINUTE: u64 = 60 * SECOND;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const MONTH: u64 = 30 * DAY;
    const YEAR: u64 = 365 * DAY;
    let (digits, multiplier) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, SECOND)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, MINUTE)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, HOUR)
    } else if let Some(n) = s.strip_suffix('d') {
        (n, DAY)
    } else if let Some(n) = s.strip_suffix('w') {
        (n, WEEK)
    } else if let Some(n) = s.strip_suffix('M') {
        (n, MONTH)
    } else if let Some(n) = s.strip_suffix('y') {
        (n, YEAR)
    } else {
        (s, SECOND)
    };
    let value: u64 = digits.parse().ok()?;
    value.checked_mul(multiplier)
}
