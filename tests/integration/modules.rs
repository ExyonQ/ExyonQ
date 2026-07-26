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
    let listen = spawn_server(modules_config()).await;
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
