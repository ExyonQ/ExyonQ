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
//! In-memory FastCGI record encoding and response decoding — PR4-A module-local.

use crate::parser::{parse_record, ParseError};
use crate::record::{
    RecordHeader, END_REQUEST_BODY_LEN, FCGI_BEGIN_REQUEST, FCGI_END_REQUEST, FCGI_PARAMS,
    FCGI_STDERR, FCGI_STDIN, FCGI_STDOUT, FCGI_VERSION_1, MAX_CONTENT_LENGTH, RECORD_HEADER_LEN,
};

/// Encode-time failures (bounded params; no live HTTP mapping).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    ParamTooLong,
    ContentTooLong,
    AggregateCapExceeded,
}

/// Decode-time failures for scripted responder sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    EmptyResponse,
    Frame(ParseError),
    UnexpectedRecordType { got: u8 },
    MissingEndRequest,
    TruncatedEndRequest,
    StdoutAfterEndRequest,
    StderrCapExceeded,
    InvalidEndRequestStatus,
}

/// Decoded scripted forward response (stdout + END_REQUEST fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedResponse {
    pub stdout: Vec<u8>,
    pub app_status: u32,
    pub protocol_status: u8,
}

/// Encode an 8-byte record header.
pub fn encode_header(header: RecordHeader) -> [u8; RECORD_HEADER_LEN] {
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

/// One complete record frame (header + content + padding).
pub fn encode_record_frame(
    request_id: u16,
    record_type: u8,
    content: &[u8],
) -> Result<Vec<u8>, EncodeError> {
    if content.len() > usize::from(MAX_CONTENT_LENGTH) {
        return Err(EncodeError::ContentTooLong);
    }
    let header = RecordHeader {
        version: FCGI_VERSION_1,
        record_type,
        request_id,
        content_length: content.len() as u16,
        padding_length: 0,
        reserved: 0,
    };
    let mut frame = encode_header(header).to_vec();
    frame.extend_from_slice(content);
    Ok(frame)
}

/// Append a FastCGI name/value length field (short or long encoding).
pub fn append_length(buf: &mut Vec<u8>, len: usize) -> Result<(), EncodeError> {
    if len > u32::MAX as usize {
        return Err(EncodeError::ParamTooLong);
    }
    if len < 128 {
        buf.push(len as u8);
    } else {
        let value = len as u32;
        buf.push(((value >> 24) & 0xFF) as u8 | 0x80);
        buf.push(((value >> 16) & 0xFF) as u8);
        buf.push(((value >> 8) & 0xFF) as u8);
        buf.push((value & 0xFF) as u8);
    }
    Ok(())
}

/// `FCGI_BEGIN_REQUEST` for responder role (PHP-FPM); flags usually `0` or [`crate::FCGI_KEEP_CONN`].
pub fn encode_begin_request_frame(request_id: u16) -> Result<Vec<u8>, EncodeError> {
    encode_begin_request_frame_with_flags(request_id, 0)
}

/// `FCGI_BEGIN_REQUEST` with explicit flags (`FCGI_KEEP_CONN` for pooled reuse).
pub fn encode_begin_request_frame_with_flags(
    request_id: u16,
    flags: u8,
) -> Result<Vec<u8>, EncodeError> {
    let mut body = [0u8; 8];
    body[0..2].copy_from_slice(&1u16.to_be_bytes());
    body[2] = flags;
    encode_record_frame(request_id, FCGI_BEGIN_REQUEST, &body)
}

/// FastCGI name/value body for [`FCGI_PARAMS`] (each name/value length <= 127).
pub fn encode_params_body(params: &[(&str, &str)]) -> Result<Vec<u8>, EncodeError> {
    let mut body = Vec::new();
    for (name, value) in params {
        append_length(&mut body, name.len())?;
        append_length(&mut body, value.len())?;
        body.extend_from_slice(name.as_bytes());
        body.extend_from_slice(value.as_bytes());
    }
    Ok(body)
}

/// Owned name/value PARAMS body with aggregate cap enforcement.
pub fn encode_params_body_owned(params: &[(String, String)]) -> Result<Vec<u8>, EncodeError> {
    let refs: Vec<(&str, &str)> = params
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    let body = encode_params_body(&refs)?;
    crate::caps::check_size_cap(crate::caps::MAX_FCGI_PARAMS_BYTES, body.len())
        .map_err(|_| EncodeError::AggregateCapExceeded)?;
    Ok(body)
}

fn chunk_content(content: &[u8]) -> impl Iterator<Item = &[u8]> {
    content.chunks(usize::from(MAX_CONTENT_LENGTH))
}

/// PARAMS stream: data record(s) + empty PARAMS terminator.
pub fn encode_params_frames(
    request_id: u16,
    params: &[(&str, &str)],
) -> Result<Vec<Vec<u8>>, EncodeError> {
    let body = encode_params_body(params)?;
    encode_params_frames_from_body(request_id, &body)
}

/// PARAMS stream from owned pairs (cap-checked).
pub fn encode_params_frames_owned(
    request_id: u16,
    params: &[(String, String)],
) -> Result<Vec<Vec<u8>>, EncodeError> {
    let body = encode_params_body_owned(params)?;
    encode_params_frames_from_body(request_id, &body)
}

fn encode_params_frames_from_body(
    request_id: u16,
    body: &[u8],
) -> Result<Vec<Vec<u8>>, EncodeError> {
    let mut frames = Vec::new();
    if !body.is_empty() {
        for chunk in chunk_content(body) {
            frames.push(encode_record_frame(request_id, FCGI_PARAMS, chunk)?);
        }
    }
    frames.push(encode_record_frame(request_id, FCGI_PARAMS, &[])?);
    Ok(frames)
}

/// STDIN stream: body record(s) + empty STDIN terminator (end of body).
pub fn encode_stdin_frames(request_id: u16, body: &[u8]) -> Result<Vec<Vec<u8>>, EncodeError> {
    crate::caps::check_size_cap(crate::caps::MAX_FCGI_STDIN_BYTES, body.len())
        .map_err(|_| EncodeError::AggregateCapExceeded)?;
    let mut frames = Vec::new();
    for chunk in chunk_content(body) {
        frames.push(encode_record_frame(request_id, FCGI_STDIN, chunk)?);
    }
    frames.push(encode_record_frame(request_id, FCGI_STDIN, &[])?);
    Ok(frames)
}

/// Decode a responder frame sequence into stdout + END_REQUEST fields.
pub fn decode_forward_response(frames: &[Vec<u8>]) -> Result<DecodedResponse, DecodeError> {
    decode_forward_response_with_stderr(frames, true)
}

/// Decode with optional STDERR accumulation (diagnostic only; capped).
pub fn decode_forward_response_with_stderr(
    frames: &[Vec<u8>],
    collect_stderr: bool,
) -> Result<DecodedResponse, DecodeError> {
    if frames.is_empty() {
        return Err(DecodeError::EmptyResponse);
    }

    let mut stdout = Vec::new();
    let mut stderr_bytes = 0usize;
    let mut app_status = None;
    let mut protocol_status = None;
    let mut saw_end_request = false;

    for frame in frames {
        if saw_end_request {
            return Err(DecodeError::StdoutAfterEndRequest);
        }
        let parsed = parse_record(frame).map_err(DecodeError::Frame)?;
        match parsed.header.record_type {
            FCGI_STDOUT => {
                stdout.extend_from_slice(parsed.content);
                if stdout.len() > crate::caps::MAX_FCGI_RESPONSE_BYTES {
                    return Err(DecodeError::UnexpectedRecordType { got: FCGI_STDOUT });
                }
            }
            FCGI_STDERR if collect_stderr => {
                stderr_bytes = crate::caps::check_add_cap(
                    crate::caps::MAX_FCGI_STDERR_BYTES,
                    stderr_bytes,
                    parsed.content.len(),
                )
                .map_err(|_| DecodeError::StderrCapExceeded)?;
            }
            FCGI_STDERR => {}
            FCGI_END_REQUEST => {
                if parsed.content.len() < END_REQUEST_BODY_LEN {
                    return Err(DecodeError::TruncatedEndRequest);
                }
                app_status = Some(u32::from_be_bytes([
                    parsed.content[0],
                    parsed.content[1],
                    parsed.content[2],
                    parsed.content[3],
                ]));
                protocol_status = Some(parsed.content[4]);
                saw_end_request = true;
            }
            other => return Err(DecodeError::UnexpectedRecordType { got: other }),
        }
    }

    match (app_status, protocol_status) {
        (Some(app_status), Some(protocol_status)) => {
            if protocol_status != crate::record::FCGI_REQUEST_COMPLETE {
                return Err(DecodeError::InvalidEndRequestStatus);
            }
            Ok(DecodedResponse {
                stdout,
                app_status,
                protocol_status,
            })
        }
        _ => Err(DecodeError::MissingEndRequest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_length_short_and_long() {
        let mut short = Vec::new();
        append_length(&mut short, 42).expect("short");
        assert_eq!(short, vec![42]);

        let mut long = Vec::new();
        append_length(&mut long, 200).expect("long");
        assert_eq!(long.len(), 4);
        assert_eq!(long[0] & 0x80, 0x80);
    }

    #[test]
    fn params_fragmentation_splits_large_body() {
        let name = "K".to_string();
        let value = "V".repeat(usize::from(MAX_CONTENT_LENGTH));
        let params = vec![(name, value)];
        let frames = encode_params_frames_owned(7, &params).expect("frames");
        assert!(frames.len() >= 2);
    }

    #[test]
    fn stdin_fragmentation_for_large_body() {
        let body = vec![b'a'; usize::from(MAX_CONTENT_LENGTH) + 1];
        let frames = encode_stdin_frames(3, &body).expect("stdin");
        assert!(frames.len() >= 3);
    }

    #[test]
    fn decode_multi_record_stdout_concatenates() {
        let a = encode_record_frame(1, FCGI_STDOUT, b"Content-Type: text/plain\r\n\r\n").unwrap();
        let b = encode_record_frame(1, FCGI_STDOUT, b"hello").unwrap();
        let mut end = [0u8; 8];
        end[4] = crate::record::FCGI_REQUEST_COMPLETE;
        let end_frame = encode_record_frame(1, FCGI_END_REQUEST, &end).unwrap();
        let decoded = decode_forward_response(&[a, b, end_frame]).expect("decoded");
        assert!(decoded.stdout.ends_with(b"hello"));
    }

    #[test]
    fn decode_missing_end_request_fails() {
        let stdout = encode_record_frame(1, FCGI_STDOUT, b"ok").unwrap();
        assert_eq!(
            decode_forward_response(&[stdout]).unwrap_err(),
            DecodeError::MissingEndRequest
        );
    }

    #[test]
    fn decode_stderr_with_success_is_allowed() {
        let stdout =
            encode_record_frame(1, FCGI_STDOUT, b"Content-Type: text/plain\r\n\r\nok").unwrap();
        let stderr = encode_record_frame(1, FCGI_STDERR, b"warning").unwrap();
        let mut end = [0u8; 8];
        end[4] = crate::record::FCGI_REQUEST_COMPLETE;
        let end_frame = encode_record_frame(1, FCGI_END_REQUEST, &end).unwrap();
        let decoded =
            decode_forward_response_with_stderr(&[stdout, stderr, end_frame], true).expect("ok");
        assert!(decoded.stdout.ends_with(b"ok"));
    }
}
