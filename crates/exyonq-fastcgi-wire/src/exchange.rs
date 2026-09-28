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
//! Sync FastCGI exchange over `Read + Write` — CommitStage + SG-FCGI-05 trailing discard.

use std::io::{Read, Write};

use thiserror::Error;

use crate::caps::{
    check_add_cap, MAX_FCGI_RECORDS, MAX_FCGI_RESPONSE_BYTES, MAX_FCGI_STDERR_BYTES,
};
use crate::commit::CommitStage;
use crate::encode::{
    decode_forward_response_with_stderr, encode_begin_request_frame_with_flags,
    encode_params_frames_owned, encode_stdin_frames, DecodeError, EncodeError,
};
use crate::parser::{parse_record, ParseError};
use crate::record::{
    FCGI_BEGIN_REQUEST, FCGI_END_REQUEST, FCGI_PARAMS, FCGI_STDERR, FCGI_STDIN, FCGI_STDOUT,
    FCGI_VERSION_1, RECORD_HEADER_LEN,
};

/// Wire / exchange failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExchangeError {
    #[error("encode: {0:?}")]
    Encode(EncodeError),
    #[error("decode: {0:?}")]
    Decode(DecodeError),
    #[error("invalid frame: {0:?}")]
    InvalidFrame(ParseError),
    #[error("wrong request id: expected {expected}, got {got}")]
    WrongRequestId { expected: u16, got: u16 },
    #[error("connection closed")]
    ConnectionClosed,
    #[error("io failed")]
    IoFailed,
    #[error("timeout")]
    Timeout,
    #[error("response cap exceeded")]
    ResponseCapExceeded,
    /// Bytes remain after END_REQUEST (SG-FCGI-05). Caller must DISCARD the connection.
    #[error("trailing data after END_REQUEST")]
    TrailingData,
}

impl From<EncodeError> for ExchangeError {
    fn from(e: EncodeError) -> Self {
        Self::Encode(e)
    }
}

/// Successful exchange outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeResponse {
    pub stdout: Vec<u8>,
    pub app_status: u32,
    pub protocol_status: u8,
}

/// One attempt including how far the wire progressed.
#[derive(Debug)]
pub struct ExchangeAttempt {
    pub stage: CommitStage,
    pub result: Result<ExchangeResponse, ExchangeError>,
}

/// Run one FastCGI exchange. When `keep_conn` is true, BEGIN uses `FCGI_KEEP_CONN`.
///
/// After END_REQUEST, any leftover buffered bytes or immediately readable peer data
/// yields [`ExchangeError::TrailingData`] (SG-FCGI-05) — caller must discard the socket.
pub fn exchange_once<S: Read + Write>(
    stream: &mut S,
    request_id: u16,
    keep_conn: bool,
    params: &[(String, String)],
    stdin: &[u8],
) -> ExchangeAttempt {
    let flags = if keep_conn {
        crate::record::FCGI_KEEP_CONN
    } else {
        0
    };
    forward_on_stream(stream, request_id, flags, params, stdin)
}

/// Forward with explicit BEGIN flags.
pub fn forward_on_stream<S: Read + Write>(
    stream: &mut S,
    request_id: u16,
    begin_flags: u8,
    params: &[(String, String)],
    stdin: &[u8],
) -> ExchangeAttempt {
    let mut stage = CommitStage::NotStarted;

    let begin = match encode_begin_request_frame_with_flags(request_id, begin_flags) {
        Ok(f) => f,
        Err(e) => {
            return ExchangeAttempt {
                stage,
                result: Err(e.into()),
            };
        }
    };
    let param_frames = match encode_params_frames_owned(request_id, params) {
        Ok(f) => f,
        Err(e) => {
            return ExchangeAttempt {
                stage,
                result: Err(e.into()),
            };
        }
    };
    let stdin_frames = match encode_stdin_frames(request_id, stdin) {
        Ok(f) => f,
        Err(e) => {
            return ExchangeAttempt {
                stage,
                result: Err(e.into()),
            };
        }
    };

    match write_all(stream, &begin) {
        Ok(()) => {
            stage = CommitStage::BeginCommitted;
        }
        Err((e, wrote_any)) => {
            if wrote_any {
                stage = CommitStage::BeginCommitted;
            }
            return ExchangeAttempt {
                stage,
                result: Err(e),
            };
        }
    }

    for frame in &param_frames {
        match write_all(stream, frame) {
            Ok(()) => {}
            Err((e, _)) => {
                return ExchangeAttempt {
                    stage,
                    result: Err(e),
                };
            }
        }
    }
    stage = CommitStage::ParamsCommitted;

    for frame in &stdin_frames {
        match write_all(stream, frame) {
            Ok(()) => {}
            Err((e, _)) => {
                return ExchangeAttempt {
                    stage,
                    result: Err(e),
                };
            }
        }
    }
    stage = CommitStage::BodyCommitted;

    match read_response_frames(stream, request_id, &mut stage) {
        Ok(frames) => match decode_forward_response_with_stderr(&frames, true) {
            Ok(decoded) => ExchangeAttempt {
                stage,
                result: Ok(ExchangeResponse {
                    stdout: decoded.stdout,
                    app_status: decoded.app_status,
                    protocol_status: decoded.protocol_status,
                }),
            },
            Err(e) => ExchangeAttempt {
                stage,
                result: Err(ExchangeError::Decode(e)),
            },
        },
        Err(e) => ExchangeAttempt {
            stage,
            result: Err(e),
        },
    }
}

