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
//! PR5-B1 adapter — maps module transport/client errors to [`FcgiDispatchOutcome`].

use crate::caps::{MAX_CGI_HEADER_BYTES, MAX_CGI_HEADER_COUNT, MAX_FCGI_RESPONSE_BYTES};
use crate::client::{ClientError, PhpFpmClient};
use crate::metrics;
use crate::mock::{MockFpmConfig, MockFpmTransport};
use crate::params::MinForwardRequest;
use crate::transport::TransportError;
use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchRequest, FcgiSuccessResponse,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Map module transport errors to the closed core-visible outcome enum.
pub fn map_transport_error(err: TransportError) -> FcgiDispatchOutcome {
    match err {
        TransportError::Timeout => FcgiDispatchOutcome::GatewayTimeout,
        TransportError::ConnectionFailed => FcgiDispatchOutcome::BadGateway,
        TransportError::ConnectionClosed => FcgiDispatchOutcome::BadGateway,
        TransportError::InvalidFrame(_) => FcgiDispatchOutcome::BadGateway,
        TransportError::UnexpectedRecordType { .. } => FcgiDispatchOutcome::BadGateway,
        TransportError::WrongRequestId { .. } => FcgiDispatchOutcome::BadGateway,
        TransportError::RequestIncomplete => FcgiDispatchOutcome::BadGateway,
        TransportError::ResponseAlreadyTaken => FcgiDispatchOutcome::BadGateway,
        TransportError::RequestAlreadyComplete => FcgiDispatchOutcome::BadGateway,
        TransportError::EncodeFailed => FcgiDispatchOutcome::BadGateway,
        TransportError::ResponseCapExceeded => FcgiDispatchOutcome::BadGateway,
        TransportError::IoFailed => FcgiDispatchOutcome::BadGateway,
        TransportError::NotImplemented => FcgiDispatchOutcome::NotRegistered,
    }
}

/// Map client-level errors (encode/decode/params/wire) to dispatch outcomes.
pub fn map_client_error(err: ClientError) -> FcgiDispatchOutcome {
    match err {
        ClientError::NotImplemented => FcgiDispatchOutcome::NotRegistered,
        ClientError::Transport(e) => map_transport_error(e),
        ClientError::Encode(_) | ClientError::Decode(_) | ClientError::Params(_) => {
            FcgiDispatchOutcome::BadGateway
        }
        ClientError::Wire(e) => map_wire_error(e),
    }
}

fn map_wire_error(err: crate::wire::WireError) -> FcgiDispatchOutcome {
    match err {
        crate::wire::WireError::PoolBusy => FcgiDispatchOutcome::ServiceUnavailable,
        other => map_transport_error(other.into()),
    }
}

/// CGI stdout parse failure (PR5-B1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgiParseError {
    Invalid,
}

/// Parse CGI/1.1 stdout into an authorized success response.
pub fn parse_cgi_stdout(stdout: &[u8]) -> Result<FcgiSuccessResponse, CgiParseError> {
    if stdout.len() > MAX_FCGI_RESPONSE_BYTES {
        return Err(CgiParseError::Invalid);
    }
    let header_end = stdout
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
        .or_else(|| {
            stdout
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|pos| pos + 2)
        })
        .ok_or(CgiParseError::Invalid)?;
    if header_end > MAX_CGI_HEADER_BYTES {
        return Err(CgiParseError::Invalid);
    }
    let header_block = trim_trailing_crlf(&stdout[..header_end]);
    let body = stdout[header_end..].to_vec();
    let header_text = std::str::from_utf8(header_block).map_err(|_| CgiParseError::Invalid)?;
    let mut status = 200u16;
    let mut headers = Vec::new();
    let mut saw_content_length = false;
    for line in header_text.lines() {
        if line.is_empty() {
            continue;
        }
        if headers.len() >= MAX_CGI_HEADER_COUNT {
            return Err(CgiParseError::Invalid);
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(CgiParseError::Invalid);
        };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.contains('\r') || value.contains('\n') {
            return Err(CgiParseError::Invalid);
        }
        if name.eq_ignore_ascii_case("status") {
            status = parse_status_value(value).ok_or(CgiParseError::Invalid)?;
            continue;
        }
        if is_hop_by_hop_header(name) {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if lower == "content-length" {
            if saw_content_length {
                return Err(CgiParseError::Invalid);
            }
            saw_content_length = true;
        }
        headers.push((lower, value.to_string()));
    }
    Ok(FcgiSuccessResponse {
        status,
        headers,
        body,
    })
}

