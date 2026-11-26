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
//! s2n-quic HTTP/3 provider (P13D Phase 3) — sole authorized `s2n-quic` zone.
//!
//! Uses hyperium `h3` via `tachyon-quic` (h3↔s2n-quic binding; replaces yanked `s2n-quic-h3`).
//! Local `max_ack_delay` is applied through
//! `s2n_quic::provider::limits::Limits::with_max_ack_delay`.

use bytes::{Buf, Bytes};
use exyonq_http3_provider_api::Http3ProviderConfig;
use exyonq_module_api::http3_runtime::{
    Http3ConnectionLifecycle, Http3DispatchService, Http3MaterializedResponse,
};
use exyonq_module_api::proxy_dispatch::PROXY_MAX_REQUEST_BODY_BYTES;
use h3::error::Code;
use http::{Request, Response, StatusCode};
use s2n_quic::provider::limits::Limits;
use s2n_quic::Server;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Process-local diagnostics for P13D Phase 3.
#[derive(Debug, Default)]
pub struct S2nProviderDiagnostics {
    pub clean_finish: AtomicU64,
    pub dispatch_errors: AtomicU64,
    pub response_errors: AtomicU64,
    pub handshake_rejects: AtomicU64,
}

impl S2nProviderDiagnostics {
    pub fn snapshot(&self) -> S2nProviderDiagSnapshot {
        S2nProviderDiagSnapshot {
            provider: "s2n",
            clean_finish: self.clean_finish.load(Ordering::Relaxed),
            dispatch_errors: self.dispatch_errors.load(Ordering::Relaxed),
            response_errors: self.response_errors.load(Ordering::Relaxed),
            handshake_rejects: self.handshake_rejects.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct S2nProviderDiagSnapshot {
    pub provider: &'static str,
    pub clean_finish: u64,
    pub dispatch_errors: u64,
    pub response_errors: u64,
    pub handshake_rejects: u64,
}

/// NG-21: header-block rejections (over budget) and header-deadline cuts.
static HEADER_REJECTS: AtomicU64 = AtomicU64::new(0);
static HEADER_DEADLINES: AtomicU64 = AtomicU64::new(0);

/// `(rejected_over_budget, cut_by_deadline)` for the HTTP/3 header limits.
pub fn header_limit_stats() -> (u64, u64) {
    (
        HEADER_REJECTS.load(Ordering::Relaxed),
        HEADER_DEADLINES.load(Ordering::Relaxed),
    )
}

static DIAG: S2nProviderDiagnostics = S2nProviderDiagnostics {
    clean_finish: AtomicU64::new(0),
    dispatch_errors: AtomicU64::new(0),
    response_errors: AtomicU64::new(0),
    handshake_rejects: AtomicU64::new(0),
};

pub fn diagnostics_snapshot() -> S2nProviderDiagSnapshot {
    DIAG.snapshot()
}

/// Build the s2n-tls server from every configured pair. Each chain goes into
/// the s2n certificate store (`load_chain`), which selects the certificate by
/// client SNI; the first loaded pair is the default for unmatched names.
fn build_tls(
    certificates: &[exyonq_http3_provider_api::Http3Certificate],
) -> anyhow::Result<s2n_quic::provider::tls::s2n_tls::Server> {
    use s2n_quic::provider::tls::s2n_tls;

    if certificates.is_empty() {
        anyhow::bail!("http3 tls: no certificate configured");
    }
    let mut tls = s2n_tls::Server::builder();
    for entry in certificates {
        let cert_pem = std::fs::read(&entry.cert_path)
            .map_err(|err| anyhow::anyhow!("s2n tls cert {}: {err}", entry.cert_path.display()))?;
        let key_pem = std::fs::read(&entry.key_path)
            .map_err(|err| anyhow::anyhow!("s2n tls key {}: {err}", entry.key_path.display()))?;
        let mut chain = s2n_tls::cert_chain::Builder::new()
            .map_err(|err| anyhow::anyhow!("s2n cert chain: {err}"))?;
        chain.load_pem(&cert_pem, &key_pem).map_err(|err| {
            anyhow::anyhow!("s2n cert chain {}: {err}", entry.cert_path.display())
        })?;
        let chain = chain.build().map_err(|err| {
            anyhow::anyhow!("s2n cert chain {}: {err}", entry.cert_path.display())
        })?;
        tls.config_mut().load_chain(chain).map_err(|err| {
            anyhow::anyhow!("s2n load_chain {}: {err}", entry.cert_path.display())
        })?;
    }
    tls.build()
        .map_err(|err| anyhow::anyhow!("s2n tls build: {err}"))
}

/// Serve HTTP/3 via s2n-quic until the acceptor closes.
pub async fn serve<LC>(
    config: Http3ProviderConfig,
    dispatch: Arc<dyn Http3DispatchService>,
    lifecycle: Arc<LC>,
) -> anyhow::Result<()>
where
    LC: Http3ConnectionLifecycle + Send + Sync + 'static,
{
    config.validate().map_err(|e| anyhow::anyhow!("{e}"))?;

    let limits = Limits::default()
        .with_max_ack_delay(Duration::from_millis(config.max_ack_delay_ms))
        .map_err(|e| anyhow::anyhow!("s2n limits max_ack_delay: {e}"))?
        .with_max_idle_timeout(Duration::from_millis(config.idle_timeout_ms))
        .map_err(|e| anyhow::anyhow!("s2n limits idle_timeout: {e}"))?
        .with_max_open_remote_bidirectional_streams(config.max_concurrent_streams)
        .map_err(|e| anyhow::anyhow!("s2n limits streams: {e}"))?;

    let tls = build_tls(&config.certificates)?;
    let mut server = Server::builder()
        .with_tls(tls)
        .map_err(|e| anyhow::anyhow!("s2n tls: {e}"))?
        .with_io(config.listen)
        .map_err(|e| anyhow::anyhow!("s2n io: {e}"))?
        .with_limits(limits)
        .map_err(|e| anyhow::anyhow!("s2n limits: {e}"))?
        .start()
        .map_err(|e| anyhow::anyhow!("s2n start: {e}"))?;

    info!(
        provider = "s2n",
        listen = %config.listen,
        max_ack_delay_ms = config.max_ack_delay_ms,
        drain_cap = config.request_body_drain_cap_bytes,
        "HTTP/3 listening (s2n-quic provider)"
    );

    let drain_cap = config.request_body_drain_cap_bytes;
    while let Some(connection) = server.accept().await {
        let dispatch = Arc::clone(&dispatch);
        let lifecycle = Arc::clone(&lifecycle);
        tokio::spawn(async move {
            let peer_ip = connection
                .remote_addr()
                .map(|a| a.ip().to_string())
                .unwrap_or_else(|_| "0.0.0.0".to_string());
            let Ok(lease) = lifecycle.try_enter_connection_for(&peer_ip) else {
                DIAG.handshake_rejects.fetch_add(1, Ordering::Relaxed);
                // Drop connection without serving while draining or over limit.
                drop(connection);
                return;
            };
            if let Err(err) =
                serve_connection(connection, dispatch, lease, drain_cap, peer_ip).await
            {
                warn!(%err, "s2n http3 connection ended with error");
            }
        });
    }

    Ok(())
}

async fn serve_connection<L>(
    connection: s2n_quic::Connection,
    dispatch: Arc<dyn Http3DispatchService>,
    _lease: L,
    drain_cap: usize,
    peer_ip: String,
) -> anyhow::Result<()>
where
    L: Send + 'static,
{
    let mut h3_conn = h3::server::builder()
        .build(tachyon_quic::Connection::new(connection))
        .await?;

    while let Some(resolver) = h3_conn.accept().await? {
        let dispatch = Arc::clone(&dispatch);
        let peer_ip = peer_ip.clone();
        // NG-21: the committed header geometry applies here too. A client that
        // does not deliver a complete request head within the deadline is cut
        // exactly as H1 cuts one (the connection ends without a response),
        // because the QUIC stream id is only known once `resolve_request` has
        // produced the head and its `RequestStream`.
        let resolved = match tokio::time::timeout(
            exyonq_module_api::http_limits::header_deadline(),
            resolver.resolve_request(),
        )
        .await
        {
            Ok(resolved) => resolved,
            Err(_elapsed) => {
                HEADER_DEADLINES.fetch_add(1, Ordering::Relaxed);
                warn!(
                    peer_ip = %peer_ip,
                    "s2n http3 header deadline elapsed; closing connection"
                );
                return Ok(());
            }
        };
        let Ok((req, mut stream)) = resolved else {
            continue;
        };
        if !exyonq_module_api::http_limits::header_list_within_budget(req.headers()) {
            HEADER_REJECTS.fetch_add(1, Ordering::Relaxed);
            warn!(
                peer_ip = %peer_ip,
                header_bytes = exyonq_module_api::http_limits::header_list_bytes(req.headers()),
                budget = exyonq_module_api::http_limits::header_budget_bytes(),
                "s2n http3 header block over budget; resetting stream"
            );
            // Same observable rejection as HTTP/2: no content is served and the
            // client gets 431, written through the existing error-response path.
            let response = header_fields_too_large_response();
            if let Err(err) = write_h3_response(&mut stream, response, drain_cap).await {
                DIAG.response_errors.fetch_add(1, Ordering::Relaxed);
                warn!(%err, "s2n http3 431 write failed");
            }
            continue;
        }
        let req = match collect_request_body(req, &mut stream, drain_cap).await {
            Ok(req) => req,
            Err(RequestBodyReadError::TooLarge) => {
                let response = payload_too_large_response();
                // Required error-response write: failure is authoritative for emission,
                // never treated as a successfully emitted 413.
                if let Err(err) = write_h3_response(&mut stream, response, drain_cap).await {
                    DIAG.response_errors.fetch_add(1, Ordering::Relaxed);
                    warn!(%err, "s2n http3 error response failed after payload-too-large");
                }
                continue;
            }
            Err(RequestBodyReadError::Body) => {
                DIAG.dispatch_errors.fetch_add(1, Ordering::Relaxed);
                let response = bad_request_response();
                if let Err(err) = write_h3_response(&mut stream, response, drain_cap).await {
                    DIAG.response_errors.fetch_add(1, Ordering::Relaxed);
                    warn!(%err, "s2n http3 error response failed after bad-request body");
                }
                continue;
            }
        };
        match dispatch.dispatch(req, &peer_ip).await {
            Ok(response) => {
                if let Err(err) = write_h3_response(&mut stream, response, drain_cap).await {
                    DIAG.response_errors.fetch_add(1, Ordering::Relaxed);
                    warn!(%err, "s2n http3 response failed");
                } else {
                    DIAG.clean_finish.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(err) => {
                DIAG.dispatch_errors.fetch_add(1, Ordering::Relaxed);
                warn!(%err, "s2n http3 dispatch failed");
                let fallback = Http3MaterializedResponse {
                    status: StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    headers: vec![(
                        "content-type".to_string(),
                        "text/plain; charset=utf-8".to_string(),
                    )],
                    body: Bytes::from_static(b"internal server error"),
                };
                if let Err(write_err) = write_h3_response(&mut stream, fallback, drain_cap).await {
                    DIAG.response_errors.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        %write_err,
                        "s2n http3 error response failed after dispatch error"
                    );
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestBodyReadError {
    Body,
    TooLarge,
}

fn request_body_cap(drain_cap: usize) -> usize {
    drain_cap.min(PROXY_MAX_REQUEST_BODY_BYTES)
}

fn content_length(req: &Request<()>) -> Option<usize> {
    req.headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
}

fn accept_request_chunk(current_len: usize, chunk_len: usize, max_bytes: usize) -> Option<usize> {
    current_len
        .checked_add(chunk_len)
        .filter(|total| *total <= max_bytes)
}

async fn collect_request_body<S>(
    req: Request<()>,
    stream: &mut h3::server::RequestStream<S, Bytes>,
    drain_cap: usize,
) -> Result<Request<Bytes>, RequestBodyReadError>
where
    S: h3::quic::BidiStream<Bytes>,
{
    let max_bytes = request_body_cap(drain_cap);
    if content_length(&req).is_some_and(|len| len > max_bytes) {
        stream.stop_sending(Code::H3_NO_ERROR);
        return Err(RequestBodyReadError::TooLarge);
    }

    let mut buf = Vec::new();
    loop {
        match stream.recv_data().await {
            Ok(Some(mut chunk)) => {
                let chunk_len = chunk.remaining();
                if chunk_len == 0 {
                    continue;
                }
                if accept_request_chunk(buf.len(), chunk_len, max_bytes).is_none() {
                    drop(buf);
                    stream.stop_sending(Code::H3_NO_ERROR);
                    return Err(RequestBodyReadError::TooLarge);
                }
                while chunk.has_remaining() {
                    let bytes = chunk.chunk();
                    buf.extend_from_slice(bytes);
                    chunk.advance(bytes.len());
                }
            }
            Ok(None) => break,
            Err(_) => {
                stream.stop_sending(Code::H3_NO_ERROR);
                return Err(RequestBodyReadError::Body);
            }
        }
    }
    let _ = stream.recv_trailers().await;
    let (parts, _) = req.into_parts();
    let mut req = Request::from_parts(parts, Bytes::from(buf));
    let len = req.body().len().to_string();
    if let Ok(value) = http::HeaderValue::from_str(&len) {
        req.headers_mut()
            .insert(http::header::CONTENT_LENGTH, value);
    }
    Ok(req)
}

async fn drain_request_recv<S>(stream: &mut h3::server::RequestStream<S, Bytes>, drain_cap: usize)
where
    S: h3::quic::BidiStream<Bytes>,
{
    let mut discarded = 0usize;
    loop {
        match stream.recv_data().await {
            Ok(Some(chunk)) => {
                discarded = discarded.saturating_add(chunk.remaining());
                if discarded > drain_cap {
                    stream.stop_sending(Code::H3_NO_ERROR);
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => {
                stream.stop_sending(Code::H3_NO_ERROR);
                break;
            }
        }
    }
    let _ = stream.recv_trailers().await;
}

fn payload_too_large_response() -> Http3MaterializedResponse {
    Http3MaterializedResponse {
        status: StatusCode::PAYLOAD_TOO_LARGE.as_u16(),
        headers: vec![(
            "content-type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )],
        body: Bytes::from_static(b"payload too large"),
    }
}

fn header_fields_too_large_response() -> Http3MaterializedResponse {
    Http3MaterializedResponse {
        status: StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE.as_u16(),
        headers: vec![(
            "content-type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )],
        body: Bytes::from_static(b"request header fields too large"),
    }
}

fn bad_request_response() -> Http3MaterializedResponse {
    Http3MaterializedResponse {
        status: StatusCode::BAD_REQUEST.as_u16(),
        headers: vec![(
            "content-type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )],
        body: Bytes::from_static(b"bad request"),
    }
}

async fn write_h3_response<S>(
    stream: &mut h3::server::RequestStream<S, Bytes>,
    materialized: Http3MaterializedResponse,
    drain_cap: usize,
) -> anyhow::Result<()>
where
    S: h3::quic::BidiStream<Bytes>,
{
    // P13F: drain request recv before responding to avoid post-body resets.
    drain_request_recv(stream, drain_cap).await;

    let Http3MaterializedResponse {
        status,
        headers,
        body,
    } = materialized;
    let mut builder = Response::builder().status(status);
    for (name, value) in &headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let response = builder.body(())?;
    stream.send_response(response).await?;
    if !body.is_empty() {
        stream.send_data(body).await?;
    }
    stream.finish().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_http3_provider_api::{Http3Certificate, Http3ProviderId};
    use std::net::SocketAddr;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn openssl_bin() -> &'static str {
        for candidate in [
            "/opt/homebrew/opt/openssl@3/bin/openssl",
            "/usr/local/opt/openssl@3/bin/openssl",
        ] {
            if Path::new(candidate).is_file() {
                return candidate;
            }
        }
        "openssl"
    }

    fn write_pair(dir: &Path, stem: &str, san: &str) -> (PathBuf, PathBuf) {
        let cert = dir.join(format!("{stem}.pem"));
        let key = dir.join(format!("{stem}.key"));
        let status = Command::new(openssl_bin())
            .args(["req", "-x509", "-newkey", "rsa:2048", "-keyout"])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .args(["-days", "1", "-nodes", "-subj", "/CN=exyonq-s2n-test"])
            .args(["-addext", &format!("subjectAltName={san}")])
            .status()
            .expect("openssl available for ephemeral QUIC TLS fixtures");
        assert!(status.success(), "openssl SAN cert generation failed");
        (cert, key)
    }

    /// NG-15 over QUIC: both `[[server]]` pairs enter the s2n certificate
    /// store (SNI selection by SAN); an empty list fails closed.
    #[test]
    fn build_tls_loads_every_pair_into_the_sni_store() {
        let dir = std::env::temp_dir().join(format!("exyonq-s2n-multi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (cert_a, key_a) = write_pair(&dir, "a", "DNS:a.test");
        let (cert_b, key_b) = write_pair(&dir, "b", "DNS:b.test");
        let certificates = vec![
            Http3Certificate {
                cert_path: cert_a,
                key_path: key_a,
            },
            Http3Certificate {
                cert_path: cert_b,
                key_path: key_b,
            },
        ];
        build_tls(&certificates).expect("both chains load into the s2n SNI store");
        assert!(
            build_tls(&[]).is_err(),
            "empty certificate list fails closed"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validates_ack_delay_config() {
        let cfg = Http3ProviderConfig {
            provider: Http3ProviderId::S2n,
            listen: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            certificates: vec![exyonq_http3_provider_api::Http3Certificate {
                cert_path: PathBuf::from("c.pem"),
                key_path: PathBuf::from("k.pem"),
            }],
            max_ack_delay_ms: 1,
            qlog_enabled: false,
            request_body_drain_cap_bytes: 64 * 1024,
            idle_timeout_ms: 30_000,
            max_concurrent_streams: 256,
            initial_stream_window: 1_000_000,
            initial_connection_window: 10_000_000,
        };
        assert!(cfg.validate().is_ok());
        let limits = Limits::default()
            .with_max_ack_delay(Duration::from_millis(cfg.max_ack_delay_ms))
            .expect("max_ack_delay");
        let _ = limits;
    }

    #[test]
    fn diagnostics_start_zero() {
        let snap = diagnostics_snapshot();
        assert_eq!(snap.provider, "s2n");
    }
}
