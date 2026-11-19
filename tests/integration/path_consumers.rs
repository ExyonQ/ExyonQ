/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! Consumers of the canonical request path, in its URI form.
//!
//! `canonicalize_path` keeps the request target in URI form (`%20` stays
//! escaped, `(` becomes literal). These tests read that contract from the
//! outside, through a real server, for the three consumers that read the path
//! after canonicalization: the sensitive-name gate, route rewrites (literal and
//! patterned, including a capture) and the upstream request-target.

mod bootstrap;
/// The servers in this binary share the process-wide proxy client and the OS
/// ephemeral port range: run concurrently, one test can answer another test's
/// request, so the suite serialises itself like `WC2C`/`WC3` do.
static SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

use bytes::Bytes;
use exyonq_core::AppConfig;
use http_body_util::{BodyExt, Full};
use hyper::{Request, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

fn ephemeral_addr() -> SocketAddr {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

/// Upstream that answers 200 and records the request line it received.
async fn spawn_echo_upstream(seen: Arc<Mutex<Vec<String>>>) -> SocketAddr {
    let listener = TcpListener::bind(ephemeral_addr()).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(line) = request.lines().next() {
                    seen.lock().unwrap().push(line.to_string());
                }
                let body = "upstream-ok";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    addr
}

fn config(root: &std::path::Path, upstream: SocketAddr) -> AppConfig {
    // The `app` route owns its own root so a rewrite target is distinguishable
    // from the catch-all `site` route.
    let app_root = root.join("app");
    let toml = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:0"
routes = ["site", "legacy", "items", "app", "api"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = {root:?}
index = "index.html"

[[route]]
name = "legacy"
match = {{ path = "/legacy" }}
rewrite = "/app/index.html"

[[route]]
name = "items"
match = {{ path = "/items" }}
rewrite = {{ pattern = "^/items/(.*)$", replacement = "/app/$1" }}

[[route]]
name = "app"
match = {{ path = "/app" }}
root = {app_root:?}
index = "index.html"

[[route]]
name = "api"
match = {{ path = "/api" }}
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://{upstream}"
timeout_ms = 2000
"#,
        root = root.display().to_string(),
        app_root = app_root.display().to_string(),
    );
    let mut config = AppConfig::parse_str(&toml).expect("config parses");
    // Default-on WAF would answer before the static sensitivity policy runs.
    config.waf.enabled = false;
    config.http3 = Default::default();
    config
}

async fn spawn(config: AppConfig) -> SocketAddr {
    bootstrap::ensure_integration_modules();
    let listen = ephemeral_addr();
    let mut config = config;
    config.servers[0].listen = listen.to_string();
    tokio::spawn(async move {
        exyonq_core::server::run_on(listen, config).await.unwrap();
    });
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(listen).await.is_ok() {
            return listen;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("server did not start on {listen}");
}

async fn get(
    client: &Client<HttpConnector, Full<Bytes>>,
    listen: SocketAddr,
    path: &str,
) -> (StatusCode, Bytes) {
    let uri: Uri = format!("http://{listen}{path}").parse().expect("uri");
    let req = Request::get(uri)
        .body(Full::new(Bytes::new()))
        .expect("request");
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body)
}

fn fixture_root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".git")).expect(".git dir");
    std::fs::create_dir_all(dir.path().join("app")).expect("app dir");
    std::fs::write(dir.path().join(".env"), "SECRET=1").expect(".env");
    std::fs::write(dir.path().join(".git/config"), "[core]").expect(".git/config");
    std::fs::write(dir.path().join("ok.txt"), "ok").expect("ok");
    std::fs::write(dir.path().join("index.html"), "index").expect("index");
    std::fs::write(dir.path().join("app/index.html"), "rewritten-index").expect("app index");
    // Reached only when a patterned rewrite captured `a%20b` from the request.
    std::fs::write(dir.path().join("app/a b"), "captured").expect("captured file");
    dir
}

