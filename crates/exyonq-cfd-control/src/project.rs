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

//! Project control-plane routes into an immutable dataplane [`RouteTable`].
//!
//! Routing SSOT remains the product/control plane. This module only compiles
//! a Cap033-compatible projection for the separate-process dataplane.
//!
//! Text format for `EXYONQ_CFD_ROUTES` (optional override / bench):
//! ```text
//! # host|path|connect_addr|authority_host
//! host-a.example|/a|127.0.0.1:9001|127.0.0.1
//! *|/api|127.0.0.1:9002|127.0.0.1
//! |/fallback|127.0.0.1:9003|127.0.0.1
//!
//! # FastCGI pool definition:
//! # fcgi|pool_id|unix:/path|document_root|max_conn|idle_ms|connect_ms|read_ms|write_ms|total_ms
//! # max_conn must be 1 (ADR-042): serial execute_get per shard; not concurrent socket budget.
//! # fcgi|pool_id|tcp:127.0.0.1:9000|document_root|...
//! # ADR-043 optional routing policy (after pool definition):
//! # fcgi-dir-index|pool_id|index.php[|index.html…]
//! # fcgi-front-controller|pool_id|/index.php
//! # host|path|fcgi:pool_id
//! ```
//! Empty host field = hostless. Leading `*` = wildcard host.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use exyonq_cfd_gen::{
    validate_directory_index_candidate, validate_front_controller_target, BackendKind,
    CompiledFcgiPool, CompiledFrontControllerPolicy, CompiledRoute, CompiledStaticPolicy,
    CompiledUpstream, FcgiTransport, RouteTable, MAX_DIRECTORY_INDEX_CANDIDATES,
};
use thiserror::Error;

pub const ENV_ROUTES: &str = "EXYONQ_CFD_ROUTES";

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid routes line {line}: {message}")]
    InvalidLine { line: usize, message: String },
    #[error("empty route projection")]
    Empty,
}

/// Load route projection from `EXYONQ_CFD_ROUTES` file when set.
pub fn load_routes_from_env() -> Result<Option<RouteTable>, ProjectError> {
    let Ok(path) = std::env::var(ENV_ROUTES) else {
        return Ok(None);
    };
    if path.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(parse_routes_file(Path::new(&path))?))
}

/// Parse the Phase-2 / Phase-6B text projection format.
pub fn parse_routes_file(path: &Path) -> Result<RouteTable, ProjectError> {
    let raw = fs::read_to_string(path)?;
    parse_routes_text(&raw)
}

