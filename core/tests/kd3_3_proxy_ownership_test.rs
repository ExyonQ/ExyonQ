//! KD3.3 — Hyper ownership closure: adapter mechanics, reload binding, shim delegation.

use exyonq_core::{
    bind_proxy_compiled_slots, build_proxy_dispatch_request, dispatch_proxy,
    proxy_outcome_to_hyper, register_proxy_dispatch_service, ProxyDispatchTestGuard, ProxyMethod,
    ProxyRegisterError,
};
use exyonq_mod_proxy::ProxyRuntime;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchService, ProxyMaterializedResponse,
};
use http_body_util::BodyExt;
use hyper::StatusCode;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn proxy_outcome_to_hyper_materialized_body_exact() {
    let outcome = ProxyDispatchOutcome::Materialized(ProxyMaterializedResponse {
        status: 200,
        headers: vec![("content-type".into(), "application/octet-stream".into())],
        body: vec![0, 1, 2, 3],
    });
    let response = proxy_outcome_to_hyper(outcome);
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("x-test").map(|_| ()), // absent — adapter must not add policy headers
        None
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], &[0, 1, 2, 3]);
}

#[tokio::test]
async fn proxy_outcome_to_hyper_materialized_status_and_headers() {
    let outcome = ProxyDispatchOutcome::Materialized(ProxyMaterializedResponse {
        status: 418,
        headers: vec![
            ("x-test".into(), "1".into()),
            ("content-type".into(), "text/plain".into()),
        ],
        body: b"teapot".to_vec(),
    });
    let response = proxy_outcome_to_hyper(outcome);
    assert_eq!(response.status(), StatusCode::from_u16(418).unwrap());
    assert_eq!(
        response
            .headers()
            .get("x-test")
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/plain")
    );
}

#[tokio::test]
async fn proxy_outcome_to_hyper_head_empty_body() {
    let outcome = ProxyDispatchOutcome::Materialized(ProxyMaterializedResponse {
        status: 200,
        headers: vec![("content-length".into(), "0".into())],
        body: Vec::new(),
    });
    let response = proxy_outcome_to_hyper(outcome);
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn proxy_outcome_to_hyper_not_registered_501() {
    let response = proxy_outcome_to_hyper(ProxyDispatchOutcome::NotRegistered);
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"not implemented");
}

#[test]
fn kd3_5_proxy_cache_and_streaming_owned_by_module() {
    assert!(
        !std::path::Path::new("src/proxy/mod.rs").exists(),
        "core proxy shim removed in KD3.5"
    );
    let handler = std::fs::read_to_string("src/server/handler.rs").expect("handler");
    assert!(handler.contains("load_get_for_cache"));
    assert!(!handler.contains("proxy::forward_get_streaming"));
}

#[tokio::test]
async fn reload_rebinds_compiled_slots_without_reregistering_service() {
    let runtime = Arc::new(ProxyRuntime::new());
    let _guard = ProxyDispatchTestGuard::install(runtime.clone());
    bind_proxy_compiled_slots(
        1,
        &[ProxyCompiledSlot {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: "http://127.0.0.1:1".into(),
            timeout: Duration::from_millis(100),
        }],
    );
    bind_proxy_compiled_slots(
        2,
        &[ProxyCompiledSlot {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: "http://127.0.0.1:1".into(),
            timeout: Duration::from_millis(200),
        }],
    );
    let outcome = dispatch_proxy(build_proxy_dispatch_request(
        0,
        ProxyMethod::Get,
        "/",
        None,
        Vec::new(),
        None,
        "127.0.0.1",
        "http",
    ))
    .await;
    assert!(
        matches!(
            outcome,
            ProxyDispatchOutcome::BadGateway | ProxyDispatchOutcome::GatewayTimeout
        ),
        "rebound slot still dispatches through same service: {outcome:?}"
    );
}

#[tokio::test]
async fn module_metrics_not_double_counted_on_adapter() {
    let runtime = Arc::new(ProxyRuntime::new());
    let _guard =
        ProxyDispatchTestGuard::install(Arc::clone(&runtime) as Arc<dyn ProxyDispatchService>);
    bind_proxy_compiled_slots(
        1,
        &[ProxyCompiledSlot {
            cluster_id: 0,
            upstream_name: "backend".into(),
            target: "http://127.0.0.1:1".into(),
            timeout: Duration::from_millis(50),
        }],
    );
    let before = runtime.metrics();
    let outcome = dispatch_proxy(build_proxy_dispatch_request(
        0,
        ProxyMethod::Get,
        "/",
        None,
        Vec::new(),
        None,
        "127.0.0.1",
        "http",
    ))
    .await;
    let after = runtime.metrics();
    let _response = proxy_outcome_to_hyper(outcome);
    let final_metrics = runtime.metrics();
    assert_eq!(final_metrics.responses_502, after.responses_502);
    assert!(after.responses_502 >= before.responses_502);
}

#[test]
fn register_duplicate_still_rejected_kd3_3() {
    use exyonq_core::{
        clear_global_proxy_dispatch_for_register_once_test, contract_service_registration_test_gate,
    };
    let _gate = contract_service_registration_test_gate();
    clear_global_proxy_dispatch_for_register_once_test();
    register_proxy_dispatch_service(Arc::new(ProxyRuntime::new())).expect("first");
    assert_eq!(
        register_proxy_dispatch_service(Arc::new(ProxyRuntime::new())),
        Err(ProxyRegisterError::AlreadyRegistered)
    );
    clear_global_proxy_dispatch_for_register_once_test();
}
