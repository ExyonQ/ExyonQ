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
//! NG-18 / ADR-064 Phase C — sub_filter through the real handler.

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

static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Harness {
    ctx: Arc<ConnectionContext>,
    _root: tempfile::TempDir,
    _fcgi: FcgiDispatchTestGuard,
    _resolver: FastcgiScriptResolverTestGuard,
}

struct Fixed(FcgiDispatchOutcome);
impl FcgiBackendExecutor for Fixed {
    fn dispatch(&self, _r: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        self.0.clone()
    }
}

async fn harness(once: bool) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("public")).unwrap();
    std::fs::write(tmp.path().join("public/index.php"), "<?php\n").unwrap();
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
[[fcgi_pool]]
name = "php"
address = "/tmp/exyonq-fcgi-sub.sock"
document_root = "{root}"
[modules.sub_filter.routes."php"]
once = {once}
last_modified = false
types = ["text/html"]
[[modules.sub_filter.routes."php".rules]]
pattern = "http://"
replacement = "https://"
"#,
        root = tmp.path().display()
    );
    let config: AppConfig = raw.parse().expect("config");
    let resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(51, config, proxy_client.clone())
        .await
        .unwrap();
    let body = b"<html>http://a http://b</html>".to_vec();
    let fcgi = FcgiDispatchTestGuard::install(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(Fixed(FcgiDispatchOutcome::Success(FcgiSuccessResponse {
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/html; charset=utf-8".into()),
                    ("content-length".into(), body.len().to_string()),
                    (
                        "last-modified".into(),
                        "Wed, 21 Oct 2015 07:28:00 GMT".into(),
                    ),
                ],
                body,
            }))),
            pool_capacities: vec![(0, 16)],
        })
        .unwrap(),
    ));
    Harness {
        ctx: Arc::new(ConnectionContext {
            state,
            proxy_client,
            x_forwarded_for: hyper::header::HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        }),
        _root: tmp,
        _fcgi: fcgi,
        _resolver: resolver,
    }
}

async fn get(ctx: &ConnectionContext) -> (StatusCode, hyper::HeaderMap, Vec<u8>) {
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://example.test/public/index.php")
        .header("host", "example.test")
        .body(())
        .unwrap();
    let response = serve_http3_request(ctx.clone(), req).await;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, body.to_vec())
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn sub_filter_replaces_all_and_removes_last_modified() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    let h = harness(false).await;
    let (status, headers, body) = get(&h.ctx).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"<html>https://a https://b</html>");
    assert!(headers.get("last-modified").is_none());
    assert!(headers.get("content-length").is_none());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn sub_filter_once_replaces_first_only() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    let h = harness(true).await;
    let (_, _, body) = get(&h.ctx).await;
    assert_eq!(body, b"<html>https://a http://b</html>");
}