pub fn parse_routes_text(raw: &str) -> Result<RouteTable, ProjectError> {
    let mut upstreams: Vec<CompiledUpstream> = Vec::new();
    let mut fcgi_pools: Vec<CompiledFcgiPool> = Vec::new();
    let mut static_policies: Vec<CompiledStaticPolicy> = Vec::new();
    let mut routes: Vec<CompiledRoute> = Vec::new();
    let mut next_up = 1u32;
    let mut next_rt = 1u32;

    for (idx, line) in raw.lines().enumerate() {
        let line_no = idx + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            // Allow `# fcgi|...` and `# host|path|fcgi:id` as active lines when
            // the comment marker is only documentation prefix for the format —
            // but skip true comments. Active fcgi lines do NOT start with `#`.
            continue;
        }
        let parts: Vec<&str> = line.split('|').collect();
        if parts.first().copied() == Some("fcgi") {
            parse_fcgi_pool_line(line_no, &parts, &mut fcgi_pools)?;
            continue;
        }
        if parts.first().copied() == Some("fcgi-dir-index") {
            parse_fcgi_dir_index_line(line_no, &parts, &mut fcgi_pools)?;
            continue;
        }
        if parts.first().copied() == Some("fcgi-front-controller") {
            parse_fcgi_front_controller_line(line_no, &parts, &mut fcgi_pools)?;
            continue;
        }
        if parts.first().copied() == Some("static") {
            parse_static_policy_line(line_no, &parts, &mut static_policies)?;
            continue;
        }
        if parts.first().copied() == Some("static-index") {
            parse_static_index_line(line_no, &parts, &mut static_policies)?;
            continue;
        }
        if parts.len() == 3 && parts[2].trim().starts_with("fcgi:") {
            parse_fcgi_route_line(line_no, &parts, &mut routes, &mut next_rt, &fcgi_pools)?;
            continue;
        }
        if parts.len() == 3 && parts[2].trim().starts_with("static:") {
            parse_static_route_line(line_no, &parts, &mut routes, &mut next_rt, &static_policies)?;
            continue;
        }
        if parts.len() != 4 {
            return Err(ProjectError::InvalidLine {
                line: line_no,
                message: "expected host|path|connect_addr|authority_host or fcgi|... or static|... or host|path|fcgi:id/static:id"
                    .into(),
            });
        }
        let host_raw = parts[0].trim();
        let path = parts[1].trim();
        let connect: SocketAddr =
            parts[2]
                .trim()
                .parse()
                .map_err(|e| ProjectError::InvalidLine {
                    line: line_no,
                    message: format!("connect addr: {e}"),
                })?;
        let authority_host = parts[3].trim().to_string();
        if path.is_empty() {
            return Err(ProjectError::InvalidLine {
                line: line_no,
                message: "empty path".into(),
            });
        }
        if authority_host.is_empty() {
            return Err(ProjectError::InvalidLine {
                line: line_no,
                message: "empty authority_host".into(),
            });
        }
        let upstream_id = next_up;
        next_up = next_up.saturating_add(1);
        upstreams.push(CompiledUpstream {
            id: upstream_id,
            connect,
            authority_host,
        });
        let host = if host_raw.is_empty() {
            None
        } else {
            Some(host_raw.to_string())
        };
        let route_id = next_rt;
        next_rt = next_rt.saturating_add(1);
        routes.push(CompiledRoute {
            route_id,
            host,
            path: path.to_string(),
            backend_kind: BackendKind::Proxy,
            backend_id: upstream_id,
        });
    }

    if routes.is_empty() {
        return Err(ProjectError::Empty);
    }
    Ok(RouteTable {
        upstreams,
        fcgi_pools,
        static_policies,
        routes,
    })
}

fn parse_fcgi_pool_line(
    line_no: usize,
    parts: &[&str],
    pools: &mut Vec<CompiledFcgiPool>,
) -> Result<(), ProjectError> {
    // fcgi|pool_id|unix:/path|document_root|max_conn|idle_ms|connect_ms|read_ms|write_ms|total_ms
    // optional 11th field script_suffix: must be empty (ADR-041). Non-empty = fail-closed.
    // max_conn must be 1 (ADR-042): CFD FastCGI is serial per shard; values other than 1
    // do not represent concurrent backend capacity and are rejected fail-closed.
    if parts.len() < 10 {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "fcgi line needs pool_id|transport|document_root|max_conn|idle|connect|read|write|total"
                .into(),
        });
    }
    let id: u32 = parts[1]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid pool_id".into(),
        })?;
    let transport = parse_transport(line_no, parts[2].trim())?;
    let document_root = parts[3].trim().to_string();
    if document_root.is_empty() {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "empty document_root".into(),
        });
    }
    let max_connections: u32 = parts[4]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid max_conn".into(),
        })?;
    if max_connections != 1 {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: format!(
                "max_conn={max_connections} unsupported (ADR-042): CFD FastCGI execute_get is serial per shard; \
                 max_conn does not control concurrent backend sockets; supported value is max_conn=1"
            ),
        });
    }
    let idle_timeout_ms: u32 = parts[5]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid idle_ms".into(),
        })?;
    let connect_timeout_ms: u32 =
        parts[6]
            .trim()
            .parse()
            .map_err(|_| ProjectError::InvalidLine {
                line: line_no,
                message: "invalid connect_ms".into(),
            })?;
    let read_timeout_ms: u32 = parts[7]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid read_ms".into(),
        })?;
    let write_timeout_ms: u32 = parts[8]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid write_ms".into(),
        })?;
    let total_timeout_ms: u32 = parts[9]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid total_ms".into(),
        })?;
    let script_suffix = if parts.len() > 10 {
        parts[10].trim().to_string()
    } else {
        String::new()
    };
    if !script_suffix.is_empty() {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "script_suffix is unsupported; leave empty (ADR-041 fail-closed)".into(),
        });
    }
    pools.push(CompiledFcgiPool {
        id,
        transport,
        document_root,
        script_suffix,
        max_connections,
        idle_timeout_ms,
        connect_timeout_ms,
        read_timeout_ms,
        write_timeout_ms,
        total_timeout_ms,
        directory_index: Vec::new(),
        front_controller: None,
    });
    Ok(())
}

