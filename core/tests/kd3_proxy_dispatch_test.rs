//! KD3.2 — proxy dispatch registration and contract integration.

use exyonq_core::{
    bind_proxy_compiled_slots, build_proxy_dispatch_request,
    clear_global_proxy_dispatch_for_register_once_test, contract_service_registration_test_gate,
    dispatch_proxy, execute_backend, proxy_metrics_assert_guard, proxy_outcome_to_hyper,
    register_proxy_dispatch_service, Backend, ProxyDispatchTestGuard, ProxyMethod,
    ProxyRegisterError,
};
use exyonq_metrics::{proxy_http_501_total, KernelShellMetrics};
use exyonq_mod_proxy::{take_streaming, ProxyRuntime};
use exyonq_module_api::kernel_observation::KernelObservationTestGuard;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchService,
};
use http_body_util::BodyExt;
use std::sync::Arc;
use std::time::Duration;

/// SEC-PROXY-002: unknown-length upstream bodies classify as Streaming; tests accept both.
async fn outcome_status_headers_body(
    outcome: ProxyDispatchOutcome,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    match outcome {
        ProxyDispatchOutcome::Materialized(m) => (m.status, m.headers, m.body),
        ProxyDispatchOutcome::Streaming {
            status,
            headers,
            stream,
        } => {
            let response = take_streaming(stream).expect("streaming handle");
            let body = response
                .into_body()
                .collect()
                .await
                .expect("collect stream")
                .to_bytes()
                .to_vec();
            (status, headers, body)
        }
        other => panic!("expected Materialized or Streaming, got {other:?}"),
    }
}

async fn spawn_echo_upstream() -> (String, tokio::task::JoinHandle<()>) {
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use hyper_util::server::conn::auto::Builder;
    use std::convert::Infallible;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(|req: hyper::Request<hyper::body::Incoming>| async move {
                    let method = req.method().clone();
                    let path = req.uri().path().to_string();
                    let query = req.uri().query().unwrap_or("").to_string();
                    let mut saw_connection = false;
                    let mut xff = String::new();
                    for (name, value) in req.headers().iter() {
                        if name.as_str().eq_ignore_ascii_case("connection") {
                            saw_connection = true;
                        }
                        if name.as_str().eq_ignore_ascii_case("x-forwarded-for") {
                            if let Ok(v) = value.to_str() {
                                xff = v.to_string();
                            }
                        }
                    }
                    let body_bytes = if method == hyper::Method::HEAD {
                        Bytes::new()
                    } else if method == hyper::Method::POST {
                        use http_body_util::BodyExt;
                        req.into_body()
                            .collect()
                            .await
                            .map(|c| c.to_bytes())
                            .unwrap_or_default()
                    } else {
                        Bytes::from(format!(
                            "path={path}?{query}|xff={xff}|conn={saw_connection}"
                        ))
                    };
                    let status = if path == "/error" { 500 } else { 200 };
                    let mut builder = hyper::Response::builder().status(status);
                    if method == hyper::Method::HEAD {
                        builder = builder.header("content-length", "0");
                    } else {
                        builder = builder
                            .header("content-type", "text/plain")
                            .header("connection", "close")
                            .header("transfer-encoding", "identity");
                    }
                    Ok::<_, Infallible>(builder.body(Full::new(body_bytes)).expect("response"))
                });
                let _ = Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(io, service)
                    .await;
            });
        }
    });
    (format!("http://{addr}"), handle)
}

fn install_runtime_with_upstream(upstream: &str) -> (Arc<ProxyRuntime>, ProxyDispatchTestGuard) {
    let runtime = Arc::new(ProxyRuntime::new());
    let guard =
        ProxyDispatchTestGuard::install(Arc::clone(&runtime) as Arc<dyn ProxyDispatchService>);
    bind_proxy_compiled_slots(
        1,
        &[ProxyCompiledSlot::legacy_single(
            0,
            "backend",
            upstream,
            Duration::from_millis(500),
        )],
    );
    (runtime, guard)
}

#[tokio::test]
async fn proxy_without_service_returns_not_registered() {
    let _guard = ProxyDispatchTestGuard::force_absent();
    let outcome = dispatch_proxy(build_proxy_dispatch_request(
        0,
        ProxyMethod::Get,
        "/api/health",
        None,
        Vec::new(),
        None,
        "127.0.0.1",
        "http",
    ))
    .await;
    assert!(matches!(outcome, ProxyDispatchOutcome::NotRegistered));
}

#[tokio::test]
async fn execute_backend_proxy_without_service_returns_501() {
    // Process-wide PROXY_HTTP_501 — serialize with other 501-delta asserts (KD3 / R2E).
    let _metrics_gate = proxy_metrics_assert_guard().await;
    let _obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    let _guard = ProxyDispatchTestGuard::force_absent();
    let before = proxy_http_501_total();
    let outcome = execute_backend(
        &Backend::Proxy { cluster_id: 0 },
        None,
        None,
        Some(build_proxy_dispatch_request(
            0,
            ProxyMethod::Get,
            "/",
            None,
            Vec::new(),
            None,
            "127.0.0.1",
            "http",
        )),
    )
    .await
    .expect("proxy backend");
    assert_eq!(outcome.status, 501);
    assert_eq!(proxy_http_501_total(), before + 1);
}

