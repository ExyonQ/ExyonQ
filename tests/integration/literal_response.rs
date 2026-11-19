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
//! EXY-03 — literal responses (`NGINX return <status> "<body>"`).
//!
//! These are real HTTP requests against a running server: the assertions are
//! the status line, the body bytes and the `Content-Type` header the client
//! receives, not the config parser's opinion.

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

async fn spawn_server(mut config: AppConfig) -> SocketAddr {
    bootstrap::ensure_integration_modules();
    let listen = ephemeral_addr();
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
) -> hyper::Response<hyper::body::Incoming> {
    let uri: Uri = format!("http://{listen}{path}").parse().unwrap();
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
    timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed")
}

/// A literal route must win over static resolution for the same URI, and it
/// must serve exactly the declared status, body and default content type.
#[tokio::test]
async fn literal_response_beats_the_static_index_and_serves_declared_body() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The static root owns `/` and holds a file with the same name as the
    // literal route, so the only way `/healthz` returns "ok" is the literal
    // response winning before static resolution runs.
    std::fs::write(dir.path().join("index.html"), "from-index").expect("index");
    std::fs::write(dir.path().join("healthz"), "from-disk").expect("healthz file");

    let toml = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:0"
routes = ["root", "health", "teapot"]

[[route]]
name = "root"
match = {{ path = "/" }}
root = {root:?}
index = "index.html"

[[route]]
name = "health"
match = {{ path = "/healthz", exact = true }}
respond = {{ status = 200, body = "ok" }}

[[route]]
name = "teapot"
match = {{ path = "/teapot", exact = true }}
respond = {{ status = 418, body = "short-and-stout", content_type = "application/json" }}
"#,
        root = dir.path().display().to_string()
    );
    let config = AppConfig::parse_str(&toml).expect("config parses with respond");
    let listen = spawn_server(config).await;
    let client = client();

    let response = get(&client, listen, "/healthz").await;
    assert_eq!(response.status(), StatusCode::OK, "declared status");
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .expect("content-type header")
            .to_str()
            .unwrap(),
        "text/plain; charset=utf-8",
        "default content type for a literal body"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), b"ok", "declared body wins over the file");
    assert_ne!(
        body.as_ref(),
        b"from-disk",
        "static resolution must not answer this route"
    );

    let response = get(&client, listen, "/teapot").await;
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .expect("content-type header")
            .to_str()
            .unwrap(),
        "application/json",
        "declared content type is honored"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), b"short-and-stout");

    // The static route still works: the literal route did not take over `/`.
    let response = get(&client, listen, "/").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        body.windows(10).any(|w| w == b"from-index"),
        "static index still served"
    );
}

/// `respond` is exclusive with a backend: the IR must reject a config that
/// declares both, rather than silently choosing one.
#[test]
fn respond_with_a_backend_is_rejected() {
    let toml = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:0"
routes = ["both"]

[[route]]
name = "both"
match = { path = "/" }
root = "/srv/www"
respond = { status = 200, body = "ok" }
"#;
    let err = AppConfig::parse_str(toml).expect_err("respond + root must be rejected");
    let message = err.to_string();
    assert!(
        message.contains("both") && message.contains("conflicting"),
        "unexpected error: {message}"
    );
}

/// A literal body is a body: the bodyless statuses are refused with a clear
/// message instead of sending something clients strip.
#[test]
fn respond_with_a_bodyless_status_is_rejected() {
    let toml = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:0"
routes = ["empty"]

[[route]]
name = "empty"
match = { path = "/empty", exact = true }
respond = { status = 204, body = "nope" }
"#;
    let err = AppConfig::parse_str(toml).expect_err("204 with a body must be rejected");
    let message = err.to_string();
    assert!(
        message.contains("respond.status"),
        "unexpected error: {message}"
    );
}
