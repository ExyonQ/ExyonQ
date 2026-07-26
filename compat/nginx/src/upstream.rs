//! NGINX upstream block resolution for Tier 2A offline import.

use crate::ast::ParsedUpstream;
use crate::limits::{MAX_GENERATED_ENDPOINTS, MAX_SERVERS_PER_UPSTREAM, MAX_UPSTREAM_BLOCKS};
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};
use exyonq_config_ir::{FcgiPoolConfig, UpstreamConfig, DEFAULT_FCGI_MAX_CONCURRENCY};
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
    } else if looks_like_host_port(endpoint) {
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
        let primary = block.servers.first().and_then(|s| http_target_url(s));
        let Some(target) = primary else {
            report.push(
                &block.loc,
                "upstream",
                CompatStatus::Error,
                format!("upstream `{name}` has no valid HTTP endpoint"),
                None,
            );
            continue;
        };
        let endpoint_count = block.servers.len();
        if endpoint_count > 1 {
            for server in block.servers.iter().skip(1) {
                report.push(
                    &block.loc,
                    "server",
                    CompatStatus::Partial,
                    format!(
                        "upstream `{name}` extra endpoint `{server}` not represented (IR single target; using first)"
                    ),
                    Some(format!("upstream.{name}.endpoints={endpoint_count}")),
                );
            }
            report.push(
                &block.loc,
                "upstream",
                CompatStatus::Partial,
                format!(
                    "upstream `{name}` has {endpoint_count} HTTP endpoints; ExyonQ IR uses first only (`{target}`)"
                ),
                Some(format!("upstream.{name} → target={target}")),
            );
        } else {
            report.push(
                &block.loc,
                "upstream",
                CompatStatus::Supported,
                format!("upstream `{name}` → ExyonQ upstream `{target}`"),
                Some(format!("upstream.{name}.endpoints=1")),
            );
        }
        out.insert(
            name.clone(),
            UpstreamConfig {
                name: name.clone(),
                target,
                timeout_ms: exyonq_config_ir::DEFAULT_UPSTREAM_TIMEOUT_MS,
            },
        );
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
