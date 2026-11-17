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
//! Real control-API purge for paths that carry escapes (WC3 + H-01).
//!
//! The FPC key is built from the canonical request path and the purge key from
//! the operator URL, both through `fpc_canonicalize_path`. This test proves the
//! pair on a running server: a stored entry is served from cache, the purge
//! socket receives the operator's raw spelling, and the next request reaches
//! the origin again. `%2F` keeps failing closed, and purging a URL that was
//! never cached is a successful no-op.

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
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const PURGE_TOKEN: &str = "integration-purge-token";

/// Origin bodies: different lengths, so a cached copy and a fresh one cannot be
/// confused even if a stale file were served.
const ORIGIN_V1: &str = "origin-v1";
const ORIGIN_V2: &str = "origin-v2-after-purge";

fn ephemeral_addr() -> SocketAddr {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

fn config(root: &Path) -> AppConfig {
    let toml = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:0"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = {root:?}

[full_page_cache]
enabled = true
namespace = 4
max_entries = 32
max_total_bytes = 1048576
max_object_bytes = 1048576
default_ttl_seconds = 60
max_ttl_seconds = 300
"#,
        root = root.display().to_string(),
    );
    AppConfig::parse_str(&toml).expect("config parses")
}

async fn spawn(config: AppConfig) -> SocketAddr {
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
    let response = tokio::time::timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body)
}

/// Replace the served object so a fresh origin read is observable, with a new
/// inode and a different length (the static path revalidates on both).
fn write_origin(path: &Path, body: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => panic!("remove {}: {err}", path.display()),
    }
    std::fs::write(path, body).expect("write origin body");
}

