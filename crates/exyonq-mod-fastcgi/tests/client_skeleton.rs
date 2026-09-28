use exyonq_mod_fastcgi::{
    ClientError, PhpFpmClient, TransportError, FCGI_VERSION_1, RECORD_HEADER_LEN,
};

fn minimal_frame() -> [u8; RECORD_HEADER_LEN] {
    [FCGI_VERSION_1, 1, 0, 1, 0, 0, 0, 0]
}

#[test]
fn client_exposes_pool_label_only() {
    let client = PhpFpmClient::new("php");
    assert_eq!(client.pool_label().name, "php");
}

#[test]
fn forward_request_is_not_implemented() {
    let client = PhpFpmClient::new("php");
    let frame = minimal_frame();
    assert_eq!(
        client.forward_request(&[&frame]),
        Err(ClientError::InertUnavailable)
    );
}

#[test]
fn submit_frame_uses_inert_transport() {
    let client = PhpFpmClient::new("php");
    assert_eq!(
        client.submit_frame(&minimal_frame()),
        Err(ClientError::Transport(TransportError::InertUnavailable))
    );
}

#[test]
fn client_skeleton_has_no_socket_fields() {
    for path in ["src/client.rs", "src/transport.rs", "src/scripted.rs"] {
        let content = std::fs::read_to_string(format!("{}/{}", env!("CARGO_MANIFEST_DIR"), path))
            .expect("read src");
        for forbidden in ["TcpStream", "UnixStream", "connect(", "tokio::net"] {
            assert!(
                !content.contains(forbidden),
                "{path} must not reference `{forbidden}`"
            );
        }
    }
}

#[test]
fn default_client_is_inert() {
    let client = PhpFpmClient::default();
    assert_eq!(client.pool_label().name, "default");
    assert!(client.forward_request(&[]).is_err());
}