fn parse_fcgi_dir_index_line(
    line_no: usize,
    parts: &[&str],
    pools: &mut [CompiledFcgiPool],
) -> Result<(), ProjectError> {
    // fcgi-dir-index|pool_id|index.php[|index.html…]
    if parts.len() < 3 {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "fcgi-dir-index needs pool_id|candidate…".into(),
        });
    }
    let id: u32 = parts[1]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid pool_id".into(),
        })?;
    let candidates: Vec<String> = parts[2..]
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if candidates.is_empty() {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "fcgi-dir-index requires at least one candidate".into(),
        });
    }
    if candidates.len() > MAX_DIRECTORY_INDEX_CANDIDATES {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: format!(
                "too many directory_index candidates (max {MAX_DIRECTORY_INDEX_CANDIDATES})"
            ),
        });
    }
    for name in &candidates {
        if !validate_directory_index_candidate(name) {
            return Err(ProjectError::InvalidLine {
                line: line_no,
                message: format!("invalid directory_index candidate '{name}'"),
            });
        }
    }
    let pool = pools
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| ProjectError::InvalidLine {
            line: line_no,
            message: format!("unknown fcgi pool id {id} for fcgi-dir-index"),
        })?;
    pool.directory_index = candidates;
    Ok(())
}

fn parse_fcgi_front_controller_line(
    line_no: usize,
    parts: &[&str],
    pools: &mut [CompiledFcgiPool],
) -> Result<(), ProjectError> {
    // fcgi-front-controller|pool_id|/index.php
    if parts.len() != 3 {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "fcgi-front-controller needs pool_id|/target.php".into(),
        });
    }
    let id: u32 = parts[1]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid pool_id".into(),
        })?;
    let target = parts[2].trim().to_string();
    if !validate_front_controller_target(&target) {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: format!(
                "invalid front_controller target '{target}' (literal local .php URI required)"
            ),
        });
    }
    let pool = pools
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| ProjectError::InvalidLine {
            line: line_no,
            message: format!("unknown fcgi pool id {id} for fcgi-front-controller"),
        })?;
    pool.front_controller = Some(CompiledFrontControllerPolicy {
        target_uri: target,
        require_not_file: true,
        require_not_directory: true,
        preserve_query: true,
    });
    Ok(())
}

fn parse_transport(line_no: usize, s: &str) -> Result<FcgiTransport, ProjectError> {
    if let Some(path) = s.strip_prefix("unix:") {
        if !path.starts_with('/') {
            return Err(ProjectError::InvalidLine {
                line: line_no,
                message: "unix socket path must be absolute (SINGLE_TENANT_ADMIN_FS)".into(),
            });
        }
        return Ok(FcgiTransport::Unix(PathBuf::from(path)));
    }
    if let Some(addr) = s.strip_prefix("tcp:") {
        let a: SocketAddr = addr.parse().map_err(|e| ProjectError::InvalidLine {
            line: line_no,
            message: format!("tcp addr: {e}"),
        })?;
        return Ok(FcgiTransport::Tcp(a));
    }
    // bare addr = tcp
    let a: SocketAddr = s.parse().map_err(|e| ProjectError::InvalidLine {
        line: line_no,
        message: format!("transport: {e}"),
    })?;
    Ok(FcgiTransport::Tcp(a))
}