/// Wait for the purge socket the server publishes next to its sidecars.
async fn wait_for_purge_socket(socket: &Path) {
    for _ in 0..400 {
        if socket.exists() && socket.join("").exists() {
            return;
        }
        if socket.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("purge socket {} was not created", socket.display());
}

/// Site id published for `route` in the `<socket>.sites` sidecar.
fn published_site_id(socket: &Path, route: &str) -> u64 {
    let mut sidecar = socket.as_os_str().to_owned();
    sidecar.push(".sites");
    let index = std::fs::read_to_string(PathBuf::from(sidecar)).expect("sites sidecar");
    index
        .lines()
        .find_map(|line| {
            let (name, id) = line.split_once('\t')?;
            (name == route).then(|| id.parse::<u64>().expect("site id"))
        })
        .unwrap_or_else(|| panic!("route {route} missing from the published site index"))
}

/// Send one purge line over the real control socket and read its JSON answer.
async fn purge(socket: &Path, line: String) -> serde_json::Value {
    let stream = UnixStream::connect(socket)
        .await
        .expect("connect purge socket");
    let mut reader = BufReader::new(stream);
    reader
        .get_mut()
        .write_all(format!("{line}\n").as_bytes())
        .await
        .expect("write purge line");
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .await
        .expect("read purge response");
    serde_json::from_str(response.trim()).unwrap_or_else(|err| {
        panic!("purge response is not JSON ({err}): {response:?}");
    })
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn control_api_url_purge_evicts_the_entry_for_every_spelling() {
    let _gate = SUITE_GATE.lock().unwrap_or_else(|e| e.into_inner());
    bootstrap::ensure_integration_modules();
    let root = tempfile::tempdir().expect("root");
    let control = tempfile::tempdir().expect("control dir");
    let socket = control.path().join("cache-purge.sock");
    // Env is process-global; this binary hosts a single test.
    std::env::set_var("EXYONQ_CACHE_PURGE_SOCKET", &socket);
    std::env::set_var("EXYONQ_CACHE_PURGE_TOKEN", PURGE_TOKEN);

    std::fs::write(root.path().join("a b.txt"), ORIGIN_V1).expect("space file");
    std::fs::write(root.path().join("ab.txt"), ORIGIN_V1).expect("unreserved file");
    std::fs::write(root.path().join("\u{e9}.txt"), ORIGIN_V1).expect("non-ascii file");
    std::fs::create_dir_all(root.path().join("sub")).expect("sub dir");
    std::fs::write(root.path().join("sub/b.txt"), ORIGIN_V1).expect("nested file");

    let listen = spawn(config(root.path())).await;
    wait_for_purge_socket(&socket).await;
    let site_id = published_site_id(&socket, "site");
    let client = Client::builder(TokioExecutor::new()).build_http();

    // One case per spelling the canonical form can carry: `%20` (a byte a
    // request-target cannot hold literally), `%61` (an escaped unreserved byte)
    // and `%C3%A9` (non-ASCII).
    let cases = [
        ("/a%20b.txt", "a b.txt"),
        ("/%61b.txt", "ab.txt"),
        ("/%C3%A9.txt", "\u{e9}.txt"),
    ];
    for (request_path, file) in cases {
        let (status, body) = get(&client, listen, request_path).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{request_path} must be served from origin first"
        );
        assert_eq!(body.as_ref(), ORIGIN_V1.as_bytes(), "{request_path}");

        write_origin(&root.path().join(file), ORIGIN_V2);
        let (status, body) = get(&client, listen, request_path).await;
        assert_eq!(status, StatusCode::OK, "{request_path}");
        assert_eq!(
            body.as_ref(),
            ORIGIN_V1.as_bytes(),
            "{request_path} must come from the cache before the purge"
        );

        let response = purge(
            &socket,
            format!("purge url {site_id} http 127.0.0.1 {request_path} {PURGE_TOKEN}"),
        )
        .await;
        assert_eq!(
            response["ok"], true,
            "{request_path} purge rejected: {response}"
        );
        assert!(
            response["purged_entries"].as_u64().unwrap_or(0) >= 1,
            "{request_path} purge removed nothing: {response}"
        );

        let (status, body) = get(&client, listen, request_path).await;
        assert_eq!(status, StatusCode::OK, "{request_path}");
        assert_eq!(
            body.as_ref(),
            ORIGIN_V2.as_bytes(),
            "{request_path} must reach the origin again after the purge"
        );
    }

    // Control: an encoded separator is a separator after canonicalization, so
    // the purge spelling must fail closed instead of evicting `/sub/b.txt`.
    let (status, body) = get(&client, listen, "/sub/b.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), ORIGIN_V1.as_bytes());
    write_origin(&root.path().join("sub/b.txt"), ORIGIN_V2);
    let (_, body) = get(&client, listen, "/sub/b.txt").await;
    assert_eq!(
        body.as_ref(),
        ORIGIN_V1.as_bytes(),
        "the canonical entry must be cached before the %2F control"
    );

    let response = purge(
        &socket,
        format!("purge url {site_id} http 127.0.0.1 /sub%2Fb.txt {PURGE_TOKEN}"),
    )
    .await;
    assert_eq!(
        response["ok"], false,
        "%2F must stay rejected, not purge: {response}"
    );
    assert_eq!(
        response["error"], "invalid_key",
        "%2F must be rejected before key resolution: {response}"
    );
    assert_eq!(
        response["purged_entries"].as_u64().unwrap_or(1),
        0,
        "a rejected purge must not remove entries: {response}"
    );
    let (_, body) = get(&client, listen, "/sub/b.txt").await;
    assert_eq!(
        body.as_ref(),
        ORIGIN_V1.as_bytes(),
        "the homonymous canonical entry must survive the %2F purge"
    );

    // Control: purging a URL that was never cached answers an explicit
    // `not_found` no-op (a well-formed response), never an internal error.
    let response = purge(
        &socket,
        format!("purge url {site_id} http 127.0.0.1 /never%20cached.txt {PURGE_TOKEN}"),
    )
    .await;
    assert_eq!(
        response["error"], "not_found",
        "an uncached URL must be reported as a no-op miss: {response}"
    );
    assert_eq!(
        response["purged_entries"].as_u64().unwrap_or(1),
        0,
        "nothing was cached under that URL: {response}"
    );
    assert_eq!(
        response["purged_bytes"].as_u64().unwrap_or(1),
        0,
        "a miss frees no bytes: {response}"
    );
}
