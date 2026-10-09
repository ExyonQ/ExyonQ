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

//! SUB-LET-H3-ERROR-RESPONSE-WRITE — targeted regressions (auxiliary).
//!
//! Real loopback QUIC/H3 (no mock transport). Linux-only; product default provider = s2n.
//! GREEN_TESTS_ARE_EVIDENCE_NOT_PROOF — dual-Linux product E2E remains authoritative.

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
use tokio::time::sleep;

static RUSTLS_PROVIDER: Once = Once::new();

fn ensure_rustls_provider() {
    RUSTLS_PROVIDER.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("rustls ring provider");
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
    client_config.transport_config(std::sync::Arc::new(quinn::TransportConfig::default()));
    let mut endpoint = Endpoint::client("127.0.0.1:0".parse().unwrap()).expect("client ep");
    endpoint.set_default_client_config(client_config);
    endpoint
}

async fn spawn_s2n_server(
    listen: SocketAddr,
    tls: &EphemeralTls,
    request_body_drain_cap_bytes: usize,
) -> Arc<LifecycleState> {
    let ops = LifecycleState::new();
    let mut settings = Http3Settings::legacy(
        listen,
        TlsSettings {
            cert_path: tls.cert.clone(),
            key_path: tls.key.clone(),
        },
    );
    settings.request_body_drain_cap_bytes = request_body_drain_cap_bytes;
    let dispatch = Arc::new(StaticDispatch);
    let lifecycle = Arc::new(CoreHttp3Lifecycle::new(Arc::clone(&ops)));
    tokio::spawn(exyonq_mod_http3::serve(settings, dispatch, lifecycle));
    sleep(Duration::from_millis(500)).await;
    ops
}

/// Successful error-response emission: oversize Content-Length → real 413; write must not
/// be counted as response_errors (FALSE_SUCCESS after successful write = forbidden, but
/// successful write must remain success).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s2n_oversize_content_length_emits_413_without_response_error() {
    ensure_rustls_provider();
    let tls = ephemeral_tls();
    let listen = ephemeral_udp_addr();
    // Small drain cap so Content-Length TooLarge trips quickly (still real H3 path).
    let _ops = spawn_s2n_server(listen, &tls, 4096).await;

    let before = exyonq_mod_http3::diagnostics_snapshot().response_errors;

    let client_ep = quinn_client_endpoint(&tls.cert);
    let quic_conn = client_ep
        .connect(listen, "localhost")
        .expect("connect")
        .await
        .expect("handshake");
    let (mut _h3_conn, mut send) = h3::client::new(H3QuinnConnection::new(quic_conn))
        .await
        .expect("h3 client");

    let cl = "8192"; // > drain cap 4096 → TooLarge before body
    let req = Request::builder()
        .method("POST")
        .uri("https://localhost/api/oversize")
        .header("content-length", cl)
        .body(())
        .unwrap();
    let mut stream = send.send_request(req).await.expect("send");
    stream.finish().await.expect("finish");
    let response = stream.recv_response().await.expect("response");
    assert_eq!(
        response.status().as_u16(),
        413,
        "oversize CL must yield real 413 error response"
    );
    while stream.recv_data().await.expect("recv_data").is_some() {}
    let _ = stream.recv_trailers().await;

    let after = exyonq_mod_http3::diagnostics_snapshot().response_errors;
    assert_eq!(
        after, before,
        "successful 413 write must not increment response_errors"
    );
}

/// Required error-response write failure must be observed (not silent success):
/// stream past body cap (TooLarge), then abort the real QUIC connection while the
/// server is still draining / writing the 413 so write_h3_response returns Err.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s2n_error_response_write_failure_is_observed() {
    ensure_rustls_provider();
    let tls = ephemeral_tls();
    let listen = ephemeral_udp_addr();
    let body_cap = 4096usize;
    let _ops = spawn_s2n_server(listen, &tls, body_cap).await;

    let before = exyonq_mod_http3::diagnostics_snapshot().response_errors;

    let client_ep = quinn_client_endpoint(&tls.cert);
    let quic_conn = client_ep
        .connect(listen, "localhost")
        .expect("connect")
        .await
        .expect("handshake");

    let (mut _h3_conn, mut send) = h3::client::new(H3QuinnConnection::new(quic_conn))
        .await
        .expect("h3 client");

    // No Content-Length: stream body past cap so TooLarge trips mid-recv, then cancel
    // the response direction so the server's error-response write fails for real.
    let req = Request::builder()
        .method("POST")
        .uri("https://localhost/api/oversize-abort")
        .body(())
        .unwrap();
    let mut stream = send.send_request(req).await.expect("send");
    let oversized = Bytes::from(vec![b'x'; body_cap + 2048]);
    stream
        .send_data(oversized)
        .await
        .expect("send oversized body chunk");
    // Abort the QUIC connection while the server is still on the 413 write.
    // finish()/stop_sending leave an open DATA frame; h3 then panics in poll_next
    // and the write error is never counted.
    quic_conn.close(quinn::VarInt::from_u32(0), b"abort");

    let mut observed = before;
    for _ in 0..200 {
        sleep(Duration::from_millis(50)).await;
        observed = exyonq_mod_http3::diagnostics_snapshot().response_errors;
        if observed > before {
            break;
        }
    }
    assert!(
        observed > before,
        "error-response write failure must increment response_errors (before={before} after={observed}); silent success forbidden"
    );
}
