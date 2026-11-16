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
//! NG-20 real QUIC over the UDP stream relay.
//!
//! A local `quinn` server and a local `quinn` client complete a QUIC/TLS
//! handshake and exchange application bytes through `[[stream]] protocol = "udp"`.
//! Two backends and two client endpoints prove that two QUIC connections from
//! the same UDP source stay on the selected backend.

use exyonq_config_ir::AppConfig;
use exyonq_core::lifecycle::LifecycleState;
use exyonq_core::stream_proxy::start_streams;
use exyonq_mod_tls::{load_rustls_config, TlsSettings};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

const ALPN: &[u8] = b"ng20-quic";
const IDLE_TIMEOUT_SECS: u64 = 10;

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

fn ephemeral_tls() -> (EphemeralTls, CertificateDer<'static>) {
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
    let tls = EphemeralTls {
        cert: cert.expect("CERT="),
        key: key.expect("KEY="),
        dir: dir.expect("DIR="),
    };
    let pem = std::fs::read(&tls.cert).expect("read test certificate");
    let der = CertificateDer::from_pem_slice(&pem).expect("parse test certificate");
    (tls, der)
}

#[derive(Debug)]
struct TrustTestCert(Vec<u8>);

impl ServerCertVerifier for TrustTestCert {
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
        let algorithms =
            &rustls::crypto::ring::default_provider().signature_verification_algorithms;
        rustls::crypto::verify_tls12_signature(message, cert, dss, algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let algorithms =
            &rustls::crypto::ring::default_provider().signature_verification_algorithms;
        rustls::crypto::verify_tls13_signature(message, cert, dss, algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn server_config(tls: &EphemeralTls) -> ServerConfig {
    let rustls = load_rustls_config(
        &TlsSettings {
            cert_path: tls.cert.clone(),
            key_path: tls.key.clone(),
        },
        &[ALPN],
        None,
    )
    .expect("load QUIC server TLS");
    let mut config = ServerConfig::with_crypto(Arc::new(
        QuicServerConfig::try_from(rustls).expect("quinn server TLS"),
    ));
    let mut transport = TransportConfig::default();
    transport.max_idle_timeout(Some(
        Duration::from_secs(IDLE_TIMEOUT_SECS)
            .try_into()
            .expect("idle timeout"),
    ));
    config.transport = Arc::new(transport);
    config
}

fn client_endpoint(cert: CertificateDer<'static>) -> Endpoint {
    let verifier = Arc::new(TrustTestCert(cert.as_ref().to_vec()));
    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    crypto.alpn_protocols = vec![ALPN.to_vec()];
    let mut config = ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(crypto).expect("quinn client TLS"),
    ));
    let mut transport = TransportConfig::default();
    transport.max_idle_timeout(Some(
        Duration::from_secs(IDLE_TIMEOUT_SECS)
            .try_into()
            .expect("idle timeout"),
    ));
    config.transport_config(Arc::new(transport));
    let mut endpoint = Endpoint::client("127.0.0.1:0".parse().expect("client bind"))
        .expect("QUIC client endpoint");
    endpoint.set_default_client_config(config);
    endpoint
}

fn spawn_quic_backend(label: u8, tls: &EphemeralTls) -> (SocketAddr, JoinHandle<()>) {
    let endpoint = Endpoint::server(server_config(tls), "127.0.0.1:0".parse().expect("bind"))
        .expect("QUIC server endpoint");
    let addr = endpoint.local_addr().expect("QUIC server address");
    let task = tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            tokio::spawn(async move {
                let Ok(conn) = incoming.await else {
                    return;
                };
                while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                    let Ok(data) = recv.read_to_end(4096).await else {
                        break;
                    };
                    if send.write_all(&[label]).await.is_err()
                        || send.write_all(&data).await.is_err()
                    {
                        break;
                    }
                    let _ = send.finish();
                }
            });
        }
    });
    (addr, task)
}

async fn quic_round_trip(endpoint: &Endpoint, relay: SocketAddr, payload: &[u8]) -> Vec<u8> {
    let connect = endpoint
        .connect(relay, "localhost")
        .expect("QUIC connect attempt");
    let conn = tokio::time::timeout(Duration::from_secs(5), connect)
        .await
        .expect("QUIC handshake timeout")
        .expect("QUIC handshake");
    let (mut send, mut recv) = conn.open_bi().await.expect("open QUIC stream");
    send.write_all(payload).await.expect("write QUIC payload");
    send.finish().expect("finish QUIC stream");
    let response = recv.read_to_end(4096).await.expect("read QUIC response");
    conn.close(0u32.into(), b"done");
    response
}

fn config(a: SocketAddr, b: SocketAddr) -> AppConfig {
    format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:18080"
routes = []

[[upstream]]
name = "quic"
timeout_ms = 3000

[[upstream.endpoints]]
id = "quic-a"
address = "127.0.0.1"
port = {}

[[upstream.endpoints]]
id = "quic-b"
address = "127.0.0.1"
port = {}

[[stream]]
name = "quic-udp"
listen = "127.0.0.1:0"
protocol = "udp"
upstream = "quic"
idle_timeout_ms = 10000
max_sessions = 8
"#,
        a.port(),
        b.port()
    )
    .parse()
    .expect("QUIC stream config")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quic_handshake_data_and_udp_affinity_through_relay() {
    exyonq_mod_tls::install_rustls_provider();
    let (tls, cert) = ephemeral_tls();
    let (a, a_task) = spawn_quic_backend(b'A', &tls);
    let (b, b_task) = spawn_quic_backend(b'B', &tls);
    let config = config(a, b);
    let ops = LifecycleState::new();
    let addresses = start_streams(&config.streams, &config.upstreams, Arc::clone(&ops))
        .await
        .expect("UDP relay binds");
    assert_eq!(addresses.len(), 1);
    let relay = addresses[0];

    let first_client = client_endpoint(cert.clone());
    let payload = b"ng20-quic";
    let first = quic_round_trip(&first_client, relay, payload).await;
    let second = quic_round_trip(&first_client, relay, payload).await;
    assert_eq!(first.len(), payload.len() + 1);
    assert_eq!(second.len(), payload.len() + 1);
    assert_eq!(&first[1..], payload);
    assert_eq!(&second[1..], payload);
    assert_eq!(
        first[0], second[0],
        "two QUIC connections from one UDP source must keep the same backend"
    );

    let second_client = client_endpoint(cert);
    let third = quic_round_trip(&second_client, relay, payload).await;
    assert_eq!(&third[1..], payload);
    assert_ne!(
        third[0], first[0],
        "a second UDP source must select the other equal-weight backend"
    );

    ops.request_shutdown();
    a_task.abort();
    b_task.abort();
}
