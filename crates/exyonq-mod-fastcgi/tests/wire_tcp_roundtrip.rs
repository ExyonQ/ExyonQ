mod common;

use common::scripted_peer::{join_peer, spawn_tcp_peer, PeerConfig};
use exyonq_mod_fastcgi::{MinForwardRequest, PhpFpmClient, WireTransport, FCGI_REQUEST_COMPLETE};

#[test]
fn tcp_wire_get_roundtrip() {
    let body = b"Content-Type: text/plain\r\n\r\ntcp-ok";
    let (port, handle) = spawn_tcp_peer(PeerConfig {
        stdout_body: body.to_vec(),
        ..PeerConfig::default()
    });

    let transport = WireTransport::tcp("127.0.0.1", port, 1);
    let client = PhpFpmClient::with_transport("php", transport);
    let req = MinForwardRequest::get("/index.php", "/index.php", "/var/www/index.php");

    let response = client.forward_min_request(&req).expect("tcp roundtrip");
    assert_eq!(response.stdout, body);
    assert_eq!(response.protocol_status, FCGI_REQUEST_COMPLETE);

    join_peer(handle);
}

#[test]
fn tcp_wire_post_with_stdin() {
    let (port, handle) = spawn_tcp_peer(PeerConfig::default());
    let transport = WireTransport::tcp("127.0.0.1", port, 1);
    let client = PhpFpmClient::with_transport("php", transport);
    let req = MinForwardRequest::post(
        "/index.php",
        "/index.php",
        "/var/www/index.php",
        "text/plain",
        b"ping",
    );

    let response = client.forward_min_request(&req).expect("tcp post");
    assert!(!response.stdout.is_empty());

    join_peer(handle);
}
