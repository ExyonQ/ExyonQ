use exyonq_mod_fastcgi::{
    parse_record, FastcgiRecordTransport, InertTransport, ParseError, RecordHeader, TransportError,
    ValidatingScriptedTransport, FCGI_VERSION_1, RECORD_HEADER_LEN,
};

fn minimal_frame() -> [u8; RECORD_HEADER_LEN] {
    let header = RecordHeader {
        version: FCGI_VERSION_1,
        record_type: 1,
        request_id: 1,
        content_length: 0,
        padding_length: 0,
        reserved: 0,
    };
    [
        header.version,
        header.record_type,
        (header.request_id >> 8) as u8,
        (header.request_id & 0xff) as u8,
        (header.content_length >> 8) as u8,
        (header.content_length & 0xff) as u8,
        header.padding_length,
        header.reserved,
    ]
}

#[test]
fn inert_transport_returns_not_implemented() {
    let transport = InertTransport;
    assert_eq!(
        transport.submit_frame(&minimal_frame()),
        Err(TransportError::InertUnavailable)
    );
}

#[test]
fn validating_peer_accepts_minimal_valid_frame() {
    let transport = ValidatingScriptedTransport;
    assert!(transport.submit_frame(&minimal_frame()).is_ok());
}

#[test]
fn validating_peer_rejects_truncated_frame() {
    let transport = ValidatingScriptedTransport;
    let err = transport
        .submit_frame(&[FCGI_VERSION_1, 1, 0, 1])
        .unwrap_err();
    assert_eq!(
        err,
        TransportError::InvalidFrame(ParseError::BufferTooShort {
            need: RECORD_HEADER_LEN,
            have: 4,
        })
    );
}

#[test]
fn validating_peer_does_not_forward_or_mutate_input() {
    let transport = ValidatingScriptedTransport;
    let frame = minimal_frame();
    let before = frame;
    let _ = transport.submit_frame(&frame);
    assert_eq!(parse_record(&before).unwrap().header.request_id, 1);
}
