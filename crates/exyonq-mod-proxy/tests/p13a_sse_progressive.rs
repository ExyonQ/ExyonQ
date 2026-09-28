//! P1.3a — SSE progressive read without full-buffer claim.
//!
//! Spawns a chunked `text/event-stream` upstream, forwards via
//! `forward_get_streaming`, and asserts the client observes the first event
//! before the stream completes.

use exyonq_mod_proxy::{
    forward_get_streaming, ProxyHyperMetrics, UpstreamDescriptor, UpstreamTarget,
};
use http_body_util::BodyExt;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn chunked(data: &str) -> Vec<u8> {
    format!("{:x}\r\n{}\r\n", data.len(), data).into_bytes()
}

async fn spawn_sse_upstream() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let headers = concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/event-stream\r\n",
                    "Cache-Control: no-cache\r\n",
                    "Transfer-Encoding: chunked\r\n",
                    "\r\n"
                );
                if stream.write_all(headers.as_bytes()).await.is_err() {
                    return;
                }
                let _ = stream.write_all(&chunked("data: one\n\n")).await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                let _ = stream.write_all(&chunked("data: two\n\n")).await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                let _ = stream.write_all(&chunked("data: three\n\n")).await;
                let _ = stream.write_all(b"0\r\n\r\n").await;
            });
        }
    });
    port
}

#[tokio::test]
async fn sse_progressive_read_without_full_buffer() {
    let port = spawn_sse_upstream().await;
    let desc = UpstreamDescriptor {
        cluster_id: 0,
        upstream_name: "sse".into(),
        target: format!("http://127.0.0.1:{port}"),
        timeout: Duration::from_secs(5),
        host: Some("127.0.0.1".into()),
        max_connect_retries: 1,
    };
    let upstream = UpstreamTarget::from_descriptor(&desc).unwrap();
    let metrics = Arc::new(ProxyHyperMetrics::default());

    let response = forward_get_streaming(&upstream, "/api/stream", None, &metrics).await;
    assert_eq!(response.status(), 200);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.contains("text/event-stream")),
        "expected event-stream content-type"
    );

    let mut body = response.into_body();
    let mut seen = String::new();
    let mut saw_one_before_complete = false;
    while let Some(frame) = body.frame().await {
        let frame = frame.expect("frame");
        let Ok(data) = frame.into_data() else {
            continue;
        };
        seen.push_str(&String::from_utf8_lossy(data.as_ref()));
        if seen.contains("data: one") && !seen.contains("data: three") {
            saw_one_before_complete = true;
        }
    }
    assert!(
        saw_one_before_complete,
        "expected progressive delivery; got full buffer only: {seen:?}"
    );
    assert!(seen.contains("data: one"));
    assert!(seen.contains("data: two"));
    assert!(seen.contains("data: three"));
    assert!(
        metrics.sse_streams.load(Ordering::Relaxed) >= 1,
        "sse_streams counter should increment"
    );
}