fn parse_static_policy_line(
    line_no: usize,
    parts: &[&str],
    policies: &mut Vec<CompiledStaticPolicy>,
) -> Result<(), ProjectError> {
    // static|policy_id|/absolute/docroot[|index.html|index.htm]
    if parts.len() < 3 {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "static line needs policy_id|absolute_docroot[|index...]".into(),
        });
    }
    let id: u32 = parts[1]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid static policy_id".into(),
        })?;
    let document_root = parts[2].trim().to_string();
    validate_static_document_root(line_no, &document_root)?;
    let index = parse_static_index_candidates(line_no, &parts[3..])?;
    policies.push(CompiledStaticPolicy {
        id,
        document_root,
        flags: 0,
        index,
    });
    Ok(())
}

fn parse_static_index_line(
    line_no: usize,
    parts: &[&str],
    policies: &mut [CompiledStaticPolicy],
) -> Result<(), ProjectError> {
    // static-index|policy_id|index.html[|index.htm...]
    if parts.len() < 3 {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "static-index needs policy_id|candidate...".into(),
        });
    }
    let id: u32 = parts[1]
        .trim()
        .parse()
        .map_err(|_| ProjectError::InvalidLine {
            line: line_no,
            message: "invalid static policy_id".into(),
        })?;
    let candidates = parse_static_index_candidates(line_no, &parts[2..])?;
    if candidates.is_empty() {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "static-index requires at least one candidate".into(),
        });
    }
    let policy =
        policies
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| ProjectError::InvalidLine {
                line: line_no,
                message: format!("unknown static policy id {id} for static-index"),
            })?;
    policy.index = candidates;
    Ok(())
}

fn parse_static_index_candidates(
    line_no: usize,
    parts: &[&str],
) -> Result<Vec<String>, ProjectError> {
    let candidates: Vec<String> = parts
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if candidates.len() > MAX_DIRECTORY_INDEX_CANDIDATES {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: format!(
                "too many static index candidates (max {MAX_DIRECTORY_INDEX_CANDIDATES})"
            ),
        });
    }
    for name in &candidates {
        if !validate_static_index_candidate(name) {
            return Err(ProjectError::InvalidLine {
                line: line_no,
                message: format!("invalid static index candidate '{name}'"),
            });
        }
    }
    Ok(candidates)
}

fn validate_static_document_root(line_no: usize, root: &str) -> Result<(), ProjectError> {
    if root.is_empty() || !Path::new(root).is_absolute() || root.contains('\0') {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "static document_root must be absolute".into(),
        });
    }
    Ok(())
}

fn validate_static_index_candidate(name: &str) -> bool {
    validate_directory_index_candidate(name) && !name.to_ascii_lowercase().ends_with(".php")
}

fn parse_fcgi_route_line(
    line_no: usize,
    parts: &[&str],
    routes: &mut Vec<CompiledRoute>,
    next_rt: &mut u32,
    pools: &[CompiledFcgiPool],
) -> Result<(), ProjectError> {
    let host_raw = parts[0].trim();
    let path = parts[1].trim();
    let pool_s =
        parts[2]
            .trim()
            .strip_prefix("fcgi:")
            .ok_or_else(|| ProjectError::InvalidLine {
                line: line_no,
                message: "expected fcgi:pool_id".into(),
            })?;
    let pool_id: u32 = pool_s.parse().map_err(|_| ProjectError::InvalidLine {
        line: line_no,
        message: "invalid fcgi pool_id".into(),
    })?;
    if !pools.iter().any(|p| p.id == pool_id) {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: format!("unknown fcgi pool {pool_id}"),
        });
    }
    if path.is_empty() {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "empty path".into(),
        });
    }
    let host = if host_raw.is_empty() {
        None
    } else {
        Some(host_raw.to_string())
    };
    let route_id = *next_rt;
    *next_rt = next_rt.saturating_add(1);
    routes.push(CompiledRoute {
        route_id,
        host,
        path: path.to_string(),
        backend_kind: BackendKind::Fastcgi,
        backend_id: pool_id,
    });
    Ok(())
}

