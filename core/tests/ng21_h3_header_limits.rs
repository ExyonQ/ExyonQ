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
//! NG-21 — HTTP/3 applies the same `[tuning]` header budget and header
//! deadline as HTTP/1.1 and HTTP/2.
//!
//! Real loopback QUIC/H3 through the product facade
//! (`exyonq_mod_http3::serve`, default provider = s2n) with a real
//! `quinn` + `h3-quinn` client. Linux-only, exactly like the existing H3
//! regression harness: the s2n provider is the Linux product default.

#![cfg(target_os = "linux")]

use async_trait::async_trait;
use bytes::Bytes;
use exyonq_core::http3_runtime_registry::CoreHttp3Lifecycle;
use exyonq_core::lifecycle::LifecycleState;
use exyonq_mod_http3::Http3Settings;
use exyonq_mod_tls::TlsSettings;
use exyonq_module_api::http3_runtime::{
    Http3DispatchError, Http3DispatchService, Http3MaterializedResponse,
};
use h3_quinn::Connection as H3QuinnConnection;
use http::Request;
use quinn::{ClientConfig, Endpoint};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Once};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::time::sleep;

static RUSTLS_PROVIDER: Once = Once::new();
static TUNING: Once = Once::new();

/// Header deadline the whole file runs with.
const DEADLINE_MS: u64 = 800;

fn ensure_rustls_provider() {
    RUSTLS_PROVIDER.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("rustls ring provider");
    });
}

fn publish_limits() {
    TUNING.call_once(|| {
        exyonq_module_api::runtime_tuning::publish(Some(DEADLINE_MS), None);
    });
}

struct StaticDispatch;

#[async_trait]
impl Http3DispatchService for StaticDispatch {
    async fn dispatch(
        &self,
        _req: Request<Bytes>,
        _peer_ip: &str,
    ) -> Result<Http3MaterializedResponse, Http3DispatchError> {
        Ok(Http3MaterializedResponse {
            status: 200,
            headers: vec![(
                "content-type".to_string(),
                "text/plain; charset=utf-8".to_string(),
            )],
            body: Bytes::from_static(b"ok"),
        })
    }
}

struct EphemeralTls {
    cert: PathBuf,
    key: PathBuf,
    dir: PathBuf,
}

impl Drop for EphemeralTls {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ephemeral_tls() -> EphemeralTls {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let script = root.join("scripts/test-tls/generate-ephemeral-tls.sh");
    let out = std::process::Command::new("bash")
        .arg(&script)
        .arg("--print-paths")
        .output()
        .expect("run generate-ephemeral-tls.sh");
    assert!(
        out.status.success(),
        "ephemeral TLS generation failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let mut dir = None;
    let mut cert = None;
    let mut key = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("DIR=") {
            dir = Some(PathBuf::from(v));
        } else if let Some(v) = line.strip_prefix("CERT=") {
            cert = Some(PathBuf::from(v));
        } else if let Some(v) = line.strip_prefix("KEY=") {
            key = Some(PathBuf::from(v));
        }
    }
    EphemeralTls {
        cert: cert.expect("CERT="),
        key: key.expect("KEY="),
        dir: dir.expect("DIR="),
    }
}

fn ephemeral_udp_addr() -> SocketAddr {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").expect("udp bind");
    let addr = sock.local_addr().expect("local addr");
    drop(sock);
    addr
}

#[derive(Debug)]
struct TrustFixtureCert(Vec<u8>);

impl TrustFixtureCert {
    fn from_pem_path(path: &PathBuf) -> Self {
        let pem = std::fs::read(path).expect("read fixture cert");
        let der = CertificateDer::from_pem_slice(&pem).expect("fixture cert der");
        Self(der.as_ref().to_vec())
    }
}

impl ServerCertVerifier for TrustFixtureCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.0.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let algs = &rustls::crypto::ring::default_provider().signature_verification_algorithms;
        rustls::crypto::verify_tls12_signature(message, cert, dss, algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let algs = &rustls::crypto::ring::default_provider().signature_verification_algorithms;
        rustls::crypto::verify_tls13_signature(message, cert, dss, algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn quinn_client_endpoint(cert_path: &PathBuf) -> Endpoint {
    let verifier = Arc::new(TrustFixtureCert::from_pem_path(cert_path));
    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    crypto.alpn_protocols = vec![b"h3".to_vec()];
    let mut client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto).expect("quic client"),
    ));
    client_config.transport_config(Arc::new(quinn::TransportConfig::default()));
    let mut endpoint = Endpoint::client("127.0.0.1:0".parse().unwrap()).expect("client ep");
    endpoint.set_default_client_config(client_config);
    endpoint
}

