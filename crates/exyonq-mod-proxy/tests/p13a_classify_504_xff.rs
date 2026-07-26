//! P1.3a — 504 classify + XFF trust defaults.

use exyonq_mod_proxy::hyper_forward::{classify_hyper_response, gateway_timeout};
use exyonq_mod_proxy::runtime::ProxyRuntime;
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest, ProxyDispatchService,
    ProxyMethod,
};
use std::time::Duration;

#[tokio::test]
async fn classify_maps_504_to_gateway_timeout() {
    let outcome = classify_hyper_response(gateway_timeout(), "/api/x", ProxyMethod::Get).await;
    assert!(matches!(outcome, ProxyDispatchOutcome::GatewayTimeout));
}

#[tokio::test]
async fn xff_uses_remote_addr_not_inbound_header() {
    let rt = ProxyRuntime::new();
    // Bind a slot that will fail connect quickly → we only need request building side effects.
    // Use an unreachable port; assert via unit path: dispatch still builds XFF from remote_addr.
    // Direct check: spoofed inbound XFF must not become upstream XFF when remote_addr differs.
    // We validate through a tiny local echo of headers.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let seen_c = seen.clone();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        *seen_c.lock().await = String::from_utf8_lossy(&buf[..n]).into_owned();
        use tokio::io::AsyncWriteExt;
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await;
    });

    rt.bind_compiled_slots(
        1,
        &[ProxyCompiledSlot {
            cluster_id: 0,
            upstream_name: "u".into(),
            target: format!("http://127.0.0.1:{port}"),
            timeout: Duration::from_secs(2),
        }],
    );

    let outcome = rt
        .dispatch(ProxyDispatchRequest {
            cluster_id: 0,
            method: ProxyMethod::Get,
            path_and_query: "/xff".into(),
            host: None,
            headers: vec![
                ("x-forwarded-for".into(), "9.9.9.9".into()),
                ("x-forwarded-proto".into(), "https".into()),
            ],
            body: None,
            remote_addr: "203.0.113.10".into(),
            scheme: "http".into(),
        })
        .await;
    assert!(matches!(
        outcome,
        ProxyDispatchOutcome::Materialized(_) | ProxyDispatchOutcome::Streaming { .. }
    ));

    // Allow upstream task to finish reading.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let req = seen.lock().await.clone();
    assert!(
        req.contains("x-forwarded-for: 203.0.113.10"),
        "expected remote_addr XFF, got:\n{req}"
    );
    assert!(
        !req.contains("x-forwarded-for: 9.9.9.9"),
        "must not trust inbound XFF:\n{req}"
    );
    assert!(
        req.to_ascii_lowercase().contains("x-forwarded-proto: http"),
        "expected scheme as X-Forwarded-Proto:\n{req}"
    );
}

#[tokio::test]
async fn post_xff_uses_remote_addr_not_inbound_header() {
    let rt = ProxyRuntime::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let seen_c = seen.clone();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        *seen_c.lock().await = String::from_utf8_lossy(&buf[..n]).into_owned();
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await;
    });

    rt.bind_compiled_slots(
        1,
        &[ProxyCompiledSlot {
            cluster_id: 0,
            upstream_name: "u".into(),
            target: format!("http://127.0.0.1:{port}"),
            timeout: Duration::from_secs(2),
        }],
    );

    let outcome = rt
        .dispatch(ProxyDispatchRequest {
            cluster_id: 0,
            method: ProxyMethod::Post,
            path_and_query: "/post-xff".into(),
            host: None,
            headers: vec![
                ("content-type".into(), "text/plain".into()),
                ("x-forwarded-for".into(), "9.9.9.9".into()),
                ("x-forwarded-proto".into(), "https".into()),
                ("forwarded".into(), "for=9.9.9.9".into()),
            ],
            body: Some(b"hi".to_vec()),
            remote_addr: "198.51.100.7".into(),
            scheme: "http".into(),
        })
        .await;
    assert!(matches!(
        outcome,
        ProxyDispatchOutcome::Materialized(_) | ProxyDispatchOutcome::Streaming { .. }
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    let req = seen.lock().await.clone();
    assert!(
        req.contains("x-forwarded-for: 198.51.100.7"),
        "expected remote_addr XFF on POST, got:\n{req}"
    );
    assert!(
        !req.contains("9.9.9.9"),
        "must not forward spoofed XFF/Forwarded on POST:\n{req}"
    );
    assert!(
        req.to_ascii_lowercase().contains("x-forwarded-proto: http"),
        "expected scheme XFP on POST:\n{req}"
    );
}
