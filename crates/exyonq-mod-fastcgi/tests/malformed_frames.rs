use exyonq_mod_fastcgi::{
    parse_header, parse_record, record::MAX_RECORD_FRAME_LEN, ParseError, FCGI_VERSION_1,
    RECORD_HEADER_LEN,
};

#[test]
fn rejects_buffer_shorter_than_header() {
    let err = parse_record(&[1, 1, 0, 1]).unwrap_err();
    assert_eq!(
        err,
        ParseError::BufferTooShort {
            need: RECORD_HEADER_LEN,
            have: 4,
        }
    );
}

#[test]
fn rejects_truncated_content() {
    let mut frame = vec![FCGI_VERSION_1, 1, 0, 1, 0, 4, 0, 0];
    frame.extend_from_slice(b"ab");
    let err = parse_record(&frame).unwrap_err();
    assert_eq!(
        err,
        ParseError::TruncatedContent {
            expected: 4,
            have: 2,
        }
    );
}

#[test]
fn rejects_truncated_padding() {
    let mut frame = vec![FCGI_VERSION_1, 1, 0, 1, 0, 2, 2, 0];
    frame.extend_from_slice(b"ok");
    let err = parse_record(&frame).unwrap_err();
    assert_eq!(
        err,
        ParseError::TruncatedPadding {
            expected: 2,
            have: 0,
        }
    );
}

#[test]
fn rejects_invalid_version() {
    let frame = [2u8, 1, 0, 1, 0, 0, 0, 0];
    let err = parse_header(&frame).unwrap_err();
    assert_eq!(err, ParseError::UnsupportedVersion(2));
    let err = parse_record(&frame).unwrap_err();
    assert_eq!(err, ParseError::UnsupportedVersion(2));
}

#[test]
fn max_record_frame_len_matches_spec() {
    assert_eq!(MAX_RECORD_FRAME_LEN, RECORD_HEADER_LEN + 65535 + 255);
}

#[test]
fn rejects_truncated_content_when_header_claims_max_length() {
    let frame = [
        FCGI_VERSION_1,
        255, // unknown record type — framing-only layer does not reject
        0,
        1,
        0xFF,
        0xFF, // content_length = u16::MAX
        0,
        0,
    ];
    let err = parse_record(&frame).unwrap_err();
    assert_eq!(
        err,
        ParseError::TruncatedContent {
            expected: 65535,
            have: 0,
        }
    );
}

#[test]
fn unknown_record_type_allowed_at_framing_layer() {
    let frame = [FCGI_VERSION_1, 255, 0, 1, 0, 0, 0, 0];
    let parsed = parse_record(&frame).expect("record_type is not validated in PR3-A framing");
    assert_eq!(parsed.header.record_type, 255);
}

#[test]
fn rejects_buffer_one_byte_short_of_spec_max_frame() {
    let mut frame = vec![FCGI_VERSION_1, 1, 0, 1, 0xFF, 0xFF, 0xFF, 0];
    frame.resize(MAX_RECORD_FRAME_LEN - 1, 0);
    let err = parse_record(&frame).unwrap_err();
    assert!(matches!(err, ParseError::TruncatedPadding { .. }));
}

#[test]
fn small_random_buffers_never_panic() {
    for len in 0..32 {
        for seed in 0u8..=255 {
            let buf: Vec<u8> = (0..len).map(|i| seed.wrapping_add(i as u8)).collect();
            let _ = parse_record(&buf);
        }
    }
}
