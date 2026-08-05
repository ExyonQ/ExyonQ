use exyonq_mod_fastcgi::{
    parse_record, record::MAX_RECORD_FRAME_LEN, RecordHeader, FCGI_VERSION_1, MAX_CONTENT_LENGTH,
    MAX_PADDING_LENGTH, RECORD_HEADER_LEN,
};

fn encode_header(header: RecordHeader) -> [u8; RECORD_HEADER_LEN] {
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
fn parse_minimal_empty_record() {
    let header = RecordHeader {
        version: FCGI_VERSION_1,
        record_type: 1,
        request_id: 1,
        content_length: 0,
        padding_length: 0,
        reserved: 0,
    };
    let frame = encode_header(header);
    let parsed = parse_record(&frame).expect("minimal record");
    assert_eq!(parsed.header, header);
    assert!(parsed.content.is_empty());
    assert!(parsed.padding.is_empty());
    assert_eq!(parsed.consumed, RECORD_HEADER_LEN);
}

#[test]
fn parse_record_with_content() {
    let header = RecordHeader {
        version: FCGI_VERSION_1,
        record_type: 4,
        request_id: 42,
        content_length: 5,
        padding_length: 3,
        reserved: 0,
    };
    let mut frame = encode_header(header).to_vec();
    frame.extend_from_slice(b"hello");
    frame.extend_from_slice(&[0, 0, 0]);

    let parsed = parse_record(&frame).expect("record with content");
    assert_eq!(parsed.header, header);
    assert_eq!(parsed.content, b"hello");
    assert_eq!(parsed.padding, &[0, 0, 0]);
    assert_eq!(parsed.consumed, frame.len());
}

#[test]
fn parse_record_at_max_spec_frame_length() {
    let header = RecordHeader {
        version: FCGI_VERSION_1,
        record_type: 1,
        request_id: 1,
        content_length: MAX_CONTENT_LENGTH,
        padding_length: MAX_PADDING_LENGTH,
        reserved: 0,
    };
    assert_eq!(header.frame_len(), MAX_RECORD_FRAME_LEN);

    let mut frame = encode_header(header).to_vec();
    frame.resize(MAX_RECORD_FRAME_LEN, 0);

    let parsed = parse_record(&frame).expect("max spec frame");
    assert_eq!(parsed.header, header);
    assert_eq!(parsed.content.len(), usize::from(MAX_CONTENT_LENGTH));
    assert_eq!(parsed.padding.len(), usize::from(MAX_PADDING_LENGTH));
    assert_eq!(parsed.consumed, MAX_RECORD_FRAME_LEN);
}
