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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

async fn run_echo_upstream(addr: SocketAddr) {
    tokio::spawn(async move {
        let listener = TcpListener::bind(addr).await.unwrap();
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..n]);
                let body = if request.contains("GET /api/health") {
                    "upstream-ok"
                } else {
                    "echo"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
}

#[tokio::test]
async fn proxies_to_configured_upstream() {
    let upstream_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();
    drop(upstream_listener);
    run_echo_upstream(upstream_addr).await;

    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml");
    let mut config = AppConfig::from_file(&config_path).expect("config");
    config.upstreams.get_mut("backend").unwrap().target = format!("http://{upstream_addr}");

    let listen = {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        addr
    };

    bootstrap::ensure_integration_modules();
    tokio::spawn(async move {
        exyonq_core::server::run_on(listen, config).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(150)).await;

    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    let uri: Uri = format!("http://{listen}/api/health").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();

    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");

    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"upstream-ok");
}