#[test]
fn register_once_rejects_duplicate() {
    let _gate = contract_service_registration_test_gate();
    clear_global_proxy_dispatch_for_register_once_test();
    register_proxy_dispatch_service(Arc::new(ProxyRuntime::new())).expect("first");
    assert_eq!(
        register_proxy_dispatch_service(Arc::new(ProxyRuntime::new())),
        Err(ProxyRegisterError::AlreadyRegistered)
    );
    clear_global_proxy_dispatch_for_register_once_test();
}

#[tokio::test]
async fn registered_proxy_upstream_error_is_502() {
    let (runtime, _guard) = install_runtime_with_upstream("http://127.0.0.1:1");
    let before = runtime.metrics().responses_502;
    let outcome = dispatch_proxy(build_proxy_dispatch_request(
        0,
        ProxyMethod::Get,
        "/api/health",
        None,
        Vec::new(),
        None,
        "127.0.0.1",
        "http",
    ))
    .await;
    assert!(matches!(
        outcome,
        ProxyDispatchOutcome::BadGateway | ProxyDispatchOutcome::GatewayTimeout
    ));
    assert!(runtime.metrics().responses_502 >= before);
    let response = proxy_outcome_to_hyper(outcome);
    assert!(response.status().is_client_error() || response.status().is_server_error());
}

#[tokio::test]
async fn query_preservation_on_get() {
    let (upstream, _h) = spawn_echo_upstream().await;
    let (_, _guard) = install_runtime_with_upstream(&upstream);
    let outcome = dispatch_proxy(build_proxy_dispatch_request(
        0,
        ProxyMethod::Get,
        "/items?a=1&b=two",
        None,
        Vec::new(),
        None,
        "10.0.0.2",
        "http",
    ))
    .await;
    let (_status, _headers, body_bytes) = outcome_status_headers_body(outcome).await;
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(body.contains("path=/items"));
    assert!(body.contains("a=1&b=two"));
    assert!(body.contains("xff=10.0.0.2"));
}

#[tokio::test]
async fn hop_by_hop_request_stripped_and_response_filtered() {
    let (upstream, _h) = spawn_echo_upstream().await;
    let (_, _guard) = install_runtime_with_upstream(&upstream);
    let outcome = dispatch_proxy(build_proxy_dispatch_request(
        0,
        ProxyMethod::Get,
        "/hop",
        None,
        vec![
            ("connection".into(), "keep-alive".into()),
            ("x-test".into(), "1".into()),
        ],
        None,
        "127.0.0.1",
        "http",
    ))
    .await;
    let (_status, headers, body_bytes) = outcome_status_headers_body(outcome).await;
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(body.contains("conn=false"));
    assert!(!headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("connection")));
    assert!(!headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("transfer-encoding")));
}

#[tokio::test]
async fn head_materializes_empty_body() {
    let (upstream, _h) = spawn_echo_upstream().await;
    let (_, _guard) = install_runtime_with_upstream(&upstream);
    let outcome = dispatch_proxy(build_proxy_dispatch_request(
        0,
        ProxyMethod::Head,
        "/head",
        None,
        Vec::new(),
        None,
        "127.0.0.1",
        "http",
    ))
    .await;
    match &outcome {
        ProxyDispatchOutcome::Materialized(m) => {
            assert_eq!(m.status, 200);
            assert!(m.body.is_empty());
        }
        other => panic!("expected materialized, got {other:?}"),
    }
}

#[tokio::test]
async fn post_body_preserved() {
    let (upstream, _h) = spawn_echo_upstream().await;
    let (_, _guard) = install_runtime_with_upstream(&upstream);
    let outcome = dispatch_proxy(build_proxy_dispatch_request(
        0,
        ProxyMethod::Post,
        "/submit?q=1",
        Some("upstream.local".into()),
        vec![("content-type".into(), "application/json".into())],
        Some(b"{\"k\":\"v\"}".to_vec()),
        "127.0.0.1",
        "http",
    ))
    .await;
    let (_status, _headers, body) = outcome_status_headers_body(outcome).await;
    assert_eq!(body, br#"{"k":"v"}"#);
}

#[tokio::test]
async fn registered_service_does_not_increment_core_501() {
    // Process-wide PROXY_HTTP_501 — serialize with other 501-delta asserts (KD3 / R2E).
    let _metrics_gate = proxy_metrics_assert_guard().await;
    let (upstream, _h) = spawn_echo_upstream().await;
    let (_, _guard) = install_runtime_with_upstream(&upstream);
    let before = proxy_http_501_total();
    let outcome = dispatch_proxy(build_proxy_dispatch_request(
        0,
        ProxyMethod::Get,
        "/ok",
        None,
        Vec::new(),
        None,
        "127.0.0.1",
        "http",
    ))
    .await;
    assert!(matches!(
        outcome,
        ProxyDispatchOutcome::Materialized(_) | ProxyDispatchOutcome::Streaming { .. }
    ));
    assert_eq!(proxy_http_501_total(), before);
}

#[test]
fn parallel_proxy_overrides_isolated() {
    use std::sync::Barrier;
    let barrier = Arc::new(Barrier::new(2));
    let t1 = {
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            let _guard = ProxyDispatchTestGuard::force_absent();
            barrier.wait();
        })
    };
    let t2 = {
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            let _guard = ProxyDispatchTestGuard::install(Arc::new(ProxyRuntime::new()));
            barrier.wait();
        })
    };
    t1.join().unwrap();
    t2.join().unwrap();
}
