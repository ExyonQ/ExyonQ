mod bootstrap;

use bytes::Bytes;
use exyonq_core::AppConfig;
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

fn ephemeral_addr() -> SocketAddr {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

async fn spawn_server(config: AppConfig) -> SocketAddr {
    bootstrap::ensure_integration_modules();
    let listen = ephemeral_addr();
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

    assert_eq!(response.status(), 200);
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

    let uri: Uri = format!("http://{listen}/site/../www/index.html")
        .parse()
        .unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();

    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");

    assert_eq!(response.status(), 403);
}
