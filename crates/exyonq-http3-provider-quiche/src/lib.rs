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
//! Quiche HTTP/3 provider (P13D Phase 4) — sole authorized `quiche` zone.
//!
//! Event loop adapted from quiche examples; dispatch stays on
//! `Http3DispatchService` (provider-neutral). Local ACK floor via
//! `quiche::Config::set_max_ack_delay`.

use bytes::Bytes;
use exyonq_http3_provider_api::{Http3ProviderConfig, Http3ProviderListen};
use exyonq_module_api::http3_runtime::{
    Http3ConnectionLifecycle, Http3DispatchService, Http3MaterializedResponse,
};
use http::{Method, Request, Uri};
use quiche::h3::NameValue;
use ring::rand::*;
use std::collections::HashMap;
use std::net;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

const MAX_DATAGRAM_SIZE: usize = 1350;

/// Process-local diagnostics for P13D Phase 4.
#[derive(Debug, Default)]
pub struct QuicheProviderDiagnostics {
    pub max_ack_delay_ms_configured: AtomicU64,
    pub clean_finish: AtomicU64,
    pub dispatch_errors: AtomicU64,
    pub response_errors: AtomicU64,
    pub handshake_rejects: AtomicU64,
    pub h3_poll_errors: AtomicU64,
}

#[derive(Debug, Clone, Copy)]
pub struct QuicheProviderDiagSnapshot {
    pub provider: &'static str,
    pub max_ack_delay_ms_configured: u64,
    pub clean_finish: u64,
    pub dispatch_errors: u64,
    pub response_errors: u64,
    pub handshake_rejects: u64,
    pub h3_poll_errors: u64,
}

impl QuicheProviderDiagnostics {
    pub fn snapshot(&self) -> QuicheProviderDiagSnapshot {
        QuicheProviderDiagSnapshot {
            provider: "quiche",
            max_ack_delay_ms_configured: self.max_ack_delay_ms_configured.load(Ordering::Relaxed),
            clean_finish: self.clean_finish.load(Ordering::Relaxed),
            dispatch_errors: self.dispatch_errors.load(Ordering::Relaxed),
            response_errors: self.response_errors.load(Ordering::Relaxed),
            handshake_rejects: self.handshake_rejects.load(Ordering::Relaxed),
            h3_poll_errors: self.h3_poll_errors.load(Ordering::Relaxed),
        }
    }
}

static DIAG: QuicheProviderDiagnostics = QuicheProviderDiagnostics {
    max_ack_delay_ms_configured: AtomicU64::new(0),
    clean_finish: AtomicU64::new(0),
    dispatch_errors: AtomicU64::new(0),
    response_errors: AtomicU64::new(0),
    handshake_rejects: AtomicU64::new(0),
    h3_poll_errors: AtomicU64::new(0),
};

pub fn diagnostics_snapshot() -> QuicheProviderDiagSnapshot {
    DIAG.snapshot()
}

struct PartialResponse {
    headers: Option<Vec<quiche::h3::Header>>,
    body: Vec<u8>,
    written: usize,
}

struct Client<L> {
    conn: quiche::Connection,
    http3_conn: Option<quiche::h3::Connection>,
    partial_responses: HashMap<u64, PartialResponse>,
    /// Held for connection lifetime (PS1A-H3 / P13F admission).
    _lease: Option<L>,
    peer: net::SocketAddr,
}

type ClientMap<L> = HashMap<quiche::ConnectionId<'static>, Client<L>>;

/// Run quiche HTTP/3 listen loop until the UDP socket fails fatally.
pub async fn serve<LC>(
    config: Http3ProviderConfig,
    dispatch: Arc<dyn Http3DispatchService>,
    lifecycle: Arc<LC>,
) -> anyhow::Result<()>
where
    LC: Http3ConnectionLifecycle + Send + Sync + 'static,
{
    config.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || run_blocking(config, dispatch, lifecycle, handle)).await?
}

