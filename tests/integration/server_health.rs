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

#[tokio::test]
async fn serve_responds_to_health() {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml");
    let config = AppConfig::from_file(&config_path).expect("config");

    let std_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let listen = std_listener.local_addr().unwrap();
    drop(std_listener);

    bootstrap::ensure_integration_modules();
    tokio::spawn(async move {
        exyonq_core::server::run_on(listen, config).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    let uri: Uri = format!("http://{listen}/health").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();

    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");

    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}

#[tokio::test]
async fn serve_returns_404_for_unknown_paths() {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml");
    let config = AppConfig::from_file(&config_path).expect("config");

    let std_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let listen: SocketAddr = std_listener.local_addr().unwrap();
    drop(std_listener);

    bootstrap::ensure_integration_modules();
    tokio::spawn(async move {
        exyonq_core::server::run_on(listen, config).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    let uri: Uri = format!("http://{listen}/missing").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();

    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");

    assert_eq!(response.status(), 404);
}
