#![cfg(feature = "otel")]

use exyonq_config_ir::{LoggingConfig, OtelLoggingConfig, OtlpProtocol};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn otlp_http_posts_to_real_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
    let seen2 = Arc::clone(&seen);
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = vec![0u8; 65536];
            let mut total = Vec::new();
            stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        total.extend_from_slice(&buf[..n]);
                        if total.windows(4).any(|w| w == b"\r\n\r\n") && total.len() > 64 {
                            // respond 200 quickly so client finishes
                            let _ =
                                stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n");
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            *seen2.lock().unwrap() = total;
        }
    });

    let mut cfg = LoggingConfig::default();
    cfg.console.enabled = false;
    cfg.file.enabled = false;
    cfg.otel = OtelLoggingConfig {
        enabled: true,
        endpoint: Some(format!("http://127.0.0.1:{port}")),
        protocol: OtlpProtocol::HttpProtobuf,
        service_name: "exyonq-probe".into(),
        metrics_export: false,
    };
    // Ensure at least one sink enabled for install gates — enable console stderr
    cfg.console.enabled = true;

    let guard = exyonq_observability::install_from_config(&cfg).expect("install");
    let span = tracing::info_span!("request", request_id = "probe-req-1", otel.kind = "server");
    {
        let _g = span.enter();
        tracing::info!(event = "access", "access");
    }
    drop(span);
    // Force export via generation swap / shutdown
    guard.shutdown();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut body = Vec::new();
    while Instant::now() < deadline {
        body = seen.lock().unwrap().clone();
        if !body.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let text = String::from_utf8_lossy(&body);
    eprintln!("HTTP REQUEST SEEN:\n{text}");
    assert!(
        text.contains("POST") && text.contains("/v1/traces"),
        "expected OTLP POST /v1/traces, got: {text}"
    );
}
