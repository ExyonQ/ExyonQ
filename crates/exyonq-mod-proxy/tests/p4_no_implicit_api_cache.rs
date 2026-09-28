//! BV04-P4 — no implicit proxy response-body reuse without [[cache_policy]].
//!
//! Covers former BENCH_API_CACHE_PATHS paths (`/api/`, `/api/health`) and
//! a non-allowlisted path. Plan 12 explicit cache is tested separately.

use exyonq_mod_proxy::hyper_forward::forward_get;
use exyonq_mod_proxy::{ProxyHyperMetrics, UpstreamDescriptor, UpstreamTarget};
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

struct UpstreamProbeState {
    hits: AtomicU64,
    body_mode: AtomicUsize, // 0=A, 1=B
    last_path: Mutex<String>,
}

async fn spawn_counting_upstream(state: Arc<UpstreamProbeState>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let path_line = req.lines().next().unwrap_or("");
                let path = path_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                {
                    let mut g = state.last_path.lock().await;
                    *g = path.clone();
                }
                state.hits.fetch_add(1, Ordering::SeqCst);
                let body: &[u8] = if state.body_mode.load(Ordering::SeqCst) == 0 {
                    b"BODY-A-xxxxxxxx"
                } else {
                    b"BODY-B-yyyyyyyy"
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                let _ = stream.write_all(body).await;
            });
        }
    });
    port
}

fn target(port: u16) -> UpstreamTarget {
    let desc = UpstreamDescriptor {
        cluster_id: 0,
        upstream_name: "peer".into(),
        target: format!("http://127.0.0.1:{port}"),
        timeout: Duration::from_secs(2),
        host: Some("127.0.0.1".into()),
        max_connect_retries: 1,
    };
    UpstreamTarget::from_descriptor(&desc).unwrap()
}

async fn get_body(upstream: &UpstreamTarget, path: &str) -> (u16, bytes::Bytes) {
    let metrics = Arc::new(ProxyHyperMetrics::default());
    let resp = forward_get(upstream, path, None, &metrics).await;
    let status = resp.status().as_u16();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, body)
}

#[tokio::test]
async fn api_path_five_gets_equal_five_upstream_hits() {
    let state = Arc::new(UpstreamProbeState {
        hits: AtomicU64::new(0),
        body_mode: AtomicUsize::new(0),
        last_path: Mutex::new(String::new()),
    });
    let port = spawn_counting_upstream(Arc::clone(&state)).await;
    let upstream = target(port);
    for _ in 0..5 {
        let (status, body) = get_body(&upstream, "/api/").await;
        assert_eq!(status, 200);
        assert_eq!(&body[..], b"BODY-A-xxxxxxxx");
    }
    assert_eq!(state.hits.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn api_health_five_gets_equal_five_upstream_hits() {
    let state = Arc::new(UpstreamProbeState {
        hits: AtomicU64::new(0),
        body_mode: AtomicUsize::new(0),
        last_path: Mutex::new(String::new()),
    });
    let port = spawn_counting_upstream(Arc::clone(&state)).await;
    let upstream = target(port);
    for _ in 0..5 {
        let (status, _) = get_body(&upstream, "/api/health").await;
        assert_eq!(status, 200);
    }
    assert_eq!(state.hits.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn unique_query_strings_each_hit_upstream() {
    let state = Arc::new(UpstreamProbeState {
        hits: AtomicU64::new(0),
        body_mode: AtomicUsize::new(0),
        last_path: Mutex::new(String::new()),
    });
    let port = spawn_counting_upstream(Arc::clone(&state)).await;
    let upstream = target(port);
    for i in 1..=3 {
        let (status, _) = get_body(&upstream, &format!("/api/?nonce={i}")).await;
        assert_eq!(status, 200);
    }
    assert_eq!(state.hits.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn upstream_body_change_propagates_without_policy() {
    let state = Arc::new(UpstreamProbeState {
        hits: AtomicU64::new(0),
        body_mode: AtomicUsize::new(0),
        last_path: Mutex::new(String::new()),
    });
    let port = spawn_counting_upstream(Arc::clone(&state)).await;
    let upstream = target(port);
    let (s1, b1) = get_body(&upstream, "/api/").await;
    assert_eq!(s1, 200);
    assert_eq!(&b1[..], b"BODY-A-xxxxxxxx");
    state.body_mode.store(1, Ordering::SeqCst);
    let (s2, b2) = get_body(&upstream, "/api/").await;
    assert_eq!(s2, 200);
    assert_eq!(&b2[..], b"BODY-B-yyyyyyyy");
    assert_eq!(state.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn upstream_down_does_not_return_stale_200() {
    let state = Arc::new(UpstreamProbeState {
        hits: AtomicU64::new(0),
        body_mode: AtomicUsize::new(0),
        last_path: Mutex::new(String::new()),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accept = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        state.hits.fetch_add(1, Ordering::SeqCst);
        let body = b"BODY-A-xxxxxxxx";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes()).await;
        let _ = stream.write_all(body).await;
        // drop listener + connection → no further accepts
        drop(listener);
    });
    let upstream = target(port);
    let (s1, b1) = get_body(&upstream, "/api/").await;
    assert_eq!(s1, 200);
    assert_eq!(&b1[..], b"BODY-A-xxxxxxxx");
    let _ = accept.await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (s2, b2) = get_body(&upstream, "/api/").await;
    assert_ne!(s2, 200, "stale 200 after upstream stop is forbidden");
    assert_ne!(&b2[..], b"BODY-A-xxxxxxxx");
    assert!(s2 == 502 || s2 == 504 || s2 == 503, "got status {s2}");
}

#[tokio::test]
async fn diagnostic_route_pass_through() {
    let state = Arc::new(UpstreamProbeState {
        hits: AtomicU64::new(0),
        body_mode: AtomicUsize::new(0),
        last_path: Mutex::new(String::new()),
    });
    let port = spawn_counting_upstream(Arc::clone(&state)).await;
    let upstream = target(port);
    for _ in 0..5 {
        let (status, _) = get_body(&upstream, "/api-diagnostic/").await;
        assert_eq!(status, 200);
    }
    assert_eq!(state.hits.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn concurrent_eight_requests_eight_upstream_hits() {
    let state = Arc::new(UpstreamProbeState {
        hits: AtomicU64::new(0),
        body_mode: AtomicUsize::new(0),
        last_path: Mutex::new(String::new()),
    });
    let port = spawn_counting_upstream(Arc::clone(&state)).await;
    let upstream = Arc::new(target(port));
    let mut joins = Vec::new();
    for _ in 0..8 {
        let u = Arc::clone(&upstream);
        joins.push(tokio::spawn(async move {
            let (status, _) = get_body(&u, "/api/").await;
            assert_eq!(status, 200);
        }));
    }
    for j in joins {
        j.await.unwrap();
    }
    assert_eq!(state.hits.load(Ordering::SeqCst), 8);
}

#[test]
fn upstream_target_exposes_no_response_body_store_api() {
    // After remediation, UpstreamTarget only carries URI/host/timeout — no
    // api_cache_hit / try_store_api_cache surface.
    let t = UpstreamTarget::from_config("http://127.0.0.1:9", 1000).unwrap();
    let _ = t.uri_for("/api/");
    let _ = t.uri_for("/api/health");
    let _ = t.host.clone();
    let _ = t.timeout;
}
