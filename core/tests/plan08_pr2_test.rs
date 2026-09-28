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
//! Plan 08 PR2-A — live FastCGI route returns 501 via `execute_backend` contract path.
use exyonq_core::server::handler::{serve_connection, serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::AppConfig;
use exyonq_metrics::{fcgi_responses_501_total, KernelShellMetrics};
use exyonq_module_api::kernel_observation::KernelObservationTestGuard;
use hyper::header::HeaderValue;
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use std::sync::Arc;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

static FCGI_501_TEST_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn fcgi_501_test_gate() -> (
    tokio::sync::MutexGuard<'static, ()>,
    KernelObservationTestGuard,
) {
    let lock = FCGI_501_TEST_GATE.lock().await;
    let obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    (lock, obs)
}

async fn plan08_ctx() -> ConnectionContext {
    use std::sync::OnceLock;
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.php"), b"<?php").expect("script");
        std::env::set_var("EXYONQ_FCGI_DOCUMENT_ROOT", dir.path());
        dir
    });
    let raw = include_str!("../../scripts/architecture/fixtures/plan08/minimal-fcgi.toml");
    let config: AppConfig = raw.parse().expect("plan08 fixture");
    let proxy_client = exyonq_mod_proxy::build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("server state");
    ConnectionContext {
        state,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
        ops: exyonq_core::lifecycle::LifecycleState::new(),
    }
}

#[tokio::test]
async fn live_fastcgi_get_returns_501_with_metric() {
    let (_gate, _obs) = fcgi_501_test_gate().await;
    let before = fcgi_responses_501_total();
    let ctx = plan08_ctx().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/index.php")
        .body(())
        .expect("request");
    let response = serve_http3_request(ctx, req).await;
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(fcgi_responses_501_total(), before + 1);
}

/// End-to-end POST through `serve_connection` → `handle_core_request_with_body` (Hyper path).
async fn live_post_status(ctx: ConnectionContext) -> u16 {
    let (mut client_io, server_io) = duplex(8192);
    let server_ctx = ConnectionContext {
        state: Arc::clone(&ctx.state),
        proxy_client: ctx.proxy_client.clone(),
        x_forwarded_for: ctx.x_forwarded_for.clone(),
        ops: Arc::clone(&ctx.ops),
    };

    let server = tokio::spawn(async move {
        let service = hyper::service::service_fn(move |req| {
            let server_ctx = ConnectionContext {
                state: Arc::clone(&server_ctx.state),
                proxy_client: server_ctx.proxy_client.clone(),
                x_forwarded_for: server_ctx.x_forwarded_for.clone(),
                ops: Arc::clone(&server_ctx.ops),
            };
            async move { serve_connection(server_ctx, req).await }
        });
        let mut builder = ServerBuilder::new(TokioExecutor::new());
        builder.http1().keep_alive(false);
        builder
            .serve_connection(TokioIo::new(server_io), service)
            .await
    });

    let request = "POST /index.php HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    client_io
        .write_all(request.as_bytes())
        .await
        .expect("write POST");

    let mut response = Vec::new();
    client_io
        .read_to_end(&mut response)
        .await
        .expect("read response");
    server.await.expect("server task").expect("serve");

    String::from_utf8_lossy(&response)
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("HTTP status line")
}

#[tokio::test]
async fn live_fastcgi_post_returns_501() {
    let (_gate, _obs) = fcgi_501_test_gate().await;
    let ctx = plan08_ctx().await;
    assert_eq!(
        live_post_status(ctx).await,
        StatusCode::NOT_IMPLEMENTED.as_u16()
    );
}

#[test]
fn pr2_a_wiring_limited_to_handler_dispatch() {
    let handler = include_str!("../src/server/handler.rs");
    assert!(
        handler.contains("try_fastcgi_contract_backend_response"),
        "PR2-A FastCGI contract dispatch helper must live in handler.rs"
    );
    assert!(
        handler.contains("Backend::Fastcgi { pool_id }"),
        "PR2-A runtime helper must match Backend::Fastcgi explicitly"
    );
    assert!(
        !handler.contains("Backend::Module { .. }"),
        "PR2-A must not wire Backend::Module in handler hot path"
    );
    assert!(
        handler.contains("execute_backend::execute_backend"),
        "PR2-A must call existing execute_backend FastCGI contract path"
    );
    let server_mod = include_str!("../src/server/mod.rs");
    assert!(
        !server_mod.contains("execute_backend"),
        "PR2-A must not wire execute_backend in server/mod.rs"
    );
}

#[test]
fn pr2_a_fastcgi_helper_is_not_generic_contract_executor() {
    let handler = include_str!("../src/server/handler.rs");
    let helper = handler
        .split("async fn execute_fastcgi_backend")
        .nth(1)
        .and_then(|rest| rest.split("\nasync fn ").next())
        .expect("execute_fastcgi_backend body");
    assert!(
        helper.contains("match backend"),
        "helper must branch on backend variant"
    );
    assert!(
        helper.contains("Backend::Fastcgi { pool_id }"),
        "only Backend::Fastcgi may invoke execute_backend from handler"
    );
    assert!(
        !helper.contains("Backend::Module"),
        "Backend::Module must remain unwired in PR2-A runtime"
    );
}