fn parse_static_route_line(
    line_no: usize,
    parts: &[&str],
    routes: &mut Vec<CompiledRoute>,
    next_rt: &mut u32,
    policies: &[CompiledStaticPolicy],
) -> Result<(), ProjectError> {
    let host_raw = parts[0].trim();
    let path = parts[1].trim();
    let policy_s =
        parts[2]
            .trim()
            .strip_prefix("static:")
            .ok_or_else(|| ProjectError::InvalidLine {
                line: line_no,
                message: "expected static:policy_id".into(),
            })?;
    let policy_id: u32 = policy_s.parse().map_err(|_| ProjectError::InvalidLine {
        line: line_no,
        message: "invalid static policy_id".into(),
    })?;
    if !policies.iter().any(|p| p.id == policy_id) {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: format!("unknown static policy {policy_id}"),
        });
    }
    if path.is_empty() {
        return Err(ProjectError::InvalidLine {
            line: line_no,
            message: "empty path".into(),
        });
    }
    let host = if host_raw.is_empty() {
        None
    } else {
        Some(host_raw.to_string())
    };
    let route_id = *next_rt;
    *next_rt = next_rt.saturating_add(1);
    routes.push(CompiledRoute {
        route_id,
        host,
        path: path.to_string(),
        backend_kind: BackendKind::Static,
        backend_id: policy_id,
    });
    Ok(())
}

