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
//! Request-target representation over real HTTP.
//!
//! A request-target is opaque bytes, as in nginx. `/%20` is a legal escape and
//! must reach the route (here: a file whose name contains a space), not answer
//! 400. These tests use a real server and assert the status and body the client
//! receives, including that a percent-encoded spelling and its already-supported
//! equivalent return the same thing.

mod bootstrap;

use bytes::Bytes;
use exyonq_core::AppConfig;
use http_body_util::{BodyExt, Full};
use hyper::{Request, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::time::Duration;
use tokio::time::timeout;

fn ephemeral_addr() -> SocketAddr {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

async fn spawn(config: AppConfig) -> SocketAddr {
    bootstrap::ensure_integration_modules();
    let listen = ephemeral_addr();
    let mut config = config;
    config.servers[0].listen = listen.to_string();
    tokio::spawn(async move {
        exyonq_core::server::run_on(listen, config).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    listen
}

fn client() -> Client<HttpConnector, Full<Bytes>> {
    Client::builder(TokioExecutor::new()).build_http()
}

async fn get(
    client: &Client<HttpConnector, Full<Bytes>>,
    listen: SocketAddr,
    path: &str,
) -> (StatusCode, Bytes) {
    let uri: Uri = format!("http://{listen}{path}").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body)
}

fn site_config(root: &std::path::Path) -> AppConfig {
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
index = "index.html"
"#,
        root = root.display().to_string()
    );
    AppConfig::parse_str(&toml).expect("site config parses")
}

/// `%20` and `(` are path bytes. Both spellings must reach the same file and
/// return the same status and body as the form that already worked, instead of
/// 400 from the URI rebuild.
#[tokio::test]
async fn percent_encoded_path_bytes_reach_the_same_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("hello world.html"), "space-ok").expect("space file");
    std::fs::write(dir.path().join("fn().html"), "paren-ok").expect("paren file");
    std::fs::write(dir.path().join("plain.html"), "plain-ok").expect("plain file");
    let listen = spawn(site_config(dir.path())).await;
    let client = client();

    // The already-supported form: an unescaped path with no special bytes.
    let (status, body) = get(&client, listen, "/plain.html").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), b"plain-ok");

    // `(` is a sub-delim: legal literally, so both spellings must agree.
    let (literal_status, literal_body) = get(&client, listen, "/fn().html").await;
    let (encoded_status, encoded_body) = get(&client, listen, "/fn%28%29.html").await;
    assert_eq!(literal_status, StatusCode::OK, "literal `(` path");
    assert_eq!(
        encoded_status, literal_status,
        "`%28`/`%29` must not change the outcome"
    );
    assert_eq!(encoded_body, literal_body);
    assert_eq!(literal_body.as_ref(), b"paren-ok");

    // `%20` has no literal spelling; it must open the file with the space in
    // its name. An encoded unreserved byte (`%68` = 'h') plus `%20` is the
    // equivalent already-supported spelling for the same target.
    let (status, body) = get(&client, listen, "/hello%20world.html").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "`%20` must reach the file, not answer 400: body={:?}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(body.as_ref(), b"space-ok");

    let (status, body) = get(&client, listen, "/%68ello%20world.html").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), b"space-ok", "same target, same body");

    // A missing name with a space is a 404, not a 400.
    let (status, _) = get(&client, listen, "/hello%20missing.html").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Traversal stays refused after decoding, in both spellings.
    let (status, _) = get(&client, listen, "/%2e%2e/plain.html").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