/// The sensitive-name gate classifies the canonical path, so every escape
/// spelling of a denied name is denied with the same opaque 404.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn sensitive_names_stay_denied_in_every_spelling() {
    let _gate = SUITE_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = fixture_root();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let upstream = spawn_echo_upstream(Arc::clone(&seen)).await;
    let listen = spawn(config(dir.path(), upstream)).await;
    let client = Client::builder(TokioExecutor::new()).build_http();

    let (status, body) = get(&client, listen, "/ok.txt").await;
    assert_eq!(status, StatusCode::OK, "control: a safe file is served");
    assert_eq!(body.as_ref(), b"ok");

    for spelling in ["/.env", "/%2eenv", "/.%65nv", "/%2E%65%6Ev"] {
        let (status, body) = get(&client, listen, spelling).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{spelling} must stay an opaque 404, body={:?}",
            String::from_utf8_lossy(&body)
        );
        assert!(
            !body.windows(6).any(|w| w == b"SECRET"),
            "{spelling} leaked the denied file"
        );
    }

    for spelling in ["/.git/config", "/%2egit/config", "/.%67it/config"] {
        let (status, body) = get(&client, listen, spelling).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{spelling} must be denied");
        assert!(!body.windows(6).any(|w| w == b"[core]"));
    }
}

/// A literal rewrite is applied to the canonical path and its target is served,
/// whether or not the request carries escapes.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn literal_rewrite_serves_its_target_for_encoded_requests() {
    let _gate = SUITE_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = fixture_root();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let upstream = spawn_echo_upstream(Arc::clone(&seen)).await;
    let listen = spawn(config(dir.path(), upstream)).await;
    let client = Client::builder(TokioExecutor::new()).build_http();

    for path in ["/legacy/anything", "/legacy/a%20b", "/legacy/%61"] {
        let (status, body) = get(&client, listen, path).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{path} must reach the rewrite target, body={:?}",
            String::from_utf8_lossy(&body)
        );
        assert_eq!(body.as_ref(), b"rewritten-index", "{path}");
    }
}

/// A patterned rewrite matches and captures the canonical path; the captured
/// escape survives into the rewritten target and the static resolver decodes it
/// once, so `a%20b` opens the file named `a b`.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn patterned_rewrite_captures_the_canonical_path() {
    let _gate = SUITE_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = fixture_root();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let upstream = spawn_echo_upstream(Arc::clone(&seen)).await;
    let listen = spawn(config(dir.path(), upstream)).await;
    let client = Client::builder(TokioExecutor::new()).build_http();

    let (status, body) = get(&client, listen, "/items/a%20b").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "captured `a%20b` must resolve to `a b`, body={:?}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(body.as_ref(), b"captured");

    // Negative control: the same rule with a name that does not exist is a 404,
    // so the 200 above comes from the file, not from the rewrite alone.
    let (status, _) = get(&client, listen, "/items/missing%20file").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The path forwarded upstream is the canonical URI form: the target keeps its
/// escapes and its query instead of being decoded into an invalid request line.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn proxy_forwards_the_canonical_request_target() {
    let _gate = SUITE_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = fixture_root();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let upstream = spawn_echo_upstream(Arc::clone(&seen)).await;
    let listen = spawn(config(dir.path(), upstream)).await;
    let client = Client::builder(TokioExecutor::new()).build_http();

    let (status, body) = get(&client, listen, "/api/a%20b?q=1").await;
    assert_eq!(status, StatusCode::OK, "proxy path");
    assert_eq!(body.as_ref(), b"upstream-ok");

    let lines = seen.lock().unwrap().clone();
    assert_eq!(lines.len(), 1, "one upstream request: {lines:?}");
    assert!(
        lines[0].starts_with("GET /api/a%20b?q=1 "),
        "upstream request line must keep the canonical target, got {:?}",
        lines[0]
    );
}
