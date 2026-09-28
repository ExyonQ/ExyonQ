use exyonq_mod_fastcgi::{
    decode_forward_response, encode_record_frame, DecodeError, FastcgiRecordTransport,
    PhpFpmClient, ScriptedFpmConfig, ScriptedFpmTransport, FCGI_BEGIN_REQUEST, FCGI_END_REQUEST,
    FCGI_PARAMS, FCGI_REQUEST_COMPLETE, FCGI_STDOUT, FCGI_VERSION_1,
};

#[test]
fn golden_scripted_fpm_forward_once() {
    let body = b"Content-Type: text/plain\r\n\r\nOK";
    let peer = ScriptedFpmTransport::new(ScriptedFpmConfig {
        request_id: 1,
        stdout_body: body.to_vec(),
        app_status: 0,
        protocol_status: FCGI_REQUEST_COMPLETE,
    });
    let client = PhpFpmClient::with_transport("php", peer);

    let response = client
        .forward_once(
            &[("REQUEST_METHOD", "GET"), ("SCRIPT_NAME", "/index.php")],
            b"",
        )
        .expect("golden roundtrip");

    assert_eq!(response.stdout, body);
    assert_eq!(response.app_status, 0);
    assert_eq!(response.protocol_status, FCGI_REQUEST_COMPLETE);
}

#[test]
fn forward_once_with_stdin_body() {
    let peer = ScriptedFpmTransport::default();
    let client = PhpFpmClient::with_transport("php", peer);

    let response = client
        .forward_once(&[("REQUEST_METHOD", "POST")], b"ping")
        .expect("stdin roundtrip");

    assert!(response.stdout.starts_with(b"Status:"));
    assert_eq!(response.app_status, 0);
}

#[test]
fn decode_rejects_missing_end_request() {
    let stdout_only = encode_record_frame(1, FCGI_STDOUT, b"hi").expect("stdout frame");
    let err = decode_forward_response(&[stdout_only]).unwrap_err();
    assert_eq!(err, DecodeError::MissingEndRequest);
}

#[test]
fn decode_rejects_truncated_end_request() {
    let short_end = encode_record_frame(1, FCGI_END_REQUEST, &[0, 0, 0, 0]).expect("end");
    let err = decode_forward_response(&[short_end]).unwrap_err();
    assert_eq!(err, DecodeError::TruncatedEndRequest);
}

#[test]
fn scripted_accepts_begin_request_before_params() {
    let peer = ScriptedFpmTransport::default();
    let begin = encode_record_frame(1, FCGI_BEGIN_REQUEST, &[0; 8]).expect("begin");
    peer.submit_frame(&begin).expect("begin accepted");
    let params_term = encode_record_frame(1, FCGI_PARAMS, &[]).expect("params");
    peer.submit_frame(&params_term).expect("params");
}

#[test]
fn scripted_rejects_truncated_submit() {
    let peer = ScriptedFpmTransport::default();
    let err = peer
        .submit_frame(&[FCGI_VERSION_1, FCGI_PARAMS, 0, 1])
        .unwrap_err();
    assert!(matches!(
        err,
        exyonq_mod_fastcgi::TransportError::InvalidFrame(_)
    ));
}

#[test]
fn take_response_before_stdin_complete_fails() {
    let peer = ScriptedFpmTransport::default();
    let params_end = encode_record_frame(1, FCGI_PARAMS, &[]).expect("params end");
    peer.submit_frame(&params_end).expect("params terminator");
    let err = peer.take_response().unwrap_err();
    assert!(matches!(
        err,
        exyonq_mod_fastcgi::TransportError::RequestIncomplete
    ));
}

#[test]
fn forward_once_second_call_resets_scripted_peer() {
    let peer = ScriptedFpmTransport::default();
    let client = PhpFpmClient::with_transport("php", peer);

    client.forward_once(&[], b"").expect("first");
    let response = client.forward_once(&[], b"").expect("second after reset");
    assert_eq!(response.app_status, 0);
}

#[test]
fn src_modules_have_no_socket_symbols() {
    for path in [
        "src/client.rs",
        "src/transport.rs",
        "src/scripted.rs",
        "src/encode.rs",
    ] {
        let content = std::fs::read_to_string(format!("{}/{}", env!("CARGO_MANIFEST_DIR"), path))
            .expect("read src");
        for forbidden in [
            "connect(",
            "TcpStream",
            "UnixStream",
            "tokio::net",
            "std::net",
        ] {
            assert!(
                !content.contains(forbidden),
                "{path} must not reference `{forbidden}`"
            );
        }
    }
}
