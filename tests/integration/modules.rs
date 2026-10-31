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
mod bootstrap;

use bytes::Bytes;
use exyonq_core::server;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{Request, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::timeout;

fn modules_config() -> exyonq_core::AppConfig {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/modules.toml");
    let mut config = exyonq_core::AppConfig::from_file(path).unwrap();
    config.routes[0].root = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/www"));
    config.http3 = Default::default();
    // Cap015: default-on WAF is unrelated to this modules suite.
    config.waf.enabled = false;
    config
}

fn metrics_config() -> exyonq_core::AppConfig {
    let mut config = modules_config();
    // Cap055 module limiter is process-global; do not spend /site/ budget on /metrics.
    config.modules.ratelimit.enabled = false;
    config
}

async fn spawn_server(config: exyonq_core::AppConfig) -> SocketAddr {
    bootstrap::ensure_integration_modules();
    let std_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let listen = std_listener.local_addr().unwrap();
    drop(std_listener);

    tokio::spawn(async move {
        server::run_on(listen, config).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(150)).await;
    listen
}

#[tokio::test]
async fn metrics_endpoint_exposes_prometheus() {
    let listen = spawn_server(metrics_config()).await;
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    let uri: Uri = format!("http://{listen}/metrics").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");

    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("exyonq_http_requests_total"));
}

#[tokio::test]
async fn ratelimit_returns_429_when_exceeded() {
    let listen = spawn_server(modules_config()).await;
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    for _ in 0..2 {
        let uri: Uri = format!("http://{listen}/site/").parse().unwrap();
        let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
        let response = timeout(Duration::from_secs(5), client.request(req))
            .await
            .expect("timed out")
            .expect("request failed");
        assert_eq!(response.status(), 200);
    }

    let uri: Uri = format!("http://{listen}/site/").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");
    assert_eq!(response.status(), 429);
}
