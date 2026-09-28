//! SEC-PROXY-002 — classify_hyper_response unknown-length / small-CL bounds.
//! Lives under tests/ so dependency hot-path BoxBody budgets on src/ are unchanged.

use bytes::Bytes;
use exyonq_mod_proxy::{take_streaming, BENCH_SMALL_UPSTREAM_BODY};
use exyonq_module_api::proxy_dispatch::{ProxyDispatchOutcome, ProxyMethod};
use http_body_util::{BodyExt, Full};
use hyper::Response;

// Re-export path: classify_hyper_response is crate-public via hyper_forward.
use exyonq_mod_proxy::hyper_forward::classify_hyper_response;

fn full_body(bytes: Bytes) -> http_body_util::combinators::BoxBody<Bytes, hyper::Error> {
    Full::from(bytes).map_err(|never| match never {}).boxed()
}

#[tokio::test]
async fn unknown_length_large_body_is_streaming_not_materialized() {
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from(vec![b'z'; 100_000])))
        .expect("response");
    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    match outcome {
        ProxyDispatchOutcome::Streaming { stream, .. } => {
            let resp = take_streaming(stream).expect("handle");
            let collected = resp.into_body().collect().await.expect("collect stream");
            assert_eq!(collected.to_bytes().len(), 100_000);
        }
        other => panic!("expected Streaming, got {other:?}"),
    }
}

#[tokio::test]
async fn unknown_length_small_body_is_streaming() {
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from_static(b"ok")))
        .expect("response");
    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    assert!(matches!(outcome, ProxyDispatchOutcome::Streaming { .. }));
}

#[tokio::test]
async fn small_declared_exact_streams() {
    let response = Response::builder()
        .status(200)
        .header("content-length", "4")
        .body(full_body(Bytes::from_static(b"abcd")))
        .expect("response");
    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    match outcome {
        ProxyDispatchOutcome::Streaming { stream, .. } => {
            let resp = take_streaming(stream).expect("handle");
            let collected = resp.into_body().collect().await.expect("collect");
            assert_eq!(&collected.to_bytes()[..], b"abcd");
        }
        other => panic!("expected Streaming, got {other:?}"),
    }
}

#[tokio::test]
async fn small_declared_boundary_at_threshold_streams() {
    let body = vec![b'x'; BENCH_SMALL_UPSTREAM_BODY];
    let response = Response::builder()
        .status(200)
        .header("content-length", BENCH_SMALL_UPSTREAM_BODY.to_string())
        .body(full_body(Bytes::from(body.clone())))
        .expect("response");
    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    match outcome {
        ProxyDispatchOutcome::Streaming { stream, .. } => {
            let resp = take_streaming(stream).expect("handle");
            let collected = resp.into_body().collect().await.expect("collect");
            assert_eq!(collected.to_bytes(), body);
        }
        other => panic!("expected Streaming, got {other:?}"),
    }
}

#[tokio::test]
async fn false_small_content_length_falls_back_to_streaming() {
    let response = Response::builder()
        .status(200)
        .header("content-length", "2")
        .body(full_body(Bytes::from_static(b"abcd")))
        .expect("response");
    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    match outcome {
        ProxyDispatchOutcome::Streaming { stream, .. } => {
            let resp = take_streaming(stream).expect("handle");
            let collected = resp.into_body().collect().await.expect("collect");
            assert_eq!(&collected.to_bytes()[..], b"abcd");
        }
        other => panic!("expected Streaming fallback, got {other:?}"),
    }
}

#[tokio::test]
async fn zero_content_length_with_body_streams() {
    let response = Response::builder()
        .status(200)
        .header("content-length", "0")
        .body(full_body(Bytes::from_static(b"x")))
        .expect("response");
    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    assert!(matches!(outcome, ProxyDispatchOutcome::Streaming { .. }));
}

#[tokio::test]
async fn large_declared_content_length_streams() {
    let len = BENCH_SMALL_UPSTREAM_BODY + 1;
    let response = Response::builder()
        .status(200)
        .header("content-length", len.to_string())
        .body(full_body(Bytes::from(vec![b'y'; len])))
        .expect("response");
    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Get).await;
    assert!(matches!(outcome, ProxyDispatchOutcome::Streaming { .. }));
}

#[tokio::test]
async fn sse_content_type_streams_without_collect_path() {
    let response = Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .body(full_body(Bytes::from(vec![b'e'; 50_000])))
        .expect("response");
    let outcome = classify_hyper_response(response, "/other", ProxyMethod::Get).await;
    assert!(matches!(outcome, ProxyDispatchOutcome::Streaming { .. }));
}

#[tokio::test]
async fn api_stream_path_streams() {
    let response = Response::builder()
        .status(200)
        .body(full_body(Bytes::from(vec![b's'; 50_000])))
        .expect("response");
    let outcome = classify_hyper_response(response, "/api/stream", ProxyMethod::Get).await;
    assert!(matches!(outcome, ProxyDispatchOutcome::Streaming { .. }));
}

#[tokio::test]
async fn head_empty_materialized() {
    let response = Response::builder()
        .status(200)
        .header("content-length", "100")
        .body(full_body(Bytes::from(vec![b'h'; 100])))
        .expect("response");
    let outcome = classify_hyper_response(response, "/api/data", ProxyMethod::Head).await;
    match outcome {
        ProxyDispatchOutcome::Materialized(m) => assert!(m.body.is_empty()),
        other => panic!("expected empty Materialized, got {other:?}"),
    }
}
