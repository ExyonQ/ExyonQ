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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

static MODULES_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    config.modules.metrics.scrape_bearer_token = Some("integration-metrics-token".into());
    config
}

async fn spawn_server(config: exyonq_core::AppConfig) -> SocketAddr {
    bootstrap::ensure_integration_modules();
    let wait_for_rate_bucket = config.modules.ratelimit.enabled;
    let std_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let listen = std_listener.local_addr().unwrap();
    drop(std_listener);

    tokio::spawn(async move {
        server::run_on(listen, config).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(150)).await;
    if wait_for_rate_bucket {
        // Process-global limiter state survives server snapshots; allow a previously
        // exhausted localhost bucket to refill before this independent scenario.
        tokio::time::sleep(Duration::from_millis(1_100)).await;
    }
    listen
}

async fn spawn_counting_upstream() -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream");
    let listen = listener.local_addr().expect("upstream addr");
    let hits = Arc::new(AtomicUsize::new(0));
    let task_hits = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let hits = Arc::clone(&task_hits);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    hits.fetch_add(1, Ordering::Relaxed);
                    if stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok",
                        )
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
    (listen, hits)
}

#[tokio::test]
async fn metrics_endpoint_enforces_bearer_and_exposes_prometheus() {
    let _guard = MODULES_TEST_LOCK.lock().await;
    let listen = spawn_server(metrics_config()).await;
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    let uri: Uri = format!("http://{listen}/metrics").parse().unwrap();
    let req = Request::get(uri.clone())
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");
    assert_eq!(response.status(), 401);

    let req = Request::get(uri)
        .header(
            hyper::header::AUTHORIZATION,
            "Bearer integration-metrics-token",
        )
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");

    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("exyonq_http_requests_total"));
}

async fn assert_static_owner_rate_limit_sequence(first_status: u16) {
    let _guard = MODULES_TEST_LOCK.lock().await;
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("EXYONQ_EPOLL_STATIC", "1");
        std::env::set_var("EXYONQ_EPOLL_SENDFILE", "1");
        exyonq_mod_static::reset_epoll_sendfile_enabled_cache_for_tests();
    }
    let root = tempfile::tempdir().expect("static root");
    std::fs::write(root.path().join("large.bin"), vec![b'x'; 65_536]).expect("static fixture");
    let mut config = modules_config();
    config.routes[0].root = Some(root.path().to_path_buf());
    let listen = spawn_server(config).await;
    #[cfg(target_os = "linux")]
    assert!(exyonq_module_api::static_wire::epoll_sendfile_eligible(
        b"GET /site/large.bin HTTP/1.1\r\nHost: test\r\n\r\n"
    ));
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();
    for _ in 0..4 {
        let uri: Uri = format!("http://{listen}/health").parse().unwrap();
        let req = Request::get(uri)
            .header("x-forwarded-for", "198.51.100.90")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let response = timeout(Duration::from_secs(5), client.request(req))
            .await
            .expect("timed out")
            .expect("request failed");
        assert_eq!(response.status(), 200, "health must be rate-limit exempt");
    }

    for spoofed_xff in ["198.51.100.1", "198.51.100.2"] {
        let uri: Uri = format!("http://{listen}/site/large.bin").parse().unwrap();
        let req = Request::get(uri)
            .header("x-forwarded-for", spoofed_xff)
            .body(Full::new(Bytes::new()))
            .unwrap();
        let response = timeout(Duration::from_secs(5), client.request(req))
            .await
            .expect("timed out")
            .expect("request failed");
        assert_eq!(
            response.status(),
            first_status,
            "unexpected static response headers: {:?}",
            response.headers()
        );
    }

    let uri: Uri = format!("http://{listen}/site/large.bin").parse().unwrap();
    let req = Request::get(uri)
        .header("x-forwarded-for", "203.0.113.200")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");
    assert_eq!(response.status(), 429);
    let retry_after = response
        .headers()
        .get(hyper::header::RETRY_AFTER)
        .expect("429 must include Retry-After")
        .to_str()
        .expect("Retry-After must be ASCII")
        .parse::<u64>()
        .expect("Retry-After must be seconds");
    assert!(retry_after >= 1);
}

#[cfg(not(target_os = "linux"))]
#[tokio::test]
async fn ratelimit_ae_idle_static_fast_path_returns_200_200_429() {
    assert_static_owner_rate_limit_sequence(200).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ratelimit_linux_cap067_static_owner_returns_404_404_429() {
    // The Linux Cap067 fixture reaches the existing deterministic static-miss
    // terminal, but admission remains exactly once per keepalive request.
    assert_static_owner_rate_limit_sequence(404).await;
}

#[tokio::test]
async fn ratelimit_ae_gzip_pipeline_returns_200_200_429() {
    let _guard = MODULES_TEST_LOCK.lock().await;
    let listen = spawn_server(modules_config()).await;
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    for expected in [200, 200, 429] {
        let uri: Uri = format!("http://{listen}/site/index.html").parse().unwrap();
        let req = Request::get(uri)
            .header(hyper::header::ACCEPT_ENCODING, "gzip")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let response = timeout(Duration::from_secs(5), client.request(req))
            .await
            .expect("timed out")
            .expect("request failed");
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn ratelimit_ae_idle_proxy_keepalive_returns_200_200_429_before_upstream() {
    let _guard = MODULES_TEST_LOCK.lock().await;
    let (upstream, hits) = spawn_counting_upstream().await;
    let mut config = modules_config();
    config.upstreams.insert(
        "backend".into(),
        exyonq_config_ir::UpstreamConfig::legacy("backend", format!("http://{upstream}"), 5_000),
    );
    let listen = spawn_server(config).await;
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    for expected in [200, 200, 429] {
        let uri: Uri = format!("http://{listen}/api/value").parse().unwrap();
        let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
        let response = timeout(Duration::from_secs(5), client.request(req))
            .await
            .expect("timed out")
            .expect("request failed");
        assert_eq!(response.status(), expected);
    }
    assert_eq!(
        hits.load(Ordering::Relaxed),
        2,
        "the rejected third request must not reach upstream"
    );
}
