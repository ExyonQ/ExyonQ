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
//! Quinn + hyperium h3 provider (default).

use bytes::{Buf, Bytes};
use exyonq_mod_tls::load_rustls_config;
use exyonq_module_api::http3_runtime::{
    Http3ConnectionLifecycle, Http3DispatchService, Http3MaterializedResponse,
};
use exyonq_module_api::proxy_dispatch::PROXY_MAX_REQUEST_BODY_BYTES;
use h3::error::Code;
use h3_quinn::Connection as H3QuinnConnection;
use http::{Request, Response, StatusCode};
use quinn::{Endpoint, ServerConfig as QuinnServerConfig, VarInt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::Http3Settings;

/// Cap discarded request-body bytes before aborting with H3_NO_ERROR (DoS bound).
const H3_REQUEST_DRAIN_MAX_BYTES: usize = 64 * 1024;

/// Process-local count of failed H3 response / error-response writes (Quinn legacy).
static RESPONSE_WRITE_ERRORS: AtomicU64 = AtomicU64::new(0);

/// Snapshot of Quinn-legacy response write failures (test / ops observation).
pub(crate) fn response_write_errors() -> u64 {
    RESPONSE_WRITE_ERRORS.load(Ordering::Relaxed)
}

pub(crate) async fn serve<LC>(
    settings: Http3Settings,
    dispatch: Arc<dyn Http3DispatchService>,
    lifecycle: Arc<LC>,
) -> anyhow::Result<()>
where
    LC: Http3ConnectionLifecycle + Send + Sync + 'static,
{
    let rustls = load_rustls_config(&settings.tls, &[b"h3"], None)?;
    let quic = quinn::crypto::rustls::QuicServerConfig::try_from(rustls)
        .map_err(|err| anyhow::anyhow!("quic tls: {err}"))?;
    let mut server_config = QuinnServerConfig::with_crypto(Arc::new(quic));
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(VarInt::from_u32(256));
    transport.send_window(1024 * 1024);
    transport.receive_window(VarInt::from_u32(1024 * 1024));
    transport.max_idle_timeout(Some(Duration::from_secs(30).try_into()?));
    server_config.transport = Arc::new(transport);

    let endpoint = Endpoint::server(server_config, settings.listen)?;
    info!(listen = %settings.listen, "HTTP/3 listening");

    while let Some(incoming) = endpoint.accept().await {
        let dispatch = Arc::clone(&dispatch);
        let lifecycle = Arc::clone(&lifecycle);
        tokio::spawn(async move {
            let conn = match incoming.await {
                Ok(conn) => conn,
                Err(err) => {
                    warn!(%err, "quic handshake failed");
                    return;
                }
            };
            let Ok(lease) = lifecycle.try_enter_connection() else {
                conn.close(VarInt::from_u32(0), b"draining");
                return;
            };
            if let Err(err) = serve_quic_connection(conn, dispatch, lease).await {
                warn!(%err, "http3 connection ended with error");
            }
        });
    }

    Ok(())
}

async fn serve_quic_connection<L>(
    conn: quinn::Connection,
    dispatch: Arc<dyn Http3DispatchService>,
    _lease: L,
) -> anyhow::Result<()>
where
    L: Send + 'static,
{
    let mut h3_conn = h3::server::Connection::new(H3QuinnConnection::new(conn)).await?;

    while let Some(resolver) = h3_conn.accept().await? {
        let dispatch = Arc::clone(&dispatch);
        let Ok((req, mut stream)) = resolver.resolve_request().await else {
            continue;
        };
        let peer_ip = "127.0.0.1";
        let req = match collect_request_body(req, &mut stream).await {
            Ok(req) => req,
            Err(RequestBodyReadError::TooLarge) => {
                let response = payload_too_large_response();
                // Required error-response write: failure is authoritative for emission,
                // never treated as a successfully emitted 413.
                if let Err(err) = write_h3_response(&mut stream, response).await {
                    RESPONSE_WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
                    warn!(%err, "http3 error response failed after payload-too-large");
                }
                continue;
            }
            Err(RequestBodyReadError::Body) => {
                let response = bad_request_response();
                if let Err(err) = write_h3_response(&mut stream, response).await {
                    RESPONSE_WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
                    warn!(%err, "http3 error response failed after bad-request body");
                }
                continue;
            }
        };
        match dispatch.dispatch(req, peer_ip).await {
            Ok(response) => {
                if let Err(err) = write_h3_response(&mut stream, response).await {
                    RESPONSE_WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
                    warn!(%err, "http3 response failed");
                }
            }
            Err(err) => {
                warn!(%err, "http3 dispatch failed");
                let fallback = Http3MaterializedResponse {
                    status: StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    headers: vec![(
                        "content-type".to_string(),
                        "text/plain; charset=utf-8".to_string(),
                    )],
                    body: Bytes::from_static(b"internal server error"),
                };
                if let Err(write_err) = write_h3_response(&mut stream, fallback).await {
                    RESPONSE_WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        %write_err,
                        "http3 error response failed after dispatch error"
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

fn request_body_cap() -> usize {
    H3_REQUEST_DRAIN_MAX_BYTES.min(PROXY_MAX_REQUEST_BODY_BYTES)
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

async fn collect_request_body(
    req: Request<()>,
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
) -> Result<Request<Bytes>, RequestBodyReadError> {
    let max_bytes = request_body_cap();
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

/// Drain the client→server half so Quinn RecvStream Drop does not emit
/// `STOP_SENDING(app_error_code=0)` after a successful response (P13F).
async fn drain_request_recv(
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
) {
    let mut discarded = 0usize;
    loop {
        match stream.recv_data().await {
            Ok(Some(chunk)) => {
                discarded = discarded.saturating_add(chunk.remaining());
                if discarded > H3_REQUEST_DRAIN_MAX_BYTES {
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

async fn write_h3_response(
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    materialized: Http3MaterializedResponse,
) -> anyhow::Result<()> {
    drain_request_recv(stream).await;

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
