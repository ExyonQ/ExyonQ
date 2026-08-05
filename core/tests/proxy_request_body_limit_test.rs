//! SEC-PROXY-001 — proxy request body limit order (live path + side effects).
//!
//! Unit tests for the bounded collector live in
//! `core/src/server/proxy_request_body.rs` (injectable small limits).
//! This file covers serve_connection wiring: oversized bodies must not
//! call proxy dispatch / upstream.

use async_trait::async_trait;
use exyonq_core::execute_backend::ProxyDispatchTestGuard;
use exyonq_core::server::handler::{serve_connection, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_core::{
    clear_global_proxy_dispatch_for_register_once_test, register_proxy_dispatch_service, AppConfig,
};
use exyonq_mod_proxy::{build_incoming_client, install_kernel_hooks, ProxyRuntime};
use exyonq_module_api::proxy_dispatch::{
    ProxyDispatchOutcome, ProxyDispatchRequest, ProxyDispatchService, ProxyMaterializedResponse,
    PROXY_MAX_REQUEST_BODY_BYTES,
};
use hyper::header::HeaderValue;
use hyper::StatusCode;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

static PROXY_BODY_LIMIT_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct CountingProxy {
    dispatches: AtomicUsize,
}

#[async_trait]
impl ProxyDispatchService for CountingProxy {
    async fn dispatch(&self, request: ProxyDispatchRequest) -> ProxyDispatchOutcome {
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        ProxyDispatchOutcome::Materialized(ProxyMaterializedResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: format!("ok:{}", request.body.as_ref().map(|b| b.len()).unwrap_or(0))
                .into_bytes(),
        })
    }
}

async fn proxy_ctx(counting: Arc<CountingProxy>) -> (ConnectionContext, ProxyDispatchTestGuard) {
    let raw = include_str!("../../tests/fixtures/minimal.toml");
    let config: AppConfig = raw.parse().expect("minimal.toml");
    let proxy_runtime = Arc::new(ProxyRuntime::new());
    install_kernel_hooks(Arc::clone(&proxy_runtime));
    clear_global_proxy_dispatch_for_register_once_test();
    let service: Arc<dyn ProxyDispatchService> = counting;
    let _ = register_proxy_dispatch_service(Arc::clone(&service));
    let guard = ProxyDispatchTestGuard::install(service);
    let proxy_client = build_incoming_client();
    let state = ServerState::new_with_generation(1, config, proxy_client.clone())
        .await
        .expect("state");
    (
        ConnectionContext {
            state,
            proxy_client,
            x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        guard,
    )
}

async fn live_post(
    ctx: ConnectionContext,
    content_length: Option<usize>,
    body: &[u8],
    transfer_encoding_chunked: bool,
) -> u16 {
    let (mut client_io, server_io) = duplex(64 * 1024);
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

    let mut request = String::from("POST /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if transfer_encoding_chunked {
        request.push_str("Transfer-Encoding: chunked\r\n");
    } else if let Some(len) = content_length {
        request.push_str(&format!("Content-Length: {len}\r\n"));
    } else {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("Connection: close\r\n\r\n");
    client_io
        .write_all(request.as_bytes())
        .await
        .expect("write headers");
    if transfer_encoding_chunked {
        // Single chunk + terminator (chunked equivalent of the body).
        let chunk_hdr = format!("{:x}\r\n", body.len());
        client_io
            .write_all(chunk_hdr.as_bytes())
            .await
            .expect("chunk hdr");
        client_io.write_all(body).await.expect("chunk body");
        client_io
            .write_all(b"\r\n0\r\n\r\n")
            .await
            .expect("chunk end");
    } else {
        client_io.write_all(body).await.expect("write body");
    }

    let mut response = Vec::new();
    client_io
        .read_to_end(&mut response)
        .await
        .expect("read response");
    let _ = server.await;

    String::from_utf8_lossy(&response)
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("HTTP status line")
}

#[tokio::test]
async fn oversized_content_length_rejects_without_dispatch() {
    let _gate = PROXY_BODY_LIMIT_GATE.lock().await;
    let counting = Arc::new(CountingProxy {
        dispatches: AtomicUsize::new(0),
    });
    let (ctx, _guard) = proxy_ctx(Arc::clone(&counting)).await;
    let status = live_post(
        ctx,
        Some(PROXY_MAX_REQUEST_BODY_BYTES + 1),
        &[], // body unread after early CL reject
        false,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY.as_u16());
    assert_eq!(
        counting.dispatches.load(Ordering::SeqCst),
        0,
        "oversized body must not call proxy dispatch / upstream"
    );
}

#[tokio::test]
async fn valid_small_post_still_dispatches() {
    let _gate = PROXY_BODY_LIMIT_GATE.lock().await;
    let counting = Arc::new(CountingProxy {
        dispatches: AtomicUsize::new(0),
    });
    let (ctx, _guard) = proxy_ctx(Arc::clone(&counting)).await;
    let body = b"hello-proxy";
    let status = live_post(ctx, Some(body.len()), body, false).await;
    assert_eq!(status, StatusCode::OK.as_u16());
    assert_eq!(counting.dispatches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn chunked_valid_body_still_dispatches() {
    let _gate = PROXY_BODY_LIMIT_GATE.lock().await;
    let counting = Arc::new(CountingProxy {
        dispatches: AtomicUsize::new(0),
    });
    let (ctx, _guard) = proxy_ctx(Arc::clone(&counting)).await;
    // Productive 32 MiB+1 chunked oversize would allocate heavily; unit tests cover
    // incremental oversize without CL. Live path verifies chunked still dispatches.
    let body = b"chunked-ok";
    let status = live_post(ctx, None, body, true).await;
    assert_eq!(status, StatusCode::OK.as_u16());
    assert_eq!(counting.dispatches.load(Ordering::SeqCst), 1);
}

#[test]
fn handler_uses_bounded_collector_not_unbounded_collect() {
    let handler = include_str!("../src/server/handler.rs");
    let section = handler
        .split("async fn proxy_route_target")
        .nth(1)
        .and_then(|rest| rest.split("\nasync fn ").next())
        .expect("proxy_route_target");
    assert!(
        section.contains("collect_proxy_request_body_bounded"),
        "proxy_route_target must use bounded collector"
    );
    assert!(
        !section.contains(".collect()"),
        "proxy_route_target must not BodyExt::collect the request body"
    );
    assert!(
        section.contains("PROXY_MAX_REQUEST_BODY_BYTES"),
        "must enforce productive 32 MiB constant"
    );
    assert!(
        section.contains("BadGateway"),
        "must preserve 502 rejection semantics"
    );
}

/*
TEST_GAP / TOOL_LIMITATION (this phase):
- HTTP/2 live harness for request body oversize: not available in this suite.
- Cancellation mid-read without upstream contact: not instrumented (no cancel hook).
- Physical 32 MiB + 1 incremental allocation test: covered via injectable-limit unit
  tests; productive CL early-reject covers live 32 MiB + 1 without allocating body.
- Cache insertion count: N/A for POST oversize (no cache path before dispatch).
- Retry/replay: N/A (no retry implementation).
*/