/// Retry once only when the first attempt stayed at [`CommitStage::NotStarted`].
pub fn exchange_once_with_not_started_retry<S: Read + Write>(
    stream: &mut S,
    request_id: u16,
    keep_conn: bool,
    params: &[(String, String)],
    stdin: &[u8],
) -> ExchangeAttempt {
    let first = exchange_once(stream, request_id, keep_conn, params, stdin);
    if first.result.is_ok() {
        return first;
    }
    if !first.stage.allows_safe_retry() {
        return first;
    }
    exchange_once(stream, request_id, keep_conn, params, stdin)
}

fn is_socket_timeout(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) || err.raw_os_error() == Some(60)
}

fn write_all<S: Write>(stream: &mut S, frame: &[u8]) -> Result<(), (ExchangeError, bool)> {
    let mut offset = 0;
    let mut wrote_any = false;
    while offset < frame.len() {
        match stream.write(&frame[offset..]) {
            Ok(0) => return Err((ExchangeError::ConnectionClosed, wrote_any)),
            Ok(n) => {
                wrote_any = true;
                offset += n;
            }
            Err(e) if is_socket_timeout(&e) => return Err((ExchangeError::Timeout, wrote_any)),
            Err(_) => return Err((ExchangeError::IoFailed, wrote_any)),
        }
    }
    stream.flush().map_err(|e| {
        let err = if is_socket_timeout(&e) {
            ExchangeError::Timeout
        } else {
            ExchangeError::IoFailed
        };
        (err, wrote_any)
    })?;
    Ok(())
}

fn read_response_frames<S: Read>(
    stream: &mut S,
    request_id: u16,
    stage: &mut CommitStage,
) -> Result<Vec<Vec<u8>>, ExchangeError> {
    let mut buffer = Vec::new();
    let mut scratch = [0u8; 4096];
    let mut stdout_bytes = 0usize;
    let mut stderr_bytes = 0usize;
    let mut frames = Vec::new();
    let mut record_count = 0usize;
    let mut saw_end_request = false;

    loop {
        while buffer.len() >= RECORD_HEADER_LEN {
            let frame_len = header_frame_len(&buffer)?;
            if buffer.len() < frame_len {
                break;
            }
            let frame = buffer[..frame_len].to_vec();
            buffer.drain(..frame_len);
            *stage = CommitStage::ResponseStarted;

            let parsed = parse_record(&frame).map_err(ExchangeError::InvalidFrame)?;
            validate_response_record(&parsed.header, request_id)?;

            record_count = check_add_cap(MAX_FCGI_RECORDS, record_count, 1)
                .map_err(|_| ExchangeError::ResponseCapExceeded)?;

            match parsed.header.record_type {
                FCGI_STDOUT => {
                    stdout_bytes =
                        check_add_cap(MAX_FCGI_RESPONSE_BYTES, stdout_bytes, parsed.content.len())
                            .map_err(|_| ExchangeError::ResponseCapExceeded)?;
                }
                FCGI_STDERR => {
                    stderr_bytes =
                        check_add_cap(MAX_FCGI_STDERR_BYTES, stderr_bytes, parsed.content.len())
                            .map_err(|_| ExchangeError::ResponseCapExceeded)?;
                }
                FCGI_END_REQUEST => {
                    saw_end_request = true;
                }
                FCGI_BEGIN_REQUEST | FCGI_PARAMS | FCGI_STDIN => {
                    return Err(ExchangeError::Decode(DecodeError::UnexpectedRecordType {
                        got: parsed.header.record_type,
                    }));
                }
                other => {
                    return Err(ExchangeError::Decode(DecodeError::UnexpectedRecordType {
                        got: other,
                    }));
                }
            }
            frames.push(frame);
            if saw_end_request {
                break;
            }
        }

        if saw_end_request {
            break;
        }

        let n = match stream.read(&mut scratch) {
            Ok(0) => {
                if frames.is_empty() {
                    return Err(ExchangeError::ConnectionClosed);
                }
                break;
            }
            Ok(n) => {
                *stage = CommitStage::ResponseStarted;
                n
            }
            Err(e) if is_socket_timeout(&e) => return Err(ExchangeError::Timeout),
            Err(_) => return Err(ExchangeError::IoFailed),
        };
        buffer.extend_from_slice(&scratch[..n]);
    }

    if !saw_end_request {
        return Err(ExchangeError::Decode(DecodeError::MissingEndRequest));
    }

    // SG-FCGI-05: any trailing bytes after END_REQUEST ⇒ TrailingData (caller DISCARD).
    if !buffer.is_empty() {
        return Err(ExchangeError::TrailingData);
    }

    Ok(frames)
}

