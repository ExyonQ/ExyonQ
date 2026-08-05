mod common;

use common::scripted_peer::{cleanup_unix, join_peer, spawn_unix_peer, PeerConfig, PeerMode};
use exyonq_mod_fastcgi::{parse_cgi_stdout, UnixFpmTransport};
use std::time::Duration;

#[test]
fn unix_transport_200_with_exact_body() {
    let body = b"Content-Type: text/plain\r\n\r\nexact-body";
    let (path, handle) = spawn_unix_peer(PeerConfig {
        stdout_body: body.to_vec(),
        ..PeerConfig::default()
    });
    let transport = UnixFpmTransport::new(path.clone(), 1, Duration::from_secs(2));
    let response = transport
        .forward_once(
            &[
                ("REQUEST_METHOD".into(), "GET".into()),
                ("SCRIPT_NAME".into(), "/index.php".into()),
            ],
            b"",
        )
        .expect("forward");
    assert_eq!(response.stdout, body);
    let parsed = parse_cgi_stdout(&response.stdout).expect("cgi");
    assert_eq!(parsed.body, b"exact-body");
    join_peer(handle);
    cleanup_unix(&path);
}

#[test]
fn unix_transport_propagates_cgi_404() {
    let body = b"Status: 404 Not Found\r\nContent-Type: text/plain\r\n\r\nmissing";
    let (path, handle) = spawn_unix_peer(PeerConfig {
        stdout_body: body.to_vec(),
        ..PeerConfig::default()
    });
    let transport = UnixFpmTransport::new(path.clone(), 1, Duration::from_secs(2));
    let response = transport
        .forward_once(&[("REQUEST_METHOD".into(), "GET".into())], b"")
        .expect("forward");
    let parsed = parse_cgi_stdout(&response.stdout).expect("cgi");
    assert_eq!(parsed.status, 404);
    join_peer(handle);
    cleanup_unix(&path);
}

#[test]
fn unix_transport_timeout_maps_to_error() {
    let (path, handle) = spawn_unix_peer(PeerConfig {
        mode: PeerMode::SlowRead,
        slow_delay: Duration::from_secs(2),
        ..PeerConfig::default()
    });
    let transport = UnixFpmTransport::with_timeouts(
        path.clone(),
        1,
        Duration::from_millis(100),
        Duration::from_millis(50),
        Duration::from_millis(50),
    );
    let err = transport
        .forward_once(&[("REQUEST_METHOD".into(), "GET".into())], b"")
        .unwrap_err();
    assert!(
        matches!(
            err,
            exyonq_mod_fastcgi::WireError::Timeout | exyonq_mod_fastcgi::WireError::IoFailed
        ),
        "slow peer must fail closed, got {err:?}"
    );
    join_peer(handle);
    cleanup_unix(&path);
}

#[test]
fn unix_transport_eof_before_end_request_is_bad_gateway() {
    let (path, handle) = spawn_unix_peer(PeerConfig {
        mode: PeerMode::DropAfterParams,
        ..PeerConfig::default()
    });
    let transport = UnixFpmTransport::new(path.clone(), 1, Duration::from_secs(2));
    let err = transport
        .forward_once(&[("REQUEST_METHOD".into(), "GET".into())], b"")
        .unwrap_err();
    assert!(matches!(
        err,
        exyonq_mod_fastcgi::WireError::ConnectionClosed | exyonq_mod_fastcgi::WireError::Decode(_)
    ));
    join_peer(handle);
    cleanup_unix(&path);
}

#[test]
fn unix_transport_malformed_response_is_bad_gateway() {
    let (path, handle) = spawn_unix_peer(PeerConfig {
        mode: PeerMode::SendGarbage,
        ..PeerConfig::default()
    });
    let transport = UnixFpmTransport::new(path.clone(), 1, Duration::from_secs(2));
    let err = transport
        .forward_once(&[("REQUEST_METHOD".into(), "GET".into())], b"")
        .unwrap_err();
    let _: exyonq_mod_fastcgi::WireError = err;
    join_peer(handle);
    cleanup_unix(&path);
}

#[test]
fn unix_transport_stderr_with_valid_stdout_succeeds() {
    let stdout = b"Content-Type: text/plain\r\n\r\nok";
    let (path, handle) = spawn_unix_peer(PeerConfig {
        stdout_body: stdout.to_vec(),
        ..PeerConfig::default()
    });
    let transport = UnixFpmTransport::new(path.clone(), 1, Duration::from_secs(2));
    let response = transport
        .forward_once(&[("REQUEST_METHOD".into(), "GET".into())], b"")
        .expect("forward with stderr tolerated");
    assert_eq!(response.stdout, stdout);
    join_peer(handle);
    cleanup_unix(&path);
}