/// Backward-compatible entry used by early P13P drafts.
pub async fn serve_listen<LC>(
    listen: Http3ProviderListen,
    dispatch: Arc<dyn Http3DispatchService>,
    lifecycle: Arc<LC>,
) -> anyhow::Result<()>
where
    LC: Http3ConnectionLifecycle + Send + Sync + 'static,
{
    serve(Http3ProviderConfig::from(listen), dispatch, lifecycle).await
}

fn run_blocking<LC>(
    cfg: Http3ProviderConfig,
    dispatch: Arc<dyn Http3DispatchService>,
    lifecycle: Arc<LC>,
    handle: tokio::runtime::Handle,
) -> anyhow::Result<()>
where
    LC: Http3ConnectionLifecycle + Send + Sync + 'static,
{
    let mut buf = [0; 65535];
    let mut out = [0; MAX_DATAGRAM_SIZE];
    let drain_cap = cfg.request_body_drain_cap_bytes;

    let mut poll = mio::Poll::new()?;
    let mut events = mio::Events::with_capacity(1024);

    let mut socket = mio::net::UdpSocket::bind(cfg.listen)?;
    poll.registry()
        .register(&mut socket, mio::Token(0), mio::Interest::READABLE)?;

    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION)?;
    config.load_cert_chain_from_pem_file(
        cfg.cert_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("cert path not utf-8"))?,
    )?;
    config.load_priv_key_from_pem_file(
        cfg.key_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("key path not utf-8"))?,
    )?;
    config.set_application_protos(quiche::h3::APPLICATION_PROTOCOL)?;
    config.set_max_idle_timeout(cfg.idle_timeout_ms);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(cfg.initial_connection_window);
    config.set_initial_max_stream_data_bidi_local(cfg.initial_stream_window);
    config.set_initial_max_stream_data_bidi_remote(cfg.initial_stream_window);
    config.set_initial_max_stream_data_uni(cfg.initial_stream_window);
    config.set_initial_max_streams_bidi(cfg.max_concurrent_streams);
    config.set_initial_max_streams_uni(cfg.max_concurrent_streams);
    config.set_disable_active_migration(true);
    config.enable_early_data();
    // P13D Phase 4: remove Quinn's 25 ms delayed-ACK floor (quiche default is 25).
    config.set_max_ack_delay(cfg.max_ack_delay_ms);
    DIAG.max_ack_delay_ms_configured
        .store(cfg.max_ack_delay_ms, Ordering::Relaxed);
    info!(
        provider = "quiche",
        max_ack_delay_ms = cfg.max_ack_delay_ms,
        drain_cap,
        listen = %cfg.listen,
        "HTTP/3 listening (quiche provider)"
    );

    let h3_config = quiche::h3::Config::new()?;
    let rng = SystemRandom::new();
    let conn_id_seed = ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng)
        .map_err(|_| anyhow::anyhow!("hmac keygen failed"))?;

    let mut clients: ClientMap<LC::Lease> = HashMap::new();
    let local_addr = socket.local_addr()?;

    loop {
        let timeout = clients.values().filter_map(|c| c.conn.timeout()).min();
        poll.poll(&mut events, timeout)?;

        'read: loop {
            if events.is_empty() {
                clients.values_mut().for_each(|c| c.conn.on_timeout());
                break 'read;
            }

            let (len, from) = match socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break 'read,
                Err(e) => return Err(e.into()),
            };

            let pkt_buf = &mut buf[..len];
            let hdr = match quiche::Header::from_slice(pkt_buf, quiche::MAX_CONN_ID_LEN) {
                Ok(v) => v,
                Err(e) => {
                    debug!(?e, "header parse failed");
                    continue 'read;
                }
            };

            let conn_id = ring::hmac::sign(&conn_id_seed, &hdr.dcid);
            let conn_id = &conn_id.as_ref()[..quiche::MAX_CONN_ID_LEN];
            let conn_id = conn_id.to_vec().into();

            if !clients.contains_key(&hdr.dcid) && !clients.contains_key(&conn_id) {
                if hdr.ty != quiche::Type::Initial {
                    continue 'read;
                }
                if !quiche::version_is_supported(hdr.version) {
                    let len = quiche::negotiate_version(&hdr.scid, &hdr.dcid, &mut out)?;
                    let _ = socket.send_to(&out[..len], from);
                    continue 'read;
                }

                let mut scid_buf = [0; quiche::MAX_CONN_ID_LEN];
                scid_buf.copy_from_slice(&conn_id);
                let scid = quiche::ConnectionId::from_ref(&scid_buf);
                let token = hdr.token.as_ref().unwrap();

                if token.is_empty() {
                    let new_token = mint_token(&hdr, &from);
                    let len = quiche::retry(
                        &hdr.scid,
                        &hdr.dcid,
                        &scid,
                        &new_token,
                        hdr.version,
                        &mut out,
                    )?;
                    let _ = socket.send_to(&out[..len], from);
                    continue 'read;
                }

                let odcid = match validate_token(&from, token) {
                    Some(v) => v,
                    None => continue 'read,
                };

                // Must reuse the SCID we advertised in the Retry packet (client's DCID).
                if scid.len() != hdr.dcid.len() {
                    continue 'read;
                }
                let scid = hdr.dcid.clone();
                let scid_owned = scid.clone().into_owned();

                let conn = match quiche::accept(&scid, Some(&odcid), local_addr, from, &mut config)
                {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(?e, "quiche accept failed");
                        continue 'read;
                    }
                };

                let client = Client {
                    conn,
                    http3_conn: None,
                    partial_responses: HashMap::new(),
                    _lease: None,
                    peer: from,
                };
                clients.insert(scid_owned, client);
                // Fall through: process this Initial via the shared recv path below.
            }

            let client = if clients.contains_key(&hdr.dcid) {
                clients.get_mut(&hdr.dcid).unwrap()
            } else {
                clients.get_mut(&conn_id).unwrap()
            };

            let recv_info = quiche::RecvInfo {
                to: local_addr,
                from,
            };
            if let Err(e) = client.conn.recv(pkt_buf, recv_info) {
                if e != quiche::Error::Done {
                    debug!(?e, "recv failed");
                }
                continue 'read;
            }

            if (client.conn.is_in_early_data() || client.conn.is_established())
                && client._lease.is_none()
            {
                match lifecycle.try_enter_connection() {
                    Ok(lease) => client._lease = Some(lease),
                    Err(_) => {
                        DIAG.handshake_rejects.fetch_add(1, Ordering::Relaxed);
                        let _ = client.conn.close(false, 0, b"draining");
                        continue 'read;
                    }
                }
            }

            if (client.conn.is_in_early_data() || client.conn.is_established())
                && client.http3_conn.is_none()
            {
                match quiche::h3::Connection::with_transport(&mut client.conn, &h3_config) {
                    Ok(h3) => client.http3_conn = Some(h3),
                    Err(e) => {
                        warn!(%e, "h3 connection create failed");
                        continue 'read;
                    }
                }
            }

            if client.http3_conn.is_some() {
                for stream_id in client.conn.writable() {
                    handle_writable(client, stream_id);
                }
            }

            if let Some(http3_conn) = client.http3_conn.as_mut() {
                loop {
                    match http3_conn.poll(&mut client.conn) {
                        Ok((stream_id, quiche::h3::Event::Headers { list, .. })) => {
                            let peer = client.peer;
                            handle_request(
                                &mut client.conn,
                                http3_conn,
                                stream_id,
                                &list,
                                &mut client.partial_responses,
                                &dispatch,
                                &handle,
                                &peer,
                            );
                        }
                        Ok((stream_id, quiche::h3::Event::Data)) => {
                            // Ignore leftover request body (Read already shut down on Headers).
                            let _ = stream_id;
                        }
                        Ok((_, quiche::h3::Event::Finished))
                        | Ok((_, quiche::h3::Event::Reset { .. }))
                        | Ok((_, quiche::h3::Event::PriorityUpdate))
                        | Ok((_, quiche::h3::Event::GoAway)) => {}
                        Err(quiche::h3::Error::Done) => break,
                        Err(e) => {
                            DIAG.h3_poll_errors.fetch_add(1, Ordering::Relaxed);
                            warn!(?e, "http3 poll error");
                            break;
                        }
                    }
                }
            }
        }

        for client in clients.values_mut() {
            loop {
                let (write, send_info) = match client.conn.send(&mut out) {
                    Ok(v) => v,
                    Err(quiche::Error::Done) => break,
                    Err(e) => {
                        error!(?e, "send failed");
                        let _ = client.conn.close(false, 0x1, b"fail");
                        break;
                    }
                };
                if let Err(e) = socket.send_to(&out[..write], send_info.to) {
                    if e.kind() == std::io::ErrorKind::WouldBlock {
                        break;
                    }
                    return Err(e.into());
                }
            }
        }

        clients.retain(|_, c| !c.conn.is_closed());
    }
}

