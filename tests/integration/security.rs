mod bootstrap;

use bytes::Bytes;
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::tls::{SharedTlsAcceptor, TlsSessionCache};
use exyonq_core::{reload, AppConfig};
use exyonq_discovery_runtime::apply_file_overlay;
use exyonq_mod_proxy::build_incoming_client;
use http_body_util::Full;
use hyper::header::HeaderValue;
use hyper::{Method, Request, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

#[cfg(unix)]
static CONTROL_SOCKET_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Serializes `spawn_server` while control-socket integration tests run with process env set.
#[cfg(unix)]
static SPAWN_SERVER_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn spawn_server(config: AppConfig) -> SocketAddr {
    #[cfg(unix)]
    let _spawn_gate = SPAWN_SERVER_GATE.lock().await;
    #[cfg(unix)]
    if CONTROL_SOCKET_TESTS.try_lock().is_ok() {
        std::env::remove_var("EXYONQ_CONTROL_SOCKET");
        std::env::remove_var("EXYONQ_CONFIG");
    }
    spawn_server_inner(config).await
}

#[cfg(unix)]
async fn spawn_server_with_control_env(config: AppConfig) -> SocketAddr {
    spawn_server_inner(config).await
}

async fn spawn_server_inner(config: AppConfig) -> SocketAddr {
    let (addr, _handle) = spawn_server_inner_with_handle(config).await;
    addr
}

async fn spawn_server_inner_with_handle(
    config: AppConfig,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    bootstrap::ensure_integration_modules();
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let handle = tokio::spawn(async move {
        exyonq_core::server::run_on(addr, config).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    (addr, handle)
}

#[cfg(unix)]
async fn connect_control_socket(path: &std::path::Path) -> tokio::net::UnixStream {
    use tokio::net::UnixStream;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        match UnixStream::connect(path).await {
            Ok(stream) => return stream,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(err) => panic!("control socket connect failed: {err}"),
        }
    }
}

#[cfg(unix)]
async fn read_control_line(
    stream: &mut tokio::net::UnixStream,
    op_deadline: Duration,
) -> Result<String, ()> {
    use tokio::io::AsyncBufReadExt;
    let deadline = tokio::time::Instant::now() + op_deadline;
    let mut line = String::new();
    loop {
        line.clear();
        let read = timeout(Duration::from_millis(500), async {
            tokio::io::BufReader::new(&mut *stream)
                .read_line(&mut line)
                .await
        })
        .await;
        match read {
            Ok(Ok(_)) if !line.trim().is_empty() => return Ok(line),
            _ if tokio::time::Instant::now() >= deadline => return Err(()),
            _ => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
}

#[cfg(unix)]
async fn try_control_status_once(path: &std::path::Path) -> Result<serde_json::Value, ()> {
    use tokio::net::UnixStream;
    let mut stream = UnixStream::connect(path).await.map_err(|_| ())?;
    stream.write_all(b"status\n").await.map_err(|_| ())?;
    stream.flush().await.map_err(|_| ())?;
    let line = read_control_line(&mut stream, Duration::from_secs(1)).await?;
    serde_json::from_str(line.trim()).map_err(|_| ())
}

#[cfg(unix)]
async fn wait_control_socket_ready(path: &std::path::Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match try_control_status_once(path).await {
            Ok(status) if status["ok"].as_bool() == Some(true) => return,
            Ok(status) if tokio::time::Instant::now() >= deadline => {
                panic!("control socket readiness: status not ok: {status}");
            }
            Err(_) if tokio::time::Instant::now() >= deadline => {
                panic!("control socket readiness: unavailable");
            }
            _ => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
}

#[cfg(unix)]
async fn control_command(path: &std::path::Path, command: &str) -> serde_json::Value {
    let mut stream = connect_control_socket(path).await;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .await
        .unwrap();
    stream.flush().await.unwrap();
    let line = read_control_line(&mut stream, Duration::from_secs(2))
        .await
        .unwrap_or_else(|_| panic!("control socket {command}: no JSON response"));
    serde_json::from_str(line.trim()).unwrap_or_else(|err| {
        panic!("control socket {command}: invalid JSON {line:?}: {err}");
    })
}

async fn run_echo_upstream(addr: SocketAddr) {
    tokio::spawn(async move {
        let listener = TcpListener::bind(addr).await.unwrap();
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                let _ = &buf[..n];
                let body = b"echo";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    String::from_utf8_lossy(body)
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
}

#[tokio::test]
async fn security_rejects_duplicate_content_length() {
    let upstream_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();
    drop(upstream_listener);
    run_echo_upstream(upstream_addr).await;

    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml");
    let mut config = AppConfig::from_file(&config_path).expect("config");
    config.upstreams.get_mut("backend").unwrap().target = format!("http://{upstream_addr}");

    let listen = spawn_server(config).await;

    let mut stream = TcpStream::connect(listen).await.unwrap();
    stream
        .write_all(
            b"GET /api/ HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nContent-Length: 1\r\n\r\n",
        )
        .await
        .unwrap();

    let mut buf = vec![0u8; 512];
    let n = timeout(Duration::from_secs(3), stream.read(&mut buf))
        .await
        .expect("timed out")
        .expect("read failed");
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.contains("400") || response.contains("Bad Request"),
        "expected 400, got: {response}"
    );
}

#[tokio::test]
async fn security_static_header_read_timeout() {
    let prev = std::env::var("EXYONQ_READ_TIMEOUT_MS").ok();
    std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "100");

    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/static.toml");
    let mut config = AppConfig::from_file(&config_path).expect("config");
    config.routes[0].root = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/www"));

    let listen = spawn_server(config).await;

    let mut stream = TcpStream::connect(listen).await.unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(250)).await;
    let mut buf = [0u8; 64];
    let read = stream.read(&mut buf).await;
    assert!(
        read.is_err() || read.unwrap_or(0) == 0,
        "connection should close after header timeout"
    );

    match prev {
        Some(value) => std::env::set_var("EXYONQ_READ_TIMEOUT_MS", value),
        None => std::env::remove_var("EXYONQ_READ_TIMEOUT_MS"),
    }
}

#[test]
fn security_proxy_header_validation() {
    assert!(exyonq_core::request_headers_safe_for_proxy(
        &hyper::HeaderMap::new()
    ));
    let mut headers = hyper::HeaderMap::new();
    headers.insert(hyper::header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    headers.append(hyper::header::CONTENT_LENGTH, HeaderValue::from_static("1"));
    assert!(!exyonq_core::request_headers_safe_for_proxy(&headers));
}

#[test]
fn security_discovery_invalid_json_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = dir.path().join("bad.json");
    std::fs::write(&discovery, "{not json").unwrap();

    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml");
    let config = AppConfig::from_file(&config_path).expect("config");
    let merged = apply_file_overlay(&config, &discovery);
    assert_eq!(merged.upstreams.len(), config.upstreams.len());
}

#[tokio::test]
async fn security_http3_rejects_post() {
    bootstrap::ensure_integration_modules();
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/static.toml");
    let mut config = AppConfig::from_file(&config_path).expect("config");
    config.routes[0].root = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/www"));

    let proxy_client = build_incoming_client();
    let state = ServerState::new(config, proxy_client.clone())
        .await
        .unwrap();
    let shared = reload::wrap_state(state);

    let state = reload::read_state(&shared);

    let req = Request::builder()
        .method(Method::POST)
        .uri("http://127.0.0.1/site/")
        .body(())
        .unwrap();

    let response = serve_http3_request(
        ConnectionContext {
            state,
            proxy_client,
            x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        req,
    )
    .await;

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[cfg(unix)]
#[tokio::test]
async fn security_control_reload_invalid_config() {
    let _guard = CONTROL_SOCKET_TESTS.lock().await;
    let _spawn_gate = SPAWN_SERVER_GATE.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("bench.toml");
    let socket_path = dir.path().join("exyonq.sock");

    let valid = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml"),
    )
    .unwrap();
    std::fs::write(&config_path, &valid).unwrap();

    std::env::set_var("EXYONQ_CONFIG", config_path.to_str().unwrap());
    std::env::set_var("EXYONQ_CONTROL_SOCKET", socket_path.to_str().unwrap());

    let config = AppConfig::from_file(&config_path).expect("config");

    let listen = spawn_server_with_control_env(config).await;
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();
    let uri: Uri = format!("http://{listen}/health").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
    assert_eq!(
        timeout(Duration::from_secs(3), client.request(req))
            .await
            .expect("timed out")
            .expect("request")
            .status(),
        200
    );

    wait_control_socket_ready(&socket_path).await;

    let before = control_command(&socket_path, "status").await;
    let gen_before = before["generation"].as_u64().unwrap();

    std::fs::write(&config_path, "not valid toml [[[").unwrap();

    let reload = control_command(&socket_path, "reload").await;
    assert_eq!(reload["ok"], false);

    let after = control_command(&socket_path, "status").await;
    assert_eq!(after["generation"].as_u64().unwrap(), gen_before);

    std::env::remove_var("EXYONQ_CONTROL_SOCKET");
    std::env::remove_var("EXYONQ_CONFIG");
}

#[cfg(unix)]
async fn http_exchange(addr: SocketAddr, request: &[u8]) -> String {
    let mut tcp = TcpStream::connect(addr).await.expect("connect");
    tcp.write_all(request).await.expect("write");
    let mut buf = vec![0u8; 1024];
    let n = tcp.read(&mut buf).await.expect("read");
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

#[cfg(unix)]
#[tokio::test]
async fn security_control_drain_rejects_new_requests() {
    let _guard = CONTROL_SOCKET_TESTS.lock().await;
    let _spawn_gate = SPAWN_SERVER_GATE.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("bench.toml");
    let socket_path = dir.path().join("exyonq.sock");

    let valid = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml"),
    )
    .unwrap();
    std::fs::write(&config_path, &valid).unwrap();

    std::env::set_var("EXYONQ_CONFIG", config_path.to_str().unwrap());
    std::env::set_var("EXYONQ_CONTROL_SOCKET", socket_path.to_str().unwrap());

    let config = AppConfig::from_file(&config_path).expect("config");

    let listen = spawn_server_with_control_env(config).await;
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();

    // Normal: /health, /ready, /live available before drain.
    for path in ["/health", "/ready", "/live"] {
        let uri: Uri = format!("http://{listen}{path}").parse().unwrap();
        let req = Request::builder()
            .header(hyper::header::CONNECTION, "close")
            .uri(uri)
            .body(Full::new(Bytes::new()))
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(3), client.request(req))
                .await
                .expect("timed out")
                .expect("request")
                .status(),
            200,
            "pre-drain {path}"
        );
    }

    wait_control_socket_ready(&socket_path).await;

    let drain = control_command(&socket_path, "drain").await;
    assert_eq!(drain["ok"], true);
    assert_eq!(drain["command"], "drain");
    assert_eq!(drain["draining"], true);

    let status = control_command(&socket_path, "status").await;
    assert_eq!(status["ok"], true);
    assert_eq!(status["draining"], true);

    // Product path must 503 during drain (KF-P16-014 / P15-WS5-PROBE-002).
    let product = http_exchange(
        listen,
        b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        product.contains("503") && product.contains("draining"),
        "expected product 503 draining, got: {product}"
    );

    // Readiness fails closed; liveness stays up; listener still accepts.
    let ready = http_exchange(
        listen,
        b"GET /ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        ready.contains("503") && ready.contains("not_ready"),
        "expected /ready not_ready during drain, got: {ready}"
    );

    let live = http_exchange(
        listen,
        b"GET /live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        live.contains("200") && live.contains("live"),
        "expected /live 200 during drain after product 503, got: {live}"
    );

    // Drain is idempotent (no panic / no double-close of listener).
    let drain2 = control_command(&socket_path, "drain").await;
    assert_eq!(drain2["ok"], true);
    assert_eq!(drain2["draining"], true);
    let live2 = http_exchange(
        listen,
        b"GET /live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        live2.contains("200") && live2.contains("live"),
        "expected /live 200 after repeated drain, got: {live2}"
    );

    std::env::remove_var("EXYONQ_CONTROL_SOCKET");
    std::env::remove_var("EXYONQ_CONFIG");
}

#[cfg(unix)]
#[tokio::test]
async fn security_control_drain_then_shutdown_closes_listener() {
    let _guard = CONTROL_SOCKET_TESTS.lock().await;
    let _spawn_gate = SPAWN_SERVER_GATE.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("bench.toml");
    let socket_path = dir.path().join("exyonq.sock");

    let valid = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml"),
    )
    .unwrap();
    std::fs::write(&config_path, &valid).unwrap();

    std::env::set_var("EXYONQ_CONFIG", config_path.to_str().unwrap());
    std::env::set_var("EXYONQ_CONTROL_SOCKET", socket_path.to_str().unwrap());

    let config = AppConfig::from_file(&config_path).expect("config");
    let (listen, handle) = spawn_server_inner_with_handle(config).await;
    wait_control_socket_ready(&socket_path).await;

    let drain = control_command(&socket_path, "drain").await;
    assert_eq!(drain["ok"], true);
    assert_eq!(drain["draining"], true);

    let live = http_exchange(
        listen,
        b"GET /live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        live.contains("200") && live.contains("live"),
        "live must work during drain before shutdown, got: {live}"
    );

    let shutdown = control_command(&socket_path, "shutdown").await;
    assert_eq!(shutdown["ok"], true);

    timeout(Duration::from_secs(5), handle)
        .await
        .expect("server should exit after shutdown")
        .expect("join ok");

    // Listener must be gone after shutdown.
    let refused = TcpStream::connect(listen).await;
    assert!(
        refused.is_err(),
        "listener must refuse after shutdown, got {refused:?}"
    );

    std::env::remove_var("EXYONQ_CONTROL_SOCKET");
    std::env::remove_var("EXYONQ_CONFIG");
}

/// P1.6-WS5R-PREBUILD / SECINT-003 classification.
/// Contract: new *product* request on a pre-drain keepalive must 503; probes stay distinct.
/// (Recovery WIP used `/health` post-drain — that is a liveness probe and must stay 200.)
#[cfg(unix)]
#[tokio::test]
async fn security_secint003_post_drain_keepalive_product_rejects() {
    let _guard = CONTROL_SOCKET_TESTS.lock().await;
    let _spawn_gate = SPAWN_SERVER_GATE.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("bench.toml");
    let socket_path = dir.path().join("exyonq.sock");

    let valid = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml"),
    )
    .unwrap();
    std::fs::write(&config_path, &valid).unwrap();

    std::env::set_var("EXYONQ_CONFIG", config_path.to_str().unwrap());
    std::env::set_var("EXYONQ_CONTROL_SOCKET", socket_path.to_str().unwrap());

    let config = AppConfig::from_file(&config_path).expect("config");
    let listen = spawn_server_with_control_env(config).await;
    wait_control_socket_ready(&socket_path).await;

    let mut tcp = TcpStream::connect(listen).await.expect("connect");
    // Admit keepalive with a probe (allowed to finish).
    tcp.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .expect("write first");
    let mut buf = vec![0u8; 1024];
    let n = tcp.read(&mut buf).await.expect("read first");
    let first = String::from_utf8_lossy(&buf[..n]);
    assert!(first.contains("200"), "expected initial 200, got: {first}");

    let drain = control_command(&socket_path, "drain").await;
    assert_eq!(drain["ok"], true);
    assert_eq!(drain["draining"], true);

    // New *product* request on the same keepalive after drain.
    tcp.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("write product after drain");
    let n = match timeout(Duration::from_secs(3), tcp.read(&mut buf)).await {
        Ok(Ok(n)) => n,
        Ok(Err(_)) | Err(_) => 0,
    };
    let second = String::from_utf8_lossy(&buf[..n]);
    assert!(
        second.contains("503") && second.contains("draining"),
        "SECINT-003: post-drain keepalive product must 503 draining, got: {second}"
    );
    assert!(
        !second.contains("200 OK"),
        "SECINT-003: must not return 200 OK on product keepalive after drain, got: {second}"
    );

    // New connection probes still follow drain contract.
    let live = http_exchange(
        listen,
        b"GET /live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        live.contains("200") && live.contains("live"),
        "expected /live 200 on new conn during drain, got: {live}"
    );
    let ready = http_exchange(
        listen,
        b"GET /ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        ready.contains("503") && ready.contains("not_ready"),
        "expected /ready not_ready, got: {ready}"
    );

    std::env::remove_var("EXYONQ_CONTROL_SOCKET");
    std::env::remove_var("EXYONQ_CONFIG");
}

/// SECINT-001 — H3 proxy composition (fixture-only remediation).
///
/// Canonical ownership: one `StdTcpListener` is bound once, converted with
/// `TcpListener::from_std`, and never dropped/rebound. Readiness is observed by a
/// successful upstream HTTP exchange (no sleep-only waits). Teardown aborts the
/// accept task with a bounded join timeout.
///
/// Product H3 GET/HEAD/POST use process-wide Hyper pools from
/// `ensure_integration_modules`. On Unix this test takes `SPAWN_SERVER_GATE` so
/// it does not overlap control-plane spawn storms (connect-timeout → 502).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn security_secint001_h3_proxy_get_with_upstream() {
    #[cfg(unix)]
    let _spawn_gate = SPAWN_SERVER_GATE.lock().await;
    bootstrap::ensure_integration_modules();

    let std_listener = StdTcpListener::bind("127.0.0.1:0").expect("upstream bind");
    std_listener
        .set_nonblocking(true)
        .expect("upstream nonblocking");
    let upstream_addr = std_listener.local_addr().expect("upstream addr");
    let listener = TcpListener::from_std(std_listener).expect("upstream from_std");

    let upstream_task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let Ok(n) = stream.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                        if buf.len() > 64 * 1024 {
                            return;
                        }
                        continue;
                    };
                    let headers = &buf[..header_end];
                    let is_head = headers.starts_with(b"HEAD ");
                    let mut body = buf[header_end + 4..].to_vec();
                    let cl = {
                        let s = std::str::from_utf8(headers).unwrap_or("");
                        s.lines()
                            .find_map(|line| {
                                let lower = line.to_ascii_lowercase();
                                lower
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0)
                    };
                    while body.len() < cl {
                        let Ok(n) = stream.read(&mut tmp).await else {
                            break;
                        };
                        if n == 0 {
                            break;
                        }
                        body.extend_from_slice(&tmp[..n]);
                    }
                    if body.len() > cl {
                        body.truncate(cl);
                    }
                    // GET → fixed pong; POST → echo; HEAD → headers only.
                    let out = if cl == 0 { b"pong".as_slice() } else { body.as_slice() };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nX-Exyonq-Upstream: h3-proxy\r\nConnection: close\r\n\r\n",
                        out.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    if !is_head {
                        let _ = stream.write_all(out).await;
                    }
                    return;
                }
            });
        }
    });

    // Observable readiness (not sleep-only): upstream must answer HTTP 200.
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let probe = async {
                let mut stream = TcpStream::connect(upstream_addr).await.ok()?;
                stream
                    .write_all(b"GET /ready HTTP/1.1\r\nHost: upstream\r\nConnection: close\r\n\r\n")
                    .await
                    .ok()?;
                let mut buf = [0u8; 256];
                let n = timeout(Duration::from_millis(250), stream.read(&mut buf))
                    .await
                    .ok()?
                    .ok()?;
                Some(String::from_utf8_lossy(&buf[..n]).contains("200"))
            }
            .await;
            if probe == Some(true) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                upstream_task.abort();
                panic!("SECINT-001: upstream readiness failed before product path");
            }
            tokio::task::yield_now().await;
        }
    }

    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml");
    let mut config = AppConfig::from_file(&config_path).expect("minimal.toml");
    config.upstreams.get_mut("backend").unwrap().target = format!("http://{upstream_addr}");

    let proxy_client = build_incoming_client();
    let state = ServerState::new(config, proxy_client.clone())
        .await
        .unwrap();
    let shared = reload::wrap_state(state);

    let cases = [
        (Method::GET, "http://127.0.0.1/api/ping", Bytes::new()),
        (Method::HEAD, "http://127.0.0.1/api/ping", Bytes::new()),
        (
            Method::POST,
            "http://127.0.0.1/api/echo",
            Bytes::from_static(b"tiny"),
        ),
    ];

    for (method, uri, body) in cases {
        for iter in 0..20 {
            let mut last_status = StatusCode::INTERNAL_SERVER_ERROR;
            for attempt in 0..8 {
                let req = Request::builder()
                    .method(method.clone())
                    .uri(uri)
                    .header("content-type", "application/octet-stream")
                    .body(body.clone())
                    .unwrap();
                let ctx = ConnectionContext {
                    state: reload::read_state(&shared),
                    proxy_client: proxy_client.clone(),
                    x_forwarded_for: HeaderValue::from_static("203.0.113.9"),
                    ops: exyonq_core::lifecycle::LifecycleState::new(),
                };
                let response = serve_http3_request(ctx, req).await;
                last_status = response.status();
                if last_status == StatusCode::OK {
                    break;
                }
                // Bounded backoff for transient scheduling under suite load —
                // never accept 502 as success.
                let _ = attempt;
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            assert_eq!(
                last_status,
                StatusCode::OK,
                "SECINT-001: H3 {method} iter={iter} with healthy upstream must be 200, got {last_status}"
            );
        }
    }

    upstream_task.abort();
    match timeout(Duration::from_secs(2), upstream_task).await {
        Ok(Ok(())) | Ok(Err(_)) | Err(_) => {}
    }
}

#[tokio::test]
async fn security_reload_invalid_config_keeps_snapshot() {
    bootstrap::ensure_integration_modules();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let valid = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/static.toml"),
    )
    .unwrap();
    std::fs::write(&config_path, &valid).unwrap();

    let mut config = AppConfig::from_file(&config_path).expect("config");
    config.routes[0].root = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/www"));

    let proxy_client = build_incoming_client();
    let tls_acceptor = SharedTlsAcceptor::new();
    let tls_cache = TlsSessionCache::default();
    let state = ServerState::new(config, proxy_client.clone())
        .await
        .unwrap();
    let shared = reload::wrap_state(state);
    let gen_before = reload::read_state(&shared).generation;

    std::fs::write(&config_path, "[[[ invalid").unwrap();
    assert!(reload::reload_from_path(
        &config_path,
        &shared,
        &proxy_client,
        &tls_acceptor,
        &tls_cache,
    )
    .await
    .is_err());
    assert_eq!(reload::read_state(&shared).generation, gen_before);
}
