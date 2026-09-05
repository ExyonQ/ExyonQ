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
//! WebSocket reverse proxy (HTTP/1.1 Upgrade tunnel).

use crate::hyper_client::{get_empty_body_client, ProxyClient};
use crate::hyper_forward::{bad_gateway, bad_request, ProxyHyperMetrics};
use crate::request_headers_safe_for_proxy;
use crate::upstream_target::UpstreamTarget;
use http_body_util::{combinators::BoxBody, BodyExt, Empty};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, HOST, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::warn;

const WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

fn sec_websocket_accept(key: &str) -> String {
    use base64::Engine;
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_GUID);
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

pub fn is_websocket_upgrade(headers: &hyper::HeaderMap) -> bool {
    let connection = headers
        .get(hyper::header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let upgrade = headers
        .get(hyper::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    connection.to_ascii_lowercase().contains("upgrade") && upgrade.eq_ignore_ascii_case("websocket")
}

pub async fn forward_websocket(
    _client: &ProxyClient,
    upstream: &UpstreamTarget,
    mut req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
    metrics: &ProxyHyperMetrics,
) -> Response<BoxBody<bytes::Bytes, hyper::Error>> {
    if !request_headers_safe_for_proxy(req.headers()) {
        return bad_request("ambiguous request headers");
    }

    // Hyper requires registering the client upgrade before returning 101 from the service.
    let on_client_upgrade = hyper::upgrade::on(&mut req);

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();

    let Some(client_key) = req
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
    else {
        return bad_gateway();
    };

    let mut upstream_builder = Request::builder()
        .method(req.method())
        .uri(upstream.uri_for(&path_and_query));
    // DP-H-WS-01: Skip intermediary identity headers to prevent spoofing.
    // Re-inject trusted x-forwarded-for below.
    for (name, value) in req.headers() {
        let name_lower = name.as_str().to_ascii_lowercase();
        if name_lower == "x-forwarded-for"
            || name_lower == "x-forwarded-proto"
            || name_lower == "x-forwarded-host"
            || name_lower == "forwarded"
            || name_lower == "x-real-ip"
        {
            continue;
        }
        upstream_builder = upstream_builder.header(name, value);
    }
    // Re-inject trusted x-forwarded-for from the proxy (peer IP).
    if let Some(value) = x_forwarded_for {
        upstream_builder = upstream_builder.header("x-forwarded-for", value);
    }
    if let Some(host) = &upstream.host {
        upstream_builder = upstream_builder.header(HOST, host);
    }

    let upstream_request = get_empty_body_client().request(
        upstream_builder
            .body(Empty::<bytes::Bytes>::new())
            .expect("websocket upstream request"),
    );
    let upstream_response = match tokio::time::timeout(WS_HANDSHAKE_TIMEOUT, upstream_request).await
    {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            warn!(%err, "websocket upstream error");
            metrics.responses_502.fetch_add(1, Ordering::Relaxed);
            return bad_gateway();
        }
        Err(_) => {
            warn!("websocket handshake timeout");
            metrics.responses_504.fetch_add(1, Ordering::Relaxed);
            return bad_gateway();
        }
    };

    if upstream_response.status() != StatusCode::SWITCHING_PROTOCOLS {
        let (parts, body) = upstream_response.into_parts();
        return Response::from_parts(parts, body.boxed());
    }

    let accept = sec_websocket_accept(&client_key);
    let (upstream_parts, upstream_body) = upstream_response.into_parts();
    drop(upstream_body);
    let mut upstream_response = Response::from_parts(upstream_parts, Empty::<bytes::Bytes>::new());
    let on_upstream_upgrade = hyper::upgrade::on(&mut upstream_response);

    tokio::spawn(async move {
        let (client_res, upstream_res) = tokio::join!(on_client_upgrade, on_upstream_upgrade);
        match (client_res, upstream_res) {
            (Ok(client_io), Ok(upstream_io)) => {
                if let Err(err) = tunnel(TokioIo::new(client_io), TokioIo::new(upstream_io)).await {
                    warn!(%err, "websocket tunnel closed");
                }
            }
            (client_res, upstream_res) => {
                warn!(
                    client_ok = client_res.is_ok(),
                    upstream_ok = upstream_res.is_ok(),
                    "websocket upgrade failed"
                );
            }
        }
    });

    let mut res = Response::new(
        Empty::<bytes::Bytes>::new()
            .map_err(|never| match never {})
            .boxed(),
    );
    *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    res.headers_mut()
        .insert(UPGRADE, HeaderValue::from_static("websocket"));
    res.headers_mut().insert(
        hyper::header::CONNECTION,
        HeaderValue::from_static("upgrade"),
    );
    res.headers_mut().insert(
        SEC_WEBSOCKET_ACCEPT,
        HeaderValue::from_str(&accept).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    res
}

async fn tunnel<C, U>(client: C, upstream: U) -> std::io::Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    U: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut client_read, mut client_write) = tokio::io::split(client);
    let (mut upstream_read, mut upstream_write) = tokio::io::split(upstream);

    let client_to_upstream = tokio::io::copy(&mut client_read, &mut upstream_write);
    let upstream_to_client = tokio::io::copy(&mut upstream_read, &mut client_write);

    tokio::select! {
        result = client_to_upstream => {
            result?;
        }
        result = upstream_to_client => {
            result?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyper_forward::ProxyHyperMetrics;
    use crate::upstream_target::UpstreamTarget;
    use crate::UpstreamDescriptor;
    use hyper::body::Incoming;
    use hyper::service::service_fn;
    use hyper_util::server::conn::auto::Builder;
    use std::convert::Infallible;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn ws_accept(key: &str) -> String {
        sec_websocket_accept(key)
    }

    fn ws_frame_text(text: &str, mask: bool) -> Vec<u8> {
        let payload = text.as_bytes();
        let mut frame = Vec::new();
        frame.push(0x81);
        if payload.len() < 126 {
            frame.push(if mask {
                0x80 | payload.len() as u8
            } else {
                payload.len() as u8
            });
        } else {
            frame.push(if mask { 0x80 | 126 } else { 126 });
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        if mask {
            let mask_key = [0x12, 0x34, 0x56, 0x78];
            frame.extend_from_slice(&mask_key);
            frame.extend(
                payload
                    .iter()
                    .enumerate()
                    .map(|(i, b)| b ^ mask_key[i % 4])
                    .collect::<Vec<_>>(),
            );
        } else {
            frame.extend_from_slice(payload);
        }
        frame
    }

    fn parse_ws_text(payload: &[u8]) -> Option<String> {
        if payload.len() < 2 {
            return None;
        }
        let masked = (payload[1] & 0x80) != 0;
        let mut idx = 2usize;
        let mut length = (payload[1] & 0x7f) as usize;
        if length == 126 {
            if payload.len() < 4 {
                return None;
            }
            length = u16::from_be_bytes([payload[2], payload[3]]) as usize;
            idx = 4;
        }
        let mask = if masked {
            if payload.len() < idx + 4 {
                return None;
            }
            let m = &payload[idx..idx + 4];
            idx += 4;
            Some(m)
        } else {
            None
        };
        if payload.len() < idx + length {
            return None;
        }
        let data = &payload[idx..idx + length];
        let unmasked = if let Some(m) = mask {
            data.iter()
                .enumerate()
                .map(|(i, b)| b ^ m[i % 4])
                .collect::<Vec<_>>()
        } else {
            data.to_vec()
        };
        String::from_utf8(unmasked).ok()
    }

    async fn spawn_ws_echo_upstream(coalesce_frame: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    continue;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = match stream.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };
                    let text = String::from_utf8_lossy(&buf[..n]);
                    if !text.to_ascii_lowercase().contains("upgrade: websocket") {
                        return;
                    }
                    let key = text
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.trim()
                                .eq_ignore_ascii_case("sec-websocket-key")
                                .then_some(v.trim().to_string())
                        })
                        .unwrap_or_default();
                    let accept = ws_accept(&key);
                    let resp = format!(
                        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                    );
                    if coalesce_frame {
                        let mut out = resp.into_bytes();
                        out.extend(ws_frame_text("ignored", false));
                        let _ = stream.write_all(&out).await;
                    } else {
                        let _ = stream.write_all(resp.as_bytes()).await;
                    }
                    loop {
                        let n = match stream.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        if let Some(msg) = parse_ws_text(&buf[..n]) {
                            let _ = stream.write_all(&ws_frame_text(&msg, false)).await;
                        }
                    }
                });
            }
        });
        port
    }

    async fn proxy_ws_echo(payload: &str, coalesce: bool) -> String {
        let upstream_port = spawn_ws_echo_upstream(coalesce).await;

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = proxy_listener.local_addr().unwrap().port();
        let desc = UpstreamDescriptor {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: format!("http://127.0.0.1:{upstream_port}"),
            timeout: Duration::from_millis(5000),
            host: Some("127.0.0.1".into()),
        };
        let upstream = UpstreamTarget::from_descriptor(&desc).unwrap();
        let metrics = Arc::new(ProxyHyperMetrics::default());
        let client = crate::hyper_client::build_incoming_client();

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = proxy_listener.accept().await else {
                    continue;
                };
                let upstream = upstream.clone();
                let metrics = Arc::clone(&metrics);
                let client = client.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let upstream = upstream.clone();
                        let metrics = Arc::clone(&metrics);
                        let client = client.clone();
                        async move {
                            Ok::<_, Infallible>(
                                forward_websocket(&client, &upstream, req, None, &metrics).await,
                            )
                        }
                    });
                    let _ = Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection_with_upgrades(io, svc)
                        .await;
                });
            }
        });

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", proxy_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let req = format!(
            "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{proxy_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        assert!(
            String::from_utf8_lossy(&buf[..n]).contains("101"),
            "expected 101, got {}",
            String::from_utf8_lossy(&buf[..n])
        );
        stream
            .write_all(&ws_frame_text(payload, true))
            .await
            .unwrap();
        let n = stream.read(&mut buf).await.unwrap();
        parse_ws_text(&buf[..n]).expect("ws echo frame")
    }

    #[test]
    fn sec_websocket_accept_rfc_example() {
        assert_eq!(
            sec_websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[tokio::test]
    async fn echo_1_byte() {
        let echo = proxy_ws_echo("a", false).await;
        assert_eq!(echo, "a");
    }

    #[tokio::test]
    async fn echo_125_bytes() {
        let payload = "x".repeat(125);
        let echo = proxy_ws_echo(&payload, false).await;
        assert_eq!(echo, payload);
    }

    #[tokio::test]
    async fn echo_126_bytes() {
        let payload = "y".repeat(126);
        let echo = proxy_ws_echo(&payload, false).await;
        assert_eq!(echo, payload);
    }

    #[tokio::test]
    async fn echo_127_bytes() {
        let payload = "z".repeat(127);
        let echo = proxy_ws_echo(&payload, false).await;
        assert_eq!(echo, payload);
    }

    #[tokio::test]
    async fn echo_binary_null_and_ff() {
        let payload = String::from_utf8_lossy(&[0x00, 0xff, 0x00, 0xab]).into_owned();
        let echo = proxy_ws_echo(&payload, false).await;
        assert_eq!(echo, payload);
    }

    #[tokio::test]
    async fn echo_multiple_messages() {
        let upstream_port = spawn_ws_echo_upstream(false).await;

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = proxy_listener.local_addr().unwrap().port();
        let desc = UpstreamDescriptor {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: format!("http://127.0.0.1:{upstream_port}"),
            timeout: Duration::from_millis(5000),
            host: Some("127.0.0.1".into()),
        };
        let upstream = UpstreamTarget::from_descriptor(&desc).unwrap();
        let metrics = Arc::new(ProxyHyperMetrics::default());
        let client = crate::hyper_client::build_incoming_client();

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = proxy_listener.accept().await else {
                    continue;
                };
                let upstream = upstream.clone();
                let metrics = Arc::clone(&metrics);
                let client = client.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let upstream = upstream.clone();
                        let metrics = Arc::clone(&metrics);
                        let client = client.clone();
                        async move {
                            Ok::<_, Infallible>(
                                forward_websocket(&client, &upstream, req, None, &metrics).await,
                            )
                        }
                    });
                    let _ = Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection_with_upgrades(io, svc)
                        .await;
                });
            }
        });

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", proxy_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let req = format!(
            "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{proxy_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();
        for msg in ["one", "two", "three"] {
            stream.write_all(&ws_frame_text(msg, true)).await.unwrap();
            let n = stream.read(&mut buf).await.unwrap();
            assert_eq!(parse_ws_text(&buf[..n]).as_deref(), Some(msg));
        }
    }

    #[tokio::test]
    async fn echo_client_frame_coalesced_with_request() {
        let upstream_port = spawn_ws_echo_upstream(false).await;

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = proxy_listener.local_addr().unwrap().port();
        let desc = UpstreamDescriptor {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: format!("http://127.0.0.1:{upstream_port}"),
            timeout: Duration::from_millis(5000),
            host: Some("127.0.0.1".into()),
        };
        let upstream = UpstreamTarget::from_descriptor(&desc).unwrap();
        let metrics = Arc::new(ProxyHyperMetrics::default());
        let client = crate::hyper_client::build_incoming_client();

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = proxy_listener.accept().await else {
                    continue;
                };
                let upstream = upstream.clone();
                let metrics = Arc::clone(&metrics);
                let client = client.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let upstream = upstream.clone();
                        let metrics = Arc::clone(&metrics);
                        let client = client.clone();
                        async move {
                            Ok::<_, Infallible>(
                                forward_websocket(&client, &upstream, req, None, &metrics).await,
                            )
                        }
                    });
                    let _ = Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection_with_upgrades(io, svc)
                        .await;
                });
            }
        });

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", proxy_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let mut req = format!(
            "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{proxy_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .into_bytes();
        req.extend(ws_frame_text("ping-kd34", true));
        stream.write_all(&req).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).contains("101"));
        let n = stream.read(&mut buf).await.unwrap();
        assert_eq!(parse_ws_text(&buf[..n]).as_deref(), Some("ping-kd34"));
    }

    #[tokio::test]
    async fn echo_1_kib() {
        let payload = "k".repeat(1024);
        let echo = proxy_ws_echo(&payload, false).await;
        assert_eq!(echo, payload);
    }
}