// Request-body bound: on Headers we `stream_shutdown(Read)` (official quiche
// http3-server pattern). `request_body_drain_cap_bytes` remains the shared
// config invariant (logged at listen); live byte-cap drain lands before default-on.

fn handle_request(
    conn: &mut quiche::Connection,
    http3_conn: &mut quiche::h3::Connection,
    stream_id: u64,
    headers: &[quiche::h3::Header],
    partial_responses: &mut HashMap<u64, PartialResponse>,
    dispatch: &Arc<dyn Http3DispatchService>,
    handle: &tokio::runtime::Handle,
    peer: &net::SocketAddr,
) {
    let req = match headers_to_request(headers) {
        Ok(r) => r,
        Err(e) => {
            warn!(%e, "bad h3 headers");
            return;
        }
    };

    // Official quiche http3-server pattern: stop reading the request stream so
    // remaining body is ignored and Data events are not generated (P13F-equivalent).
    let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Read, 0);

    let peer_ip = peer.ip().to_string();
    let materialized = match handle.block_on(dispatch.dispatch(req, &peer_ip)) {
        Ok(r) => r,
        Err(e) => {
            DIAG.dispatch_errors.fetch_add(1, Ordering::Relaxed);
            warn!(%e, "dispatch failed");
            Http3MaterializedResponse {
                status: 500,
                headers: vec![("content-type".into(), "text/plain; charset=utf-8".into())],
                body: Bytes::from_static(b"internal server error"),
            }
        }
    };

    let (hdrs, body) = materialized_to_h3(materialized);
    match http3_conn.send_response(conn, stream_id, &hdrs, false) {
        Ok(()) => {}
        Err(quiche::h3::Error::StreamBlocked) => {
            partial_responses.insert(
                stream_id,
                PartialResponse {
                    headers: Some(hdrs),
                    body,
                    written: 0,
                },
            );
            return;
        }
        Err(e) => {
            DIAG.response_errors.fetch_add(1, Ordering::Relaxed);
            warn!(?e, "send_response failed");
            return;
        }
    }

    let written = match http3_conn.send_body(conn, stream_id, &body, true) {
        Ok(v) => v,
        Err(quiche::h3::Error::Done) => 0,
        Err(e) => {
            DIAG.response_errors.fetch_add(1, Ordering::Relaxed);
            warn!(?e, "send_body failed");
            return;
        }
    };
    if written < body.len() {
        partial_responses.insert(
            stream_id,
            PartialResponse {
                headers: None,
                body,
                written,
            },
        );
    } else {
        DIAG.clean_finish.fetch_add(1, Ordering::Relaxed);
    }
}

