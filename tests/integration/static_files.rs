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
use exyonq_core::AppConfig;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{Request, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::timeout;

fn ephemeral_addr() -> SocketAddr {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

async fn spawn_server(mut config: AppConfig) -> SocketAddr {
    bootstrap::ensure_integration_modules();
    let listen = ephemeral_addr();
    // run_on also binds every address configured on a server. Keep the config
    // on the ephemeral port this test owns instead of the fixture's fixed 8080,
    // which may already be in use on a shared host.
    config.servers[0].listen = listen.to_string();
    tokio::spawn(async move {
        exyonq_core::server::run_on(listen, config).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    listen
}

#[tokio::test]
async fn serves_index_html_from_root() {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/static.toml");
    let mut config = AppConfig::from_file(&config_path).expect("config");
    config.routes[0].root = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/www"));

    let listen = spawn_server(config).await;

    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    let uri: Uri = format!("http://{listen}/site/").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();

    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.windows(6).any(|window| window == b"static"));
}

#[tokio::test]
async fn blocks_path_traversal() {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/static.toml");
    let mut config = AppConfig::from_file(&config_path).expect("config");
    config.routes[0].root = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/www"));

    let listen = spawn_server(config).await;

    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    // Above-root traversal is rejected before route selection (H-01).
    let uri: Uri = format!("http://{listen}/../www/index.html")
        .parse()
        .unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // In-root dot segments are canonicalized *before* routing (H-01): the
    // request `/site/../www/index.html` becomes `/www/index.html`, which no
    // route owns. The static root must not serve its file for that spelling.
    let uri: Uri = format!("http://{listen}/site/../www/index.html")
        .parse()
        .unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        !body.windows(6).any(|window| window == b"static"),
        "traversal must not serve the static body"
    );
}
