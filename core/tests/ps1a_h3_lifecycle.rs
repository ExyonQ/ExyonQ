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

//! PS1A-H3: HTTP/3 QUIC connection lifecycle integration tests.

use exyonq_core::http3_runtime_registry::CoreHttp3Lifecycle;
use exyonq_core::lifecycle::LifecycleState;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_module_api::http3_runtime::Http3ConnectionLifecycle;
use http::{Request, StatusCode};
use std::sync::Arc;

#[test]
fn h3_lease_cannot_clone_compile_time() {
    fn assert_not_clone<T>() {}
    assert_not_clone::<exyonq_core::http3_runtime_registry::Http3ConnectionLease>();
}

#[test]
fn h3_drain_blocks_new_admission_without_increment() {
    let ops = LifecycleState::new();
    let lifecycle = CoreHttp3Lifecycle::new(Arc::clone(&ops));
    let lease = lifecycle.try_enter_connection().unwrap();
    assert_eq!(ops.active_connections(), 1);
    ops.start_drain();
    assert!(!ops.drain_complete());
    assert!(lifecycle.try_enter_connection().is_err());
    assert_eq!(ops.active_connections(), 1);
    drop(lease);
    assert!(ops.drain_complete());
}

#[tokio::test]
async fn h3_request_dispatch_does_not_increment_lifecycle() {
    let ops = LifecycleState::new();
    let raw = include_str!("../../tests/fixtures/minimal.toml");
    let config: exyonq_core::config::AppConfig = raw.parse().expect("config");
    let proxy = exyonq_mod_proxy::build_incoming_client();
    let state = exyonq_core::server::state::ServerState::new(config, proxy.clone())
        .await
        .expect("state");
    let shared = exyonq_core::reload::wrap_state(state);
    let ctx = ConnectionContext {
        state: exyonq_core::reload::read_state(&shared),
        proxy_client: proxy,
        x_forwarded_for: http::HeaderValue::from_static("127.0.0.1"),
        ops: Arc::clone(&ops),
    };
    assert_eq!(ops.active_connections(), 0);
    let req = Request::get("/health").body(()).unwrap();
    let resp = serve_http3_request(ctx, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(ops.active_connections(), 0);

    ops.start_drain();
    let ctx2 = ConnectionContext {
        state: exyonq_core::reload::read_state(&shared),
        proxy_client: exyonq_mod_proxy::build_incoming_client(),
        x_forwarded_for: http::HeaderValue::from_static("127.0.0.1"),
        ops: Arc::clone(&ops),
    };
    let req2 = Request::get("/health").body(()).unwrap();
    let resp2 = serve_http3_request(ctx2, req2).await;
    assert_eq!(resp2.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(ops.active_connections(), 0);
}

#[cfg(target_os = "linux")]
mod native {
    use super::*;
    use async_trait::async_trait;
    use bytes::Bytes;
    use exyonq_core::http3_runtime_registry::CoreHttp3Dispatcher;
    use exyonq_mod_http3::Http3Settings;
    use exyonq_mod_tls::TlsSettings;
    use exyonq_module_api::http3_runtime::{
        Http3DispatchError, Http3DispatchService, Http3MaterializedResponse,
    };
    use h3_quinn::Connection as H3QuinnConnection;
    use http::Request;
    use quinn::{ClientConfig, Endpoint, VarInt};
    use rustls_pemfile::certs;
    use std::io::BufReader;
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::time::sleep;

    use std::sync::Once;

    static RUSTLS_PROVIDER: Once = Once::new();

    fn ensure_rustls_provider() {
        RUSTLS_PROVIDER.call_once(|| {
            rustls::crypto::ring::default_provider()
                .install_default()
                .expect("rustls ring provider");
        });
    }

    struct StaticDispatch {
        status: u16,
        body: &'static [u8],
    }

    #[async_trait]
    impl Http3DispatchService for StaticDispatch {
        async fn dispatch(
            &self,
            _req: Request<()>,
            _peer_ip: &str,
        ) -> Result<Http3MaterializedResponse, Http3DispatchError> {
            Ok(Http3MaterializedResponse {
                status: self.status,
                headers: vec![(
                    "content-type".to_string(),
                    "text/plain; charset=utf-8".to_string(),
                )],
                body: Bytes::from_static(self.body),
            })
        }
    }

    fn tls_fixture_paths() -> (PathBuf, PathBuf) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let cert = root.join("tests/fixtures/tls/cert.pem");
        let key = root.join("tests/fixtures/tls/key.pem");
        (cert, key)
    }

    fn ephemeral_udp_addr() -> SocketAddr {
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").expect("udp bind");
        let addr = sock.local_addr().expect("local addr");
        drop(sock);
        addr
    }

    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::DigitallySignedStruct;
    use rustls::SignatureScheme;

    #[derive(Debug)]
    struct TrustFixtureCert(Vec<u8>);

    impl TrustFixtureCert {
        fn from_pem_path(path: &PathBuf) -> Self {
            let pem = std::fs::read(path).expect("read fixture cert");
            let der = certs(&mut BufReader::new(pem.as_slice()))
                .next()
                .expect("fixture cert present")
                .expect("fixture cert der");
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

    async fn h3_get_status(
        send: &mut h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
        path: &str,
    ) -> u16 {
        let req = Request::builder()
            .method("GET")
            .uri(format!("https://localhost{path}"))
            .body(())
            .unwrap();
        let mut stream = send.send_request(req).await.expect("send");
        stream.finish().await.expect("finish");
        let response = stream.recv_response().await.expect("response");
        while stream.recv_data().await.expect("recv_data").is_some() {}
        // P13F: response stream must close cleanly (no post-body reset / stream error).
        let trailers = stream.recv_trailers().await.expect("trailers_or_clean_fin");
        assert!(trailers.is_none(), "unexpected trailers on static GET");
        response.status().as_u16()
    }

    async fn wait_for_active(ops: &LifecycleState, expected: u64) {
        for _ in 0..100 {
            if ops.active_connections() == expected {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            ops.active_connections(),
            expected,
            "timed out waiting for active_connections={expected}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_quic_connection_single_enter_multi_request() {
        ensure_rustls_provider();
        let ops = LifecycleState::new();
        let (cert, key) = tls_fixture_paths();
        assert!(cert.is_file() && key.is_file(), "tls fixtures missing");

        let listen = ephemeral_udp_addr();
        let settings = Http3Settings::legacy(
            listen,
            TlsSettings {
                cert_path: cert.clone(),
                key_path: key,
            },
        );
        let dispatch = Arc::new(StaticDispatch {
            status: 200,
            body: b"ok",
        });
        let lifecycle = Arc::new(CoreHttp3Lifecycle::new(Arc::clone(&ops)));
        tokio::spawn(exyonq_mod_http3::serve(settings, dispatch, lifecycle));

        sleep(Duration::from_millis(500)).await;

        let client_ep = quinn_client_endpoint(&cert);
        let quic_conn = client_ep
            .connect(listen, "localhost")
            .expect("connect")
            .await
            .expect("handshake");
        wait_for_active(&ops, 1).await;

        let (mut _h3_conn, mut send) = h3::client::new(H3QuinnConnection::new(quic_conn))
            .await
            .expect("h3 client");

        let s1 = h3_get_status(&mut send, "/a").await;
        assert_eq!(s1, 200);
        assert_eq!(ops.active_connections(), 1, "request must not increment");

        let s2 = h3_get_status(&mut send, "/b").await;
        assert_eq!(s2, 200);
        assert_eq!(ops.active_connections(), 1, "second request same count");

        drop(send);
        let _ = _h3_conn.shutdown(0).await;
        sleep(Duration::from_millis(300)).await;
        assert_eq!(ops.active_connections(), 0, "connection close drops once");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_drain_rejects_second_connection() {
        ensure_rustls_provider();
        let ops = LifecycleState::new();
        let (cert, key) = tls_fixture_paths();
        let listen = ephemeral_udp_addr();
        let settings = Http3Settings::legacy(
            listen,
            TlsSettings {
                cert_path: cert.clone(),
                key_path: key,
            },
        );
        let dispatch = Arc::new(StaticDispatch {
            status: 200,
            body: b"ok",
        });
        let lifecycle = Arc::new(CoreHttp3Lifecycle::new(Arc::clone(&ops)));
        tokio::spawn(exyonq_mod_http3::serve(settings, dispatch, lifecycle));

        sleep(Duration::from_millis(500)).await;

        let client_ep = quinn_client_endpoint(&cert);
        let first = client_ep
            .connect(listen, "localhost")
            .expect("connect")
            .await
            .expect("first handshake");
        wait_for_active(&ops, 1).await;

        ops.start_drain();
        assert!(!ops.drain_complete());

        let second = client_ep.connect(listen, "localhost");
        if let Ok(connecting) = second {
            let result = tokio::time::timeout(Duration::from_secs(3), connecting).await;
            match result {
                Ok(Ok(conn)) => {
                    drop(conn);
                    assert_eq!(
                        ops.active_connections(),
                        1,
                        "drain must not admit second conn"
                    );
                }
                Ok(Err(_)) | Err(_) => {}
            }
        }
        sleep(Duration::from_millis(100)).await;
        assert_eq!(ops.active_connections(), 1, "first connection still active");

        first.close(VarInt::from_u32(0), b"test");
        wait_for_active(&ops, 0).await;
        assert!(ops.drain_complete());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_core_dispatcher_zero_enter_per_request() {
        ensure_rustls_provider();
        let ops = LifecycleState::new();
        let raw = include_str!("../../tests/fixtures/minimal.toml");
        let config: exyonq_core::config::AppConfig = raw.parse().expect("config");
        let proxy = exyonq_mod_proxy::build_incoming_client();
        let state = exyonq_core::server::state::ServerState::new(config, proxy.clone())
            .await
            .expect("state");
        let shared = exyonq_core::reload::wrap_state(state);
        let dispatch = Arc::new(CoreHttp3Dispatcher::new(shared, proxy, Arc::clone(&ops)));

        let (cert, key) = tls_fixture_paths();
        let listen = ephemeral_udp_addr();
        let settings = Http3Settings::legacy(
            listen,
            TlsSettings {
                cert_path: cert.clone(),
                key_path: key,
            },
        );
        let lifecycle = Arc::new(CoreHttp3Lifecycle::new(Arc::clone(&ops)));
        tokio::spawn(exyonq_mod_http3::serve(settings, dispatch, lifecycle));

        sleep(Duration::from_millis(500)).await;

        let client_ep = quinn_client_endpoint(&cert);
        let quic_conn = client_ep
            .connect(listen, "localhost")
            .expect("connect")
            .await
            .expect("handshake");
        wait_for_active(&ops, 1).await;

        let (mut _h3_conn, mut send) = h3::client::new(H3QuinnConnection::new(quic_conn))
            .await
            .expect("h3");
        let _ = h3_get_status(&mut send, "/health").await;
        assert_eq!(ops.active_connections(), 1);
        drop(send);
        let _ = _h3_conn.shutdown(0).await;
        sleep(Duration::from_millis(300)).await;
        assert_eq!(ops.active_connections(), 0);
    }
}