fn handle_writable<L>(client: &mut Client<L>, stream_id: u64) {
    let Some(resp) = client.partial_responses.get_mut(&stream_id) else {
        return;
    };
    let http3_conn = match client.http3_conn.as_mut() {
        Some(h) => h,
        None => return,
    };

    if let Some(headers) = resp.headers.take() {
        match http3_conn.send_response(&mut client.conn, stream_id, &headers, false) {
            Ok(()) => {}
            Err(quiche::h3::Error::StreamBlocked) => {
                resp.headers = Some(headers);
                return;
            }
            Err(_) => {
                client.partial_responses.remove(&stream_id);
                return;
            }
        }
    }

    let body = &resp.body[resp.written..];
    let written = match http3_conn.send_body(&mut client.conn, stream_id, body, true) {
        Ok(v) => v,
        Err(quiche::h3::Error::Done) => 0,
        Err(_) => {
            client.partial_responses.remove(&stream_id);
            return;
        }
    };
    resp.written += written;
    if resp.written == resp.body.len() {
        client.partial_responses.remove(&stream_id);
        DIAG.clean_finish.fetch_add(1, Ordering::Relaxed);
    }
}

fn headers_to_request(headers: &[quiche::h3::Header]) -> anyhow::Result<Request<()>> {
    let mut method = Method::GET;
    let mut path = "/".to_string();
    let mut authority = None;
    let mut builder_headers = Vec::new();
    for h in headers {
        let name = std::str::from_utf8(h.name())?;
        let value = std::str::from_utf8(h.value())?.to_string();
        match name {
            ":method" => method = Method::from_bytes(h.value())?,
            ":path" => path = value,
            ":authority" | "host" => authority = Some(value),
            ":scheme" => {}
            _ if name.starts_with(':') => {}
            _ => builder_headers.push((name.to_string(), value)),
        }
    }
    let uri: Uri = if let Some(auth) = authority {
        format!("https://{auth}{path}").parse()?
    } else {
        path.parse()?
    };
    let mut builder = Request::builder().method(method).uri(uri);
    for (n, v) in builder_headers {
        builder = builder.header(n, v);
    }
    Ok(builder.body(())?)
}

