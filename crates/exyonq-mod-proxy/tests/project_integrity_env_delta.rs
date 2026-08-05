//! Differential semantic check: NORMAL vs BENCHMARK-named vs DEMO-named env
//! must not change proxy forward outcomes (status/body/upstream hits).
//!
//! PROJECT_SEMANTIC_ENVIRONMENT_DELTA = NONE (observational variance only).

use exyonq_mod_proxy::hyper_forward::forward_get;
use exyonq_mod_proxy::{ProxyHyperMetrics, UpstreamDescriptor, UpstreamTarget};
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn spawn_counting_upstream(hits: Arc<AtomicU64>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            let hits = Arc::clone(&hits);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let _ = stream.read(&mut buf).await;
                hits.fetch_add(1, Ordering::SeqCst);
                let body = b"DIFF-BODY-1024";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
        upstream_name: "mock".into(),
        target: format!("http://127.0.0.1:{port}"),
        timeout: Duration::from_secs(2),
        host: Some("127.0.0.1".into()),
    };
    UpstreamTarget::from_descriptor(&desc).unwrap()
}

async fn sample(
    upstream: &UpstreamTarget,
    path: &str,
) -> (u16, bytes::Bytes, Vec<(String, String)>) {
    let metrics = Arc::new(ProxyHyperMetrics::default());
    let resp = forward_get(upstream, path, None, &metrics).await;
    let status = resp.status().as_u16();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter_map(|(n, v)| {
            v.to_str()
                .ok()
                .map(|s| (n.as_str().to_string(), s.to_string()))
        })
        .collect();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, body, headers)
}

fn clear_named_envs() {
    for k in [
        "EXYONQ_BENCH_CACHE_HEADERS",
        "BENCHMARK_MODE",
        "DEMO_MODE",
        "BENCH_MODE",
        "PERF_TEST_MODE",
    ] {
        std::env::remove_var(k);
    }
}

fn set_named_env(kind: &str) {
    clear_named_envs();
    match kind {
        "NORMAL" => {}
        "BENCHMARK" => {
            std::env::set_var("BENCHMARK_MODE", "1");
            std::env::set_var("EXYONQ_BENCH_CACHE_HEADERS", "1");
        }
        "DEMO" => {
            std::env::set_var("DEMO_MODE", "1");
            std::env::set_var("EXYONQ_BENCH_CACHE_HEADERS", "true");
        }
        _ => panic!("unknown kind {kind}"),
    }
}

#[tokio::test]
async fn semantic_environment_delta_none_for_api_path() {
    let hits = Arc::new(AtomicU64::new(0));
    let port = spawn_counting_upstream(Arc::clone(&hits)).await;
    let upstream = target(port);

    let mut snapshots = Vec::new();
    for kind in ["NORMAL", "BENCHMARK", "DEMO"] {
        set_named_env(kind);
        let before = hits.load(Ordering::SeqCst);
        let (status, body, headers) = sample(&upstream, "/api/").await;
        let after = hits.load(Ordering::SeqCst);
        assert_eq!(after - before, 1, "{kind}: upstream must be hit once");
        assert_eq!(status, 200, "{kind}");
        assert_eq!(&body[..], b"DIFF-BODY-1024", "{kind}");
        assert!(
            !headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("x-plan10b-cache")
                    || n.eq_ignore_ascii_case("x-plan10b-server")),
            "{kind}: bench cache headers must not appear"
        );
        snapshots.push((status, body, headers));
    }
    clear_named_envs();

    assert_eq!(snapshots[0].0, snapshots[1].0);
    assert_eq!(snapshots[0].0, snapshots[2].0);
    assert_eq!(snapshots[0].1, snapshots[1].1);
    assert_eq!(snapshots[0].1, snapshots[2].1);
    assert_eq!(snapshots[0].2, snapshots[1].2);
    assert_eq!(snapshots[0].2, snapshots[2].2);
    assert_eq!(hits.load(Ordering::SeqCst), 3);
}
