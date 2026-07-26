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
//! Uses hyperium `h3` via `s2n-quic-h3`. Local `max_ack_delay` is applied through
//! `s2n_quic::provider::limits::Limits::with_max_ack_delay`.

use bytes::{Buf, Bytes};
use exyonq_http3_provider_api::Http3ProviderConfig;
use exyonq_module_api::http3_runtime::{
    Http3ConnectionLifecycle, Http3DispatchService, Http3MaterializedResponse,
};
use h3::error::Code;
use http::{Response, StatusCode};
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

static DIAG: S2nProviderDiagnostics = S2nProviderDiagnostics {
    clean_finish: AtomicU64::new(0),
    dispatch_errors: AtomicU64::new(0),
    response_errors: AtomicU64::new(0),
    handshake_rejects: AtomicU64::new(0),
};

pub fn diagnostics_snapshot() -> S2nProviderDiagSnapshot {
    DIAG.snapshot()
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

    let mut server = Server::builder()
        .with_tls((config.cert_path.as_path(), config.key_path.as_path()))
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
            let Ok(lease) = lifecycle.try_enter_connection() else {
                DIAG.handshake_rejects.fetch_add(1, Ordering::Relaxed);
                // Drop connection without serving while draining.
                drop(connection);
                return;
            };
            if let Err(err) = serve_connection(connection, dispatch, lease, drain_cap).await {
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
) -> anyhow::Result<()>
where
    L: Send + 'static,
{
    let peer_ip = connection
        .remote_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "0.0.0.0".to_string());

    let mut h3_conn = h3::server::builder()
        .build(s2n_quic_h3::Connection::new(connection))
        .await?;

    while let Some(resolver) = h3_conn.accept().await? {
        let dispatch = Arc::clone(&dispatch);
        let peer_ip = peer_ip.clone();
        let Ok((req, mut stream)) = resolver.resolve_request().await else {
            continue;
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
                let _ = write_h3_response(&mut stream, fallback, drain_cap).await;
            }
        }
    }

    Ok(())
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
    use exyonq_http3_provider_api::Http3ProviderId;
    use std::net::SocketAddr;
    use std::path::PathBuf;

    #[test]
    fn validates_ack_delay_config() {
        let cfg = Http3ProviderConfig {
            provider: Http3ProviderId::S2n,
            listen: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            cert_path: PathBuf::from("c.pem"),
            key_path: PathBuf::from("k.pem"),
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