fn materialized_to_h3(m: Http3MaterializedResponse) -> (Vec<quiche::h3::Header>, Vec<u8>) {
    let body = m.body.to_vec();
    let mut hdrs = vec![
        quiche::h3::Header::new(b":status", m.status.to_string().as_bytes()),
        quiche::h3::Header::new(b"content-length", body.len().to_string().as_bytes()),
    ];
    for (n, v) in m.headers {
        let lower = n.as_str().to_ascii_lowercase();
        if lower == "content-length" || lower == "transfer-encoding" {
            continue;
        }
        hdrs.push(quiche::h3::Header::new(n.as_bytes(), v.as_bytes()));
    }
    (hdrs, body)
}

fn mint_token(hdr: &quiche::Header, src: &net::SocketAddr) -> Vec<u8> {
    let mut token = Vec::new();
    token.extend_from_slice(b"quiche");
    let addr = match src.ip() {
        std::net::IpAddr::V4(a) => a.octets().to_vec(),
        std::net::IpAddr::V6(a) => a.octets().to_vec(),
    };
    token.extend_from_slice(&addr);
    token.extend_from_slice(&hdr.dcid);
    token
}

fn validate_token<'a>(src: &net::SocketAddr, token: &'a [u8]) -> Option<quiche::ConnectionId<'a>> {
    if token.len() < 6 || &token[..6] != b"quiche" {
        return None;
    }
    let token = &token[6..];
    let addr = match src.ip() {
        std::net::IpAddr::V4(a) => a.octets().to_vec(),
        std::net::IpAddr::V6(a) => a.octets().to_vec(),
    };
    if token.len() < addr.len() || &token[..addr.len()] != addr.as_slice() {
        return None;
    }
    Some(quiche::ConnectionId::from_ref(&token[addr.len()..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_http3_provider_api::Http3ProviderId;
    use std::net::SocketAddr;
    use std::path::PathBuf;

    #[test]
    fn validates_ack_delay_and_diag_provider() {
        let cfg = Http3ProviderConfig {
            provider: Http3ProviderId::Quiche,
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
        let snap = diagnostics_snapshot();
        assert_eq!(snap.provider, "quiche");
    }
}