/// Build a table from explicit tuples (tests / programmatic projection).
pub fn table_from_entries(entries: &[(Option<&str>, &str, SocketAddr, &str)]) -> RouteTable {
    let mut upstreams = Vec::new();
    let mut routes = Vec::new();
    for (i, (host, path, connect, authority)) in entries.iter().enumerate() {
        let id = (i as u32).saturating_add(1);
        upstreams.push(CompiledUpstream {
            id,
            connect: *connect,
            authority_host: (*authority).to_string(),
        });
        routes.push(CompiledRoute {
            route_id: id,
            host: host.map(|h| h.to_string()),
            path: (*path).to_string(),
            backend_kind: BackendKind::Proxy,
            backend_id: id,
        });
    }
    RouteTable {
        upstreams,
        fcgi_pools: Vec::new(),
        static_policies: Vec::new(),
        routes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_cfd_gen::BackendTarget;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn parse_matrix() {
        let t = parse_routes_text(
            "host-a.example|/a|127.0.0.1:9001|127.0.0.1\n\
             host-a.example|/b|127.0.0.1:9002|127.0.0.1\n\
             host-b.example|/a|127.0.0.1:9003|127.0.0.1\n",
        )
        .unwrap();
        assert_eq!(t.routes.len(), 3);
        match t.lookup("/a", Some("host-a.example")).unwrap().1 {
            BackendTarget::Proxy(u) => assert_eq!(u.connect.port(), 9001),
            _ => panic!("proxy"),
        }
    }

    #[test]
    fn parse_fcgi_lines() {
        let t = parse_routes_text(
            "fcgi|1|tcp:127.0.0.1:9000|/var/www|1|60000|2000|120000|60000|180000\n\
             |/php|fcgi:1\n",
        )
        .unwrap();
        assert_eq!(t.fcgi_pools.len(), 1);
        match t.lookup("/php/index.php", None).unwrap().1 {
            BackendTarget::Fastcgi(p) => {
                assert_eq!(p.document_root, "/var/www");
                assert_eq!(p.max_connections, 1);
            }
            _ => panic!("fcgi"),
        }
    }

    #[test]
    fn parse_static_lines() {
        let t = parse_routes_text(
            "static|7|/srv/static|index.html|index.htm\n\
             host.example|/assets|static:7\n",
        )
        .unwrap();
        assert_eq!(t.static_policies.len(), 1);
        let p = &t.static_policies[0];
        assert_eq!(p.id, 7);
        assert_eq!(p.document_root, "/srv/static");
        assert_eq!(p.index, vec!["index.html", "index.htm"]);
        match t.lookup("/assets/app.css", Some("host.example")).unwrap().1 {
            BackendTarget::Static(policy) => assert_eq!(policy.id, 7),
            _ => panic!("static"),
        }
    }

    #[test]
    fn parse_static_index_extension_line() {
        let t = parse_routes_text(
            "static|7|/srv/static\n\
             static-index|7|home.html|index.html\n\
             |/assets|static:7\n",
        )
        .unwrap();
        assert_eq!(t.static_policies[0].index, vec!["home.html", "index.html"]);
    }

    #[test]
    fn parse_static_rejects_relative_root() {
        let err = parse_routes_text(
            "static|7|relative/root|index.html\n\
             |/assets|static:7\n",
        )
        .unwrap_err();
        match err {
            ProjectError::InvalidLine { message, .. } => {
                assert!(message.contains("absolute"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_static_rejects_php_index() {
        let err = parse_routes_text(
            "static|7|/srv/static|index.php\n\
             |/assets|static:7\n",
        )
        .unwrap_err();
        match err {
            ProjectError::InvalidLine { message, .. } => {
                assert!(message.contains("static index"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_fcgi_dir_index_and_front_controller() {
        let t = parse_routes_text(
            "fcgi|1|tcp:127.0.0.1:9000|/var/www|1|60000|2000|120000|60000|180000\n\
             fcgi-dir-index|1|index.php|index.html\n\
             fcgi-front-controller|1|/index.php\n\
             |/|fcgi:1\n",
        )
        .unwrap();
        let p = &t.fcgi_pools[0];
        assert_eq!(p.directory_index, vec!["index.php", "index.html"]);
        let fc = p.front_controller.as_ref().unwrap();
        assert_eq!(fc.target_uri, "/index.php");
        assert!(fc.require_not_file && fc.require_not_directory && fc.preserve_query);
    }

    #[test]
    fn parse_fcgi_rejects_bad_front_controller() {
        let err = parse_routes_text(
            "fcgi|1|tcp:127.0.0.1:9000|/var/www|1|60000|2000|120000|60000|180000\n\
             fcgi-front-controller|1|/../index.php\n\
             |/|fcgi:1\n",
        )
        .unwrap_err();
        match err {
            ProjectError::InvalidLine { message, .. } => {
                assert!(message.contains("front_controller"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_fcgi_rejects_max_conn_gt1() {
        let err = parse_routes_text(
            "fcgi|1|tcp:127.0.0.1:9000|/var/www|6|60000|2000|120000|60000|180000\n\
             |/php|fcgi:1\n",
        )
        .unwrap_err();
        match err {
            ProjectError::InvalidLine { message, .. } => {
                assert!(message.contains("max_conn=6"), "{message}");
                assert!(message.contains("ADR-042"), "{message}");
                assert!(message.contains("serial"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_fcgi_rejects_max_conn_zero() {
        let err = parse_routes_text(
            "fcgi|1|tcp:127.0.0.1:9000|/var/www|0|60000|2000|120000|60000|180000\n\
             |/php|fcgi:1\n",
        )
        .unwrap_err();
        match err {
            ProjectError::InvalidLine { message, .. } => {
                assert!(message.contains("max_conn=0"), "{message}");
                assert!(message.contains("ADR-042"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_fcgi_rejects_nonempty_script_suffix() {
        let err = parse_routes_text(
            "fcgi|1|tcp:127.0.0.1:9000|/var/www|1|60000|2000|120000|60000|180000|.php\n\
             |/php|fcgi:1\n",
        )
        .unwrap_err();
        match err {
            ProjectError::InvalidLine { message, .. } => {
                assert!(message.contains("script_suffix"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn table_from_entries_ok() {
        let a = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1);
        let t = table_from_entries(&[(Some("h"), "/p", a, "h")]);
        assert_eq!(t.routes.len(), 1);
    }
}
