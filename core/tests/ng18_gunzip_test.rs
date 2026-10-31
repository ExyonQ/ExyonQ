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
//! NG-18 / ADR-064 Phase B — gunzip response filter through the real handler.

use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{
    AppConfig, FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchTestGuard,
    FcgiRuntimeRegistration, FcgiSuccessResponse,
};
use exyonq_mod_fastcgi::FcgiRuntime;
use exyonq_module_api::fcgi_dispatch::FcgiDispatchRequest;
use exyonq_module_api::fcgi_script_resolver::FastcgiScriptResolverTestGuard;
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode};
use std::sync::Arc;

static NG18_SUITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Harness {
    ctx: Arc<ConnectionContext>,
    _root: tempfile::TempDir,
    _fcgi_guard: FcgiDispatchTestGuard,
    _script_resolver: FastcgiScriptResolverTestGuard,
}

struct FixedExecutor {
    response: FcgiDispatchOutcome,
}

impl FcgiBackendExecutor for FixedExecutor {
    fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        self.response.clone()
    }
}

fn gzip(data: &[u8]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

async fn harness(route_gunzip: Option<bool>, global: bool) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("public")).expect("mkdir");
    std::fs::write(tmp.path().join("public/index.php"), "<?php\n").expect("index.php");
    let route_override = route_gunzip
        .map(|v| format!("gunzip = {v}\n"))
        .unwrap_or_default();
    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[route]]
name = "php"
match = {{ path = "/public/" }}
fastcgi = "php"
{route_override}
[[fcgi_pool]]
name = "php"
address = "/tmp/exyonq-fcgi-ng18.sock"
document_root = "{root}"

[modules.gunzip]
enabled = {global}
"#,
        root = tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let script_resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(41, config, proxy_client.clone())
        .await
        .expect("state");
    let plain = b"ng18-gunzip-fastcgi-body\n".repeat(128);
    let body = gzip(&plain);
    let fcgi_guard = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(FixedExecutor {
                response: FcgiDispatchOutcome::Success(FcgiSuccessResponse {
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/plain".into()),
                        ("content-encoding".into(), "gzip".into()),
                        ("content-length".into(), body.len().to_string()),
                        ("etag".into(), "\"ng18-v1\"".into()),
                    ],
                    body,
                }),
            }),
            pool_capacities: vec![(0, 16)],
        })
        .expect("fcgi runtime"),
    ));
    Harness {
        ctx: Arc::new(ConnectionContext {
            state,
            proxy_client,
            x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        }),
        _root: tmp,
        _fcgi_guard: fcgi_guard,
        _script_resolver: script_resolver,
    }
}

async fn get(
    ctx: &ConnectionContext,
    accept_encoding: Option<&str>,
    method: Method,
) -> (StatusCode, hyper::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri("http://example.test/public/index.php")
        .header("host", "example.test");
    if let Some(ae) = accept_encoding {
        builder = builder.header("accept-encoding", ae);
    }
    let req = builder.body(()).expect("req");
    let response = serve_http3_request(ctx.clone(), req).await;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.expect("collect");
    (status, headers, body.to_bytes().to_vec())
}

fn plain() -> Vec<u8> {
    b"ng18-gunzip-fastcgi-body\n".repeat(128)
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn gunzip_decompresses_only_when_client_does_not_accept_gzip() {
    let _gate = NG18_SUITE_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let h = harness(None, true).await;

    for ae in [None, Some("identity"), Some("gzip;q=0"), Some("*")] {
        let (status, headers, body) = get(&h.ctx, ae, Method::GET).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            headers.get("content-encoding").is_none(),
            "AE={ae:?} must be decompressed"
        );
        assert_eq!(body, plain(), "AE={ae:?}");
        assert!(
            headers
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("W/")),
            "gunzip must weaken ETag"
        );
    }

    for ae in [Some("gzip"), Some("br, gzip;q=0.5")] {
        let (status, headers, body) = get(&h.ctx, ae, Method::GET).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers
                .get("content-encoding")
                .and_then(|v| v.to_str().ok()),
            Some("gzip"),
            "AE={ae:?} keeps gzip"
        );
        assert_eq!(&body[..2], &[0x1f, 0x8b], "AE={ae:?} keeps gzip bytes");
        assert_eq!(
            headers.get("etag").and_then(|v| v.to_str().ok()),
            Some("\"ng18-v1\"")
        );
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn gunzip_head_never_transforms_body_or_encoding() {
    let _gate = NG18_SUITE_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let h = harness(None, true).await;
    let (status, headers, body) = get(&h.ctx, Some("identity"), Method::HEAD).await;
    assert_eq!(status, StatusCode::OK);
    // The executor echoes a body for HEAD; the filter must leave it untouched.
    assert_eq!(&body[..2], &[0x1f, 0x8b]);
    // nginx strips the encoding headers on HEAD but has no body to decode.
    assert!(headers.get("content-encoding").is_none());
    assert!(headers.get("content-length").is_none());
    assert!(headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("W/")));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn gunzip_route_override_wins_over_global() {
    let _gate = NG18_SUITE_GATE.lock().unwrap_or_else(|e| e.into_inner());
    // global on, route off -> keep gzip.
    let h = harness(Some(false), true).await;
    let (_, headers, body) = get(&h.ctx, Some("identity"), Method::GET).await;
    assert_eq!(
        headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok()),
        Some("gzip")
    );
    assert_eq!(&body[..2], &[0x1f, 0x8b]);
    drop(h);

    // global off, route on -> decompress.
    let h = harness(Some(true), false).await;
    let (_, headers, body) = get(&h.ctx, Some("identity"), Method::GET).await;
    assert!(headers.get("content-encoding").is_none());
    assert_eq!(body, plain());
}
