mod common;

use common::scripted_peer::{
    cleanup_unix, join_peer, spawn_tcp_peer, spawn_unix_peer, PeerConfig, PeerMode,
};
use exyonq_mod_fastcgi::{ClientError, MinForwardRequest, PhpFpmClient, WireError, WireTransport};
use std::path::PathBuf;

#[test]
fn connect_to_missing_unix_socket_fails() {
    let transport = WireTransport::unix(PathBuf::from("/tmp/exyonq-no-such-fcgi-peer.sock"), 1);
    let client = PhpFpmClient::with_transport("php", transport);
    let req = MinForwardRequest::get("/", "/", "/");
    let err = client.forward_min_request(&req).unwrap_err();
    assert!(matches!(
        err,
        ClientError::Wire(WireError::ConnectionFailed)
    ));
}

#[test]
fn peer_drop_after_params_closes_connection() {
    let (path, handle) = spawn_unix_peer(PeerConfig {
        mode: PeerMode::DropAfterParams,
        ..PeerConfig::default()
    });
    let client = PhpFpmClient::with_transport("php", WireTransport::unix(path.clone(), 1));
    let req = MinForwardRequest::get("/", "/", "/");
    let err = client.forward_min_request(&req).unwrap_err();
    assert!(
        matches!(err, ClientError::Wire(_)),
        "peer drop must fail at wire layer, got {err:?}"
    );
    join_peer(handle);
    cleanup_unix(&path);
}

#[test]
fn peer_send_garbage_yields_invalid_frame() {
    let (path, handle) = spawn_unix_peer(PeerConfig {
        mode: PeerMode::SendGarbage,
        ..PeerConfig::default()
    });
    let client = PhpFpmClient::with_transport("php", WireTransport::unix(path.clone(), 1));
    let req = MinForwardRequest::get("/", "/", "/");
    let err = client.forward_min_request(&req).unwrap_err();
    assert!(
        matches!(err, ClientError::Wire(_)),
        "garbage peer must fail at wire layer, got {err:?}"
    );
    join_peer(handle);
    cleanup_unix(&path);
}

#[test]
fn peer_omit_end_request_fails_decode() {
    let (path, handle) = spawn_unix_peer(PeerConfig {
        mode: PeerMode::OmitEndRequest,
        ..PeerConfig::default()
    });
    let client = PhpFpmClient::with_transport("php", WireTransport::unix(path.clone(), 1));
    let req = MinForwardRequest::get("/", "/", "/");
    let err = client.forward_min_request(&req).unwrap_err();
    assert!(matches!(err, ClientError::Wire(WireError::Decode(_))));
    join_peer(handle);
    cleanup_unix(&path);
}

#[test]
fn tcp_peer_drop_after_params() {
    let (port, handle) = spawn_tcp_peer(PeerConfig {
        mode: PeerMode::DropAfterParams,
        ..PeerConfig::default()
    });
    let client = PhpFpmClient::with_transport("php", WireTransport::tcp("127.0.0.1", port, 1));
    let req = MinForwardRequest::get("/", "/", "/");
    assert!(client.forward_min_request(&req).is_err());
    join_peer(handle);
}

#[test]
fn tcp_connect_to_closed_port_fails() {
    let transport = WireTransport::tcp("127.0.0.1", 1, 1);
    let client = PhpFpmClient::with_transport("php", transport);
    let req = MinForwardRequest::get("/", "/", "/");
    let err = client.forward_min_request(&req).unwrap_err();
    assert!(matches!(
        err,
        ClientError::Wire(WireError::ConnectionFailed)
    ));
}
