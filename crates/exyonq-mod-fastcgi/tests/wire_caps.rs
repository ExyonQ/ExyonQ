use exyonq_mod_fastcgi::{
    encode_params_body_owned, encode_stdin_frames, EncodeError, MAX_FCGI_PARAMS_BYTES,
    MAX_FCGI_STDIN_BYTES,
};

#[test]
fn stdin_over_cap_rejected_at_encode() {
    let body = vec![0u8; MAX_FCGI_STDIN_BYTES + 1];
    let err = encode_stdin_frames(1, &body).unwrap_err();
    assert_eq!(err, EncodeError::AggregateCapExceeded);
}

#[test]
fn params_over_cap_rejected_at_encode() {
    let mut pairs = Vec::new();
    let chunk = "x".repeat(120);
    let chunks_needed = MAX_FCGI_PARAMS_BYTES / 122 + 2;
    for i in 0..chunks_needed {
        pairs.push((format!("K{i:04}"), chunk.clone()));
    }
    let err = encode_params_body_owned(&pairs).unwrap_err();
    assert_eq!(err, EncodeError::AggregateCapExceeded);
}

#[test]
fn params_under_cap_ok() {
    let pairs = vec![
        ("REQUEST_METHOD".to_string(), "GET".to_string()),
        ("REQUEST_URI".to_string(), "/index.php".to_string()),
    ];
    assert!(encode_params_body_owned(&pairs).is_ok());
}
