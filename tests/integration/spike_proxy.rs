use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{Request, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

async fn run_echo_upstream(addr: SocketAddr) -> tokio::task::JoinHandle<()> {
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
                let body = if request.contains("GET /health") {
                    "ok"
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
    })
}

#[tokio::test]
async fn spike_proxy_forwards_requests() {
    let upstream_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listen_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let upstream_listener = TcpListener::bind(upstream_addr).await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();
    drop(upstream_listener);

    let _upstream = run_echo_upstream(upstream_addr).await;

    let listen_listener = TcpListener::bind(listen_addr).await.unwrap();
    let listen_addr = listen_listener.local_addr().unwrap();
    drop(listen_listener);

    let upstream_uri = format!("http://{upstream_addr}").parse().unwrap();
    tokio::spawn(async move {
        exyonq_mod_proxy::run_spike_proxy(listen_addr, upstream_uri)
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    let uri: Uri = format!("http://{listen_addr}/health").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();

    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("request timed out")
        .expect("request failed");

    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}
