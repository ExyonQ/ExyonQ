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
//! Shared Hyper client configuration — dual typed pools (KD3.2).

use http_body_util::{Empty, Full};
use hyper::body::Incoming;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

/// Single authority for Hyper legacy client settings (both body generics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HyperClientConfig {
    pub tcp_nodelay: bool,
    pub tcp_keepalive: Duration,
    pub connect_timeout: Duration,
    pub pool_idle_timeout: Duration,
    pub pool_max_idle_per_host: usize,
}

impl Default for HyperClientConfig {
    fn default() -> Self {
        Self {
            tcp_nodelay: true,
            tcp_keepalive: Duration::from_secs(60),
            connect_timeout: Duration::from_millis(500),
            pool_idle_timeout: Duration::from_secs(90),
            pool_max_idle_per_host: 256,
        }
    }
}

static CONFIG: RwLock<HyperClientConfig> = RwLock::new(HyperClientConfig {
    tcp_nodelay: true,
    tcp_keepalive: Duration::from_secs(60),
    connect_timeout: Duration::from_millis(500),
    pool_idle_timeout: Duration::from_secs(90),
    pool_max_idle_per_host: 256,
});

/// Read the active shared configuration (used by both client factories).
pub fn hyper_client_config() -> HyperClientConfig {
    *CONFIG.read().expect("hyper client config poisoned")
}

/// Replace configuration for both client pools (KD3.2 — single authority).
pub fn set_hyper_client_config(config: HyperClientConfig) {
    *CONFIG.write().expect("hyper client config poisoned") = config;
}

pub type ProxyClient = Client<HttpConnector, Incoming>;
type GetClient = Client<HttpConnector, Empty<bytes::Bytes>>;
type PostClient = Client<HttpConnector, Full<bytes::Bytes>>;

static INCOMING_CLIENT: OnceLock<ProxyClient> = OnceLock::new();
static POST_CLIENT: OnceLock<PostClient> = OnceLock::new();

pub fn build_incoming_client() -> ProxyClient {
    INCOMING_CLIENT.get_or_init(build_hyper_client).clone()
}

pub fn post_body_client() -> PostClient {
    POST_CLIENT.get_or_init(build_hyper_client).clone()
}

fn build_hyper_client<B>() -> Client<HttpConnector, B>
where
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let cfg = hyper_client_config();
    let mut connector = HttpConnector::new();
    connector.set_nodelay(cfg.tcp_nodelay);
    connector.set_keepalive(Some(cfg.tcp_keepalive));
    connector.set_connect_timeout(Some(cfg.connect_timeout));
    Client::builder(TokioExecutor::new())
        .pool_idle_timeout(cfg.pool_idle_timeout)
        .pool_max_idle_per_host(cfg.pool_max_idle_per_host)
        .build(connector)
}

static GET_CLIENT: OnceLock<GetClient> = OnceLock::new();

pub fn get_empty_body_client() -> GetClient {
    GET_CLIENT.get_or_init(build_hyper_client).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dual_clients_share_equivalent_config() {
        let cfg = hyper_client_config();
        let _incoming = build_incoming_client();
        let _empty = get_empty_body_client();
        assert!(cfg.tcp_nodelay);
        assert_eq!(cfg.pool_max_idle_per_host, 256);
    }

    #[test]
    fn incoming_pool_initialized_once() {
        let _a = build_incoming_client();
        assert!(INCOMING_CLIENT.get().is_some());
        let _b = build_incoming_client();
        assert!(INCOMING_CLIENT.get().is_some());
    }

    #[test]
    fn pools_are_separate_instances() {
        use std::any::TypeId;

        let _incoming = build_incoming_client();
        let _empty = get_empty_body_client();
        let _post = post_body_client();

        assert_ne!(TypeId::of::<ProxyClient>(), TypeId::of::<GetClient>());
        assert_ne!(TypeId::of::<ProxyClient>(), TypeId::of::<PostClient>());
        assert_ne!(TypeId::of::<GetClient>(), TypeId::of::<PostClient>());

        // Lazy-init slots: repeated clones do not create new pools.
        let _empty_again = get_empty_body_client();
        let _post_again = post_body_client();
    }

    #[tokio::test]
    async fn head_request_reaches_upstream() {
        use bytes::Bytes;
        use http_body_util::Full;
        use hyper::body::Incoming;
        use hyper::service::service_fn;
        use hyper_util::rt::TokioIo;
        use hyper_util::server::conn::auto::Builder;
        use std::convert::Infallible;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(|req: hyper::Request<Incoming>| async move {
                        assert_eq!(req.method(), hyper::Method::HEAD);
                        Ok::<_, Infallible>(
                            hyper::Response::builder()
                                .status(200)
                                .header("content-length", "0")
                                .body(Full::new(Bytes::new()))
                                .unwrap(),
                        )
                    });
                    let _ = Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection(io, svc)
                        .await;
                });
            }
        });

        let uri: hyper::Uri = format!("http://{addr}/head").parse().unwrap();
        let req = hyper::Request::builder()
            .method(hyper::Method::HEAD)
            .uri(uri)
            .body(Empty::<Bytes>::new())
            .unwrap();
        let client = get_empty_body_client();
        let resp = client.request(req).await.expect("head request");
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn head_forward_via_upstream_target() {
        use crate::hyper_forward::{forward_head, ProxyHyperMetrics};
        use crate::upstream_target::UpstreamTarget;
        use crate::UpstreamDescriptor;
        use bytes::Bytes;
        use http_body_util::Full;
        use hyper::body::Incoming;
        use hyper::service::service_fn;
        use hyper_util::rt::TokioIo;
        use hyper_util::server::conn::auto::Builder;
        use std::convert::Infallible;
        use std::sync::Arc;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(|req: hyper::Request<Incoming>| async move {
                        assert_eq!(req.method(), hyper::Method::HEAD);
                        Ok::<_, Infallible>(
                            hyper::Response::builder()
                                .status(200)
                                .header("content-length", "0")
                                .body(Full::new(Bytes::new()))
                                .unwrap(),
                        )
                    });
                    let _ = Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection(io, svc)
                        .await;
                });
            }
        });

        let desc = UpstreamDescriptor {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: format!("http://{addr}"),
            timeout: std::time::Duration::from_millis(500),
            host: Some(addr.ip().to_string()),
        };
        let target = UpstreamTarget::from_descriptor(&desc).unwrap();
        let metrics = Arc::new(ProxyHyperMetrics::default());
        let resp = forward_head(&target, "/checkhead", None, &metrics).await;
        assert_eq!(resp.status(), 200);
    }
}
