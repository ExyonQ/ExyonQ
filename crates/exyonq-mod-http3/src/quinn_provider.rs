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
use h3::error::Code;
use h3_quinn::Connection as H3QuinnConnection;
use http::{Response, StatusCode};
use quinn::{Endpoint, ServerConfig as QuinnServerConfig, VarInt};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::Http3Settings;

/// Cap discarded request-body bytes before aborting with H3_NO_ERROR (DoS bound).
const H3_REQUEST_DRAIN_MAX_BYTES: usize = 64 * 1024;

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
                let _ = conn.close(VarInt::from_u32(0), b"draining");
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
        match dispatch.dispatch(req, peer_ip).await {
            Ok(response) => {
                if let Err(err) = write_h3_response(&mut stream, response).await {
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
                let _ = write_h3_response(&mut stream, fallback).await;
            }
        }
    }

    Ok(())
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