async fn spawn_s2n_server(listen: SocketAddr, tls: &EphemeralTls) -> Arc<LifecycleState> {
    let ops = LifecycleState::new();
    let settings = Http3Settings::legacy(
        listen,
        TlsSettings {
            cert_path: tls.cert.clone(),
            key_path: tls.key.clone(),
        },
    );
    let dispatch = Arc::new(StaticDispatch);
    let lifecycle = Arc::new(CoreHttp3Lifecycle::new(Arc::clone(&ops)));
    tokio::spawn(exyonq_mod_http3::serve(settings, dispatch, lifecycle));
    sleep(Duration::from_millis(500)).await;
    publish_limits();
    ops
}

/// Real H3 request with an optional oversized header field; returns the status
/// the client observes.
async fn h3_request_status(listen: SocketAddr, tls: &EphemeralTls, pad: Option<usize>) -> u16 {
    let endpoint = quinn_client_endpoint(&tls.cert);
    let conn = endpoint
        .connect(listen, "localhost")
        .expect("connect")
        .await
        .expect("handshake");
    let (_h3_conn, mut send) = h3::client::new(H3QuinnConnection::new(conn))
        .await
        .expect("h3 client");

    let mut builder = Request::builder().method("GET").uri("https://localhost/");
    if let Some(pad) = pad {
        builder = builder.header("x-pad", "a".repeat(pad));
    }
    let req = builder.body(()).expect("request");
    let mut stream = send.send_request(req).await.expect("send request");
    stream.finish().await.expect("finish");
    let response = stream.recv_response().await.expect("response");
    response.status().as_u16()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3_header_within_budget_is_served() {
    ensure_rustls_provider();
    let tls = ephemeral_tls();
    let listen = ephemeral_udp_addr();
    let _ops = spawn_s2n_server(listen, &tls).await;

    let status = tokio::time::timeout(
        Duration::from_secs(10),
        h3_request_status(listen, &tls, Some(4_000)),
    )
    .await
    .expect("within-budget request completed");
    assert_eq!(status, 200, "within budget must be served");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3_header_over_budget_is_rejected() {
    ensure_rustls_provider();
    let tls = ephemeral_tls();
    let listen = ephemeral_udp_addr();
    let _ops = spawn_s2n_server(listen, &tls).await;

    let status = tokio::time::timeout(
        Duration::from_secs(10),
        h3_request_status(listen, &tls, Some(12_000)),
    )
    .await
    .expect("over-budget request completed");
    assert_eq!(
        status, 431,
        "an over-budget header block must be refused, never served"
    );

    // The connection and the server keep working: the refusal is per request.
    let status = tokio::time::timeout(
        Duration::from_secs(10),
        h3_request_status(listen, &tls, Some(64)),
    )
    .await
    .expect("request after rejection completed");
    assert_eq!(status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3_header_deadline_cuts_stream_without_headers() {
    ensure_rustls_provider();
    let tls = ephemeral_tls();
    let listen = ephemeral_udp_addr();
    let _ops = spawn_s2n_server(listen, &tls).await;

    let endpoint = quinn_client_endpoint(&tls.cert);
    let conn = endpoint
        .connect(listen, "localhost")
        .expect("connect")
        .await
        .expect("handshake");

    // A request stream that announces a HEADERS block (type 0x01, length varint
    // 0x0500) and never sends it: the server cannot resolve the request head.
    // Both halves stay open, so nothing but the server's header deadline can
    // end the connection. The declared length stays well below the maximum
    // field-section size, or QPACK would refuse the frame on its own.
    let (mut send, recv) = conn.open_bi().await.expect("open_bi");
    send.write_all(&[0x01, 0x05, 0x00])
        .await
        .expect("partial HEADERS frame");
    send.flush().await.expect("flush partial frame");

    let started = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(5), conn.closed())
        .await
        .expect("connection must be cut by the header deadline");
    drop(recv);
    drop(send);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cut took {:?}",
        started.elapsed()
    );
}
