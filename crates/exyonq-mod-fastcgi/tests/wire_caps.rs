/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
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