fn trim_trailing_crlf(mut bytes: &[u8]) -> &[u8] {
    while bytes.ends_with(b"\r") || bytes.ends_with(b"\n") {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn parse_status_value(value: &str) -> Option<u16> {
    let code = value.split_whitespace().next()?.parse().ok()?;
    (100..=599).contains(&code).then_some(code)
}

fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// Map successful upstream roundtrip stdout to authorized success outcome.
pub fn map_forward_success(stdout: &[u8]) -> FcgiDispatchOutcome {
    match parse_cgi_stdout(stdout) {
        Ok(response) => FcgiDispatchOutcome::Success(response),
        Err(CgiParseError::Invalid) => FcgiDispatchOutcome::BadGateway,
    }
}

/// Mock-backed executor for tests and composition registration.
#[derive(Debug, Clone)]
pub struct MockFcgiExecutor {
    config: MockFpmConfig,
}

impl MockFcgiExecutor {
    pub fn success_default() -> Self {
        Self::from_config(MockFpmConfig::default())
    }

    pub fn pr5b1_default() -> Self {
        Self::from_config(MockFpmConfig::pr5b1_default())
    }

    pub fn from_config(config: MockFpmConfig) -> Self {
        Self { config }
    }
}

impl FcgiBackendExecutor for MockFcgiExecutor {
    fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        let client =
            PhpFpmClient::with_transport("php", MockFpmTransport::new(self.config.clone()));
        match client.forward_once(
            &[("REQUEST_METHOD", "GET"), ("SCRIPT_NAME", "/index.php")],
            b"",
        ) {
            Ok(response) => map_forward_success(&response.stdout),
            Err(err) => map_client_error(err),
        }
    }
}

/// Production executor — pooled FastCGI (UDS or TCP) with generation-scoped ConnPool.
pub struct FcgiModuleExecutor {
    pools: HashMap<u32, std::sync::Arc<crate::conn_pool::ConnPool>>,
}

impl FcgiModuleExecutor {
    /// Single-pool Unix production constructor.
    pub fn production_unix(socket_path: PathBuf, connect_timeout: Duration) -> Self {
        Self::production_pools(
            vec![(0, crate::fcgi_stream::PoolEndpoint::Unix(socket_path))],
            move |_| crate::conn_pool::ConnPoolConfig {
                connect_timeout,
                ..crate::conn_pool::ConnPoolConfig::default()
            },
        )
    }

    /// Multi-pool Unix constructor (`pool_id` → unix socket path).
    pub fn production_unix_pools(pools: Vec<(u32, PathBuf)>, connect_timeout: Duration) -> Self {
        let endpoints = pools
            .into_iter()
            .map(|(id, path)| (id, crate::fcgi_stream::PoolEndpoint::Unix(path)))
            .collect();
        Self::production_pools(endpoints, move |_| crate::conn_pool::ConnPoolConfig {
            connect_timeout,
            ..crate::conn_pool::ConnPoolConfig::default()
        })
    }

    /// Multi-pool with per-pool config (generation-scoped); UDS or TCP endpoints.
    pub fn production_pools(
        pools: Vec<(u32, crate::fcgi_stream::PoolEndpoint)>,
        config_for: impl Fn(u32) -> crate::conn_pool::ConnPoolConfig,
    ) -> Self {
        let generation = crate::conn_pool::next_pool_generation();
        metrics::note_fcgi_pool_generation_created();
        metrics::note_fcgi_pool_generation();
        let mut map = HashMap::new();
        for (pool_id, endpoint) in pools {
            let cfg = config_for(pool_id);
            map.insert(
                pool_id,
                std::sync::Arc::new(crate::conn_pool::ConnPool::new(generation, endpoint, cfg)),
            );
        }
        Self { pools: map }
    }

    /// Build executor from compiled plan slots (one ConnPool generation; no cross-gen reuse).
    pub fn from_compiled_slots(
        generation: u64,
        slots: &[exyonq_module_api::fcgi_dispatch::FcgiCompiledSlot],
    ) -> Result<Self, exyonq_module_api::fcgi_dispatch::FcgiBindError> {
        use exyonq_module_api::fcgi_dispatch::FcgiBindError;
        let mut endpoints = Vec::with_capacity(slots.len());
        let mut configs = HashMap::new();
        for slot in slots {
            let transport_by_name = {
                let mut m = HashMap::new();
                m.insert(slot.name.clone(), slot.transport.clone());
                m
            };
            let address_by_name = {
                let mut m = HashMap::new();
                m.insert(slot.name.clone(), slot.address.clone());
                m
            };
            let resolved = resolve_pool_endpoints(
                &[slot.name.clone()],
                &address_by_name,
                &transport_by_name,
                None,
            );
            let Some((_, endpoint)) = resolved.into_iter().next() else {
                return Err(FcgiBindError::InvalidEndpoint {
                    pool: slot.name.clone(),
                    detail: format!(
                        "cannot resolve endpoint address={} transport={}",
                        slot.address, slot.transport
                    ),
                });
            };
            if slot.max_concurrency == 0 || slot.max_connections == 0 {
                return Err(FcgiBindError::InvalidCapacity {
                    pool: slot.name.clone(),
                });
            }
            endpoints.push((slot.pool_id, endpoint));
            configs.insert(
                slot.pool_id,
                crate::conn_pool::ConnPoolConfig {
                    max_connections: slot.max_connections as usize,
                    idle_timeout: Duration::from_millis(slot.idle_timeout_ms),
                    total_timeout: Duration::from_millis(slot.total_timeout_ms),
                    checkout_timeout: Duration::from_millis(slot.checkout_timeout_ms),
                    connect_timeout: crate::caps::FCGI_CONNECT_TIMEOUT,
                    read_timeout: crate::caps::FCGI_READ_TIMEOUT,
                    write_timeout: crate::caps::FCGI_WRITE_TIMEOUT,
                },
            );
        }
        metrics::note_fcgi_pool_generation_created();
        metrics::note_fcgi_pool_generation();
        let mut map = HashMap::new();
        for (pool_id, endpoint) in endpoints {
            let cfg = configs
                .remove(&pool_id)
                .unwrap_or_else(crate::conn_pool::ConnPoolConfig::default);
            map.insert(
                pool_id,
                std::sync::Arc::new(crate::conn_pool::ConnPool::new(generation, endpoint, cfg)),
            );
        }
        Ok(Self { pools: map })
    }

    /// True when every pool is draining and has no open sockets / waiters.
    pub fn is_quiescent(&self) -> bool {
        if self.pools.is_empty() {
            return true;
        }
        self.pools.values().all(|p| {
            let s = p.stats();
            s.draining && s.open == 0 && s.waiters == 0
        })
    }

    pub fn pool_map_len(&self) -> usize {
        self.pools.len()
    }

    pub fn any_generation(&self) -> Option<u64> {
        self.pools.values().next().map(|p| p.generation())
    }

    /// Legacy alias — Unix paths only.
    pub fn production_unix_pools_with_config(
        pools: Vec<(u32, PathBuf)>,
        config_for: impl Fn(u32) -> crate::conn_pool::ConnPoolConfig,
    ) -> Self {
        let endpoints = pools
            .into_iter()
            .map(|(id, path)| (id, crate::fcgi_stream::PoolEndpoint::Unix(path)))
            .collect();
        Self::production_pools(endpoints, config_for)
    }

    /// Begin drain on all pools (reload / shutdown).
    pub fn begin_drain_all(&self) {
        for pool in self.pools.values() {
            pool.begin_drain();
        }
    }

    /// Test/ops: pool stats for `pool_id`.
    pub fn pool_stats(&self, pool_id: u32) -> Option<crate::conn_pool::ConnPoolStats> {
        self.pools.get(&pool_id).map(|p| p.stats())
    }
}

impl FcgiBackendExecutor for FcgiModuleExecutor {
    fn dispatch(&self, request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        let Some(pool) = self.pools.get(&request.pool_id) else {
            return FcgiDispatchOutcome::BadGateway;
        };

        let forward = dispatch_request_to_min(request);
        let params = match forward.to_fcgi_params() {
            Ok(params) => params,
            Err(_) => return FcgiDispatchOutcome::BadGateway,
        };

        match pool.forward_once(&params, &forward.stdin) {
            Ok(response) => map_forward_success(&response.stdout),
            Err(err) => map_wire_error(err),
        }
    }

    fn begin_drain(&self) {
        self.begin_drain_all();
    }
}

fn dispatch_request_to_min(request: &FcgiDispatchRequest) -> MinForwardRequest {
    let query_string = if request.query_string.is_empty() {
        None
    } else {
        Some(request.query_string.clone())
    };
    MinForwardRequest {
        request_method: request.method.clone(),
        request_uri: request.request_uri.clone(),
        script_name: request.script_name.clone(),
        script_filename: request.script_filename.clone(),
        document_root: request.document_root.clone(),
        server_protocol: request.server_protocol.clone(),
        server_name: request.server_name.clone(),
        server_port: request.server_port,
        query_string,
        content_type: request.content_type.clone(),
        path_info: request.path_info.clone(),
        remote_addr: request.remote_addr.clone(),
        http_headers: request.headers.clone(),
        stdin: request.body.clone(),
    }
}

/// Normalize `unix:/path` (nginx-style) to a filesystem path, or return bare `/path`.
pub fn normalize_unix_socket_path(address: &str) -> Option<&str> {
    if let Some(rest) = address.strip_prefix("unix:") {
        return rest.starts_with('/').then_some(rest);
    }
    address.starts_with('/').then_some(address)
}

/// Returns true when `address` names a unix domain socket path.
pub fn is_unix_socket_path(address: &str) -> bool {
    normalize_unix_socket_path(address).is_some()
}

/// Returns true when `address` looks like `host:port` for TCP FastCGI.
pub fn is_tcp_pool_address(address: &str) -> bool {
    !is_unix_socket_path(address)
        && address.contains(':')
        && address.parse::<std::net::SocketAddr>().is_ok()
}

/// Resolve `host:port` once at pool construction (no per-request DNS).
pub fn parse_tcp_pool_address(address: &str) -> Result<std::net::SocketAddr, ()> {
    address.parse().map_err(|_| ())
}

/// Resolve `pool_id` → [`PoolEndpoint`] (Unix or TCP) from config.
pub fn resolve_pool_endpoints(
    sorted_pool_names: &[String],
    address_by_name: &HashMap<String, String>,
    transport_by_name: &HashMap<String, String>,
    env_socket: Option<&str>,
) -> Vec<(u32, crate::fcgi_stream::PoolEndpoint)> {
    let mut pools = Vec::new();
    for (pool_id, name) in sorted_pool_names.iter().enumerate() {
        let configured = address_by_name.get(name).map(String::as_str);
        let transport = transport_by_name
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or("auto");

        let address = configured.or_else(|| {
            if pool_id == 0 {
                env_socket
            } else {
                None
            }
        });
        let Some(address) = address else {
            continue;
        };

        let want_unix = match transport {
            "unix" => true,
            "tcp" => false,
            _ => is_unix_socket_path(address),
        };

        if want_unix {
            if let Some(path) = normalize_unix_socket_path(address) {
                pools.push((
                    pool_id as u32,
                    crate::fcgi_stream::PoolEndpoint::Unix(PathBuf::from(path)),
                ));
            }
        } else if let Ok(addr) = parse_tcp_pool_address(address) {
            // SSRF: only literal SocketAddr; no hostname DNS at request time.
            pools.push((pool_id as u32, crate::fcgi_stream::PoolEndpoint::Tcp(addr)));
        }
    }
    pools
}

/// Resolve `pool_id` → unix socket path from config addresses with optional env fallback.
pub fn resolve_unix_socket_pools(
    sorted_pool_names: &[String],
    address_by_name: &HashMap<String, String>,
    env_socket: Option<&str>,
) -> Vec<(u32, PathBuf)> {
    resolve_pool_endpoints(
        sorted_pool_names,
        address_by_name,
        &HashMap::new(),
        env_socket,
    )
    .into_iter()
    .filter_map(|(id, ep)| match ep {
        crate::fcgi_stream::PoolEndpoint::Unix(path) => Some((id, path)),
        crate::fcgi_stream::PoolEndpoint::Tcp(_) => None,
    })
    .collect()
}

#[cfg(test)]
fn test_dispatch_request() -> FcgiDispatchRequest {
    FcgiDispatchRequest {
        pool_id: 0,
        method: "GET".into(),
        request_uri: "/index.php".into(),
        query_string: String::new(),
        script_name: "/index.php".into(),
        script_filename: "/var/www/index.php".into(),
        path_info: None,
        document_root: "/var/www".into(),
        server_name: "localhost".into(),
        server_port: 80,
        remote_addr: "127.0.0.1".into(),
        server_protocol: "HTTP/1.1".into(),
        content_type: None,
        body: Vec::new(),
        headers: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PR5B1_BODY, PR5B1_STDOUT};

    #[test]
    fn transport_timeout_maps_to_gateway_timeout() {
        assert_eq!(
            map_transport_error(TransportError::Timeout),
            FcgiDispatchOutcome::GatewayTimeout
        );
    }

    #[test]
    fn transport_connect_failed_maps_to_bad_gateway() {
        assert_eq!(
            map_transport_error(TransportError::ConnectionFailed),
            FcgiDispatchOutcome::BadGateway
        );
    }

    #[test]
    fn mock_pr5b1_success_transports_status_headers_body() {
        let executor = MockFcgiExecutor::pr5b1_default();
        let outcome = executor.dispatch(&test_dispatch_request());
        match outcome {
            FcgiDispatchOutcome::Success(response) => {
                assert_eq!(response.status, 200);
                assert_eq!(response.body, PR5B1_BODY);
                assert!(response
                    .headers
                    .iter()
                    .any(|(k, v)| k == "content-type" && v == "text/plain"));
                assert!(response
                    .headers
                    .iter()
                    .any(|(k, v)| k == "content-length" && v == "19"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn parse_filters_hop_by_hop_transfer_encoding() {
        let stdout = b"Content-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\nbody";
        let parsed = parse_cgi_stdout(stdout).expect("parse");
        assert!(!parsed.headers.iter().any(|(k, _)| k == "transfer-encoding"));
    }

    #[test]
    fn client_decode_error_maps_to_bad_gateway() {
        use crate::DecodeError;
        assert_eq!(
            map_client_error(ClientError::Decode(DecodeError::MissingEndRequest)),
            FcgiDispatchOutcome::BadGateway
        );
    }

    #[test]
    fn pr5b1_stdout_roundtrip() {
        let parsed = parse_cgi_stdout(PR5B1_STDOUT).expect("parse");
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.body, PR5B1_BODY);
    }

    #[test]
    fn production_unix_executor_does_not_return_mock_body() {
        let exec = FcgiModuleExecutor::production_unix(
            PathBuf::from("/nonexistent/exyonq-fcgi-test.sock"),
            Duration::from_millis(50),
        );
        let outcome = exec.dispatch(&test_dispatch_request());
        match outcome {
            FcgiDispatchOutcome::BadGateway | FcgiDispatchOutcome::GatewayTimeout => {}
            FcgiDispatchOutcome::Success(response) => {
                assert_ne!(
                    response.body, PR5B1_BODY,
                    "mock body leaked into production"
                );
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn resolve_pools_prefers_config_over_env() {
        let mut addresses = HashMap::new();
        addresses.insert("php".into(), "/run/from-config.sock".into());
        let pools =
            resolve_unix_socket_pools(&["php".into()], &addresses, Some("/run/from-env.sock"));
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].1, PathBuf::from("/run/from-config.sock"));
    }

    #[test]
    fn resolve_pools_tcp_literal_prefers_config_over_env() {
        let mut addresses = HashMap::new();
        addresses.insert("php".into(), "127.0.0.1:9000".into());
        let pools = resolve_pool_endpoints(
            &["php".into()],
            &addresses,
            &HashMap::new(),
            Some("/run/from-env.sock"),
        );
        assert_eq!(pools.len(), 1);
        match &pools[0].1 {
            crate::fcgi_stream::PoolEndpoint::Tcp(addr) => {
                assert_eq!(*addr, "127.0.0.1:9000".parse().expect("addr"));
            }
            other => panic!("expected TCP endpoint, got {other:?}"),
        }
    }

    #[test]
    fn resolve_unix_socket_pools_skips_tcp_config() {
        let mut addresses = HashMap::new();
        addresses.insert("php".into(), "127.0.0.1:9000".into());
        let pools = resolve_unix_socket_pools(&["php".into()], &addresses, None);
        assert!(pools.is_empty());
    }

    #[test]
    fn resolve_pools_env_when_no_config_address() {
        let addresses = HashMap::new();
        let pools =
            resolve_unix_socket_pools(&["php".into()], &addresses, Some("/run/from-env.sock"));
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].1, PathBuf::from("/run/from-env.sock"));
    }

    #[test]
    fn parse_rejects_oversized_cgi_header_block() {
        let mut stdout = vec![b'X'; MAX_CGI_HEADER_BYTES + 1];
        stdout.extend_from_slice(b"\r\n\r\nbody");
        assert!(parse_cgi_stdout(&stdout).is_err());
    }

    #[test]
    fn parse_rejects_too_many_cgi_headers() {
        let mut header = String::new();
        for index in 0..=MAX_CGI_HEADER_COUNT {
            header.push_str(&format!("X-{index}: v\r\n"));
        }
        header.push_str("\r\nbody");
        assert!(parse_cgi_stdout(header.as_bytes()).is_err());
    }
}