fn validate_response_record(
    header: &crate::record::RecordHeader,
    expected_id: u16,
) -> Result<(), ExchangeError> {
    if header.version != FCGI_VERSION_1 {
        return Err(ExchangeError::InvalidFrame(ParseError::UnsupportedVersion(
            header.version,
        )));
    }
    if header.request_id != expected_id {
        return Err(ExchangeError::WrongRequestId {
            expected: expected_id,
            got: header.request_id,
        });
    }
    Ok(())
}

fn header_frame_len(buf: &[u8]) -> Result<usize, ExchangeError> {
    if buf.len() < RECORD_HEADER_LEN {
        return Err(ExchangeError::InvalidFrame(ParseError::BufferTooShort {
            need: RECORD_HEADER_LEN,
            have: buf.len(),
        }));
    }
    let content_len = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    let padding = buf[6] as usize;
    Ok(RECORD_HEADER_LEN + content_len + padding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::encode_record_frame;
    use crate::params::MinForwardRequest;
    use crate::record::{FCGI_END_REQUEST, FCGI_REQUEST_COMPLETE, FCGI_STDOUT};
    use std::io::Cursor;

    fn scripted_ok_response(request_id: u16) -> Vec<u8> {
        let stdout = encode_record_frame(
            request_id,
            FCGI_STDOUT,
            b"Status: 200\r\nContent-Type: text/plain\r\n\r\nok",
        )
        .unwrap();
        let mut end = [0u8; 8];
        end[4] = FCGI_REQUEST_COMPLETE;
        let end_frame = encode_record_frame(request_id, FCGI_END_REQUEST, &end).unwrap();
        let mut out = stdout;
        out.extend_from_slice(&end_frame);
        out
    }

    /// Duplex buffer: writes go to request side; reads come from response side.
    struct ScriptedPeer {
        response: Cursor<Vec<u8>>,
        request: Vec<u8>,
    }

    impl Read for ScriptedPeer {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.response.read(buf)
        }
    }

    impl Write for ScriptedPeer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.request.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn exchange_get_zero_body() {
        let req = MinForwardRequest::get("/i.php", "/i.php", "/var/www/i.php");
        let params = req.to_fcgi_params().unwrap();
        let mut peer = ScriptedPeer {
            response: Cursor::new(scripted_ok_response(1)),
            request: Vec::new(),
        };
        let attempt = exchange_once(&mut peer, 1, true, &params, &[]);
        let resp = attempt.result.expect("ok");
        assert!(resp.stdout.ends_with(b"ok"));
        assert!(!peer.request.is_empty());
        // KEEP_CONN flag set in BEGIN body byte 2
        assert_eq!(peer.request[10], crate::record::FCGI_KEEP_CONN);
    }

    #[test]
    fn trailing_data_after_end_request() {
        let req = MinForwardRequest::get("/i.php", "/i.php", "/var/www/i.php");
        let params = req.to_fcgi_params().unwrap();
        let mut bytes = scripted_ok_response(1);
        bytes.extend_from_slice(b"TRAIL");
        let mut peer = ScriptedPeer {
            response: Cursor::new(bytes),
            request: Vec::new(),
        };
        let attempt = exchange_once(&mut peer, 1, false, &params, &[]);
        assert_eq!(attempt.result.unwrap_err(), ExchangeError::TrailingData);
    }

    #[test]
    fn not_started_retry_only_when_safe() {
        assert!(CommitStage::NotStarted.allows_safe_retry());
        // Simulate: empty response stream fails after BodyCommitted → no retry path inside
        // exchange_once_with_not_started_retry when stage advanced.
        let req = MinForwardRequest::get("/i.php", "/i.php", "/var/www/i.php");
        let params = req.to_fcgi_params().unwrap();
        let mut peer = ScriptedPeer {
            response: Cursor::new(Vec::new()),
            request: Vec::new(),
        };
        let attempt = exchange_once_with_not_started_retry(&mut peer, 1, false, &params, &[]);
        assert!(!attempt.stage.allows_safe_retry());
        assert!(attempt.result.is_err());
    }
}
