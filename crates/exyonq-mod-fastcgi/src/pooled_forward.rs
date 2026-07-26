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
//! Budgeted FastCGI forward with explicit commit stages (UDS/TCP shared).

use crate::caps::{
    check_add_cap, MAX_FCGI_RECORDS, MAX_FCGI_RESPONSE_BYTES, MAX_FCGI_STDERR_BYTES,
};
use crate::client::ForwardResponse;
use crate::commit::CommitStage;
use crate::decode_forward_response_with_stderr;
use crate::encode::{
    encode_begin_request_frame_with_flags, encode_params_frames_owned, encode_stdin_frames,
    DecodeError,
};
use crate::fcgi_stream::FcgiStream;
use crate::parser::{parse_record, ParseError};
use crate::record::{
    FCGI_BEGIN_REQUEST, FCGI_END_REQUEST, FCGI_PARAMS, FCGI_STDERR, FCGI_STDIN, FCGI_STDOUT,
    FCGI_VERSION_1, RECORD_HEADER_LEN,
};
use crate::timeout_budget::TimeoutBudget;
use crate::wire::WireError;
use std::io::{Read, Write};

/// Outcome of one forward attempt including how far the wire progressed.
#[derive(Debug)]
pub struct ForwardAttempt {
    pub stage: CommitStage,
    pub result: Result<ForwardResponse, WireError>,
}

/// Run one FastCGI exchange under an absolute timeout budget.
pub fn forward_on_stream_budgeted(
    stream: &mut FcgiStream,
    request_id: u16,
    begin_flags: u8,
    params: &[(String, String)],
    stdin: &[u8],
    budget: &TimeoutBudget,
) -> ForwardAttempt {
    let mut stage = CommitStage::NotStarted;

    let begin = match encode_begin_request_frame_with_flags(request_id, begin_flags) {
        Ok(f) => f,
        Err(e) => {
            return ForwardAttempt {
                stage,
                result: Err(WireError::Encode(e)),
            };
        }
    };
    let param_frames = match encode_params_frames_owned(request_id, params) {
        Ok(f) => f,
        Err(e) => {
            return ForwardAttempt {
                stage,
                result: Err(WireError::Encode(e)),
            };
        }
    };
    let stdin_frames = match encode_stdin_frames(request_id, stdin) {
        Ok(f) => f,
        Err(e) => {
            return ForwardAttempt {
                stage,
                result: Err(WireError::Encode(e)),
            };
        }
    };

    if let Err(e) = apply_write_budget(stream, budget) {
        return ForwardAttempt {
            stage,
            result: Err(e),
        };
    }

    match write_all_timeout(stream, &begin) {
        Ok(()) => {
            stage = CommitStage::BeginCommitted;
        }
        Err((e, wrote_any)) => {
            if wrote_any {
                stage = CommitStage::BeginCommitted;
            }
            return ForwardAttempt {
                stage,
                result: Err(e),
            };
        }
    }

    for frame in &param_frames {
        if let Err(e) = apply_write_budget(stream, budget) {
            return ForwardAttempt {
                stage,
                result: Err(e),
            };
        }
        match write_all_timeout(stream, frame) {
            Ok(()) => {}
            Err((e, _)) => {
                return ForwardAttempt {
                    stage,
                    result: Err(e),
                };
            }
        }
    }
    stage = CommitStage::ParamsCommitted;

    for frame in &stdin_frames {
        if let Err(e) = apply_write_budget(stream, budget) {
            return ForwardAttempt {
                stage,
                result: Err(e),
            };
        }
        match write_all_timeout(stream, frame) {
            Ok(()) => {}
            Err((e, _)) => {
                return ForwardAttempt {
                    stage,
                    result: Err(e),
                };
            }
        }
    }
    stage = CommitStage::BodyCommitted;

    if let Err(e) = apply_read_budget(stream, budget) {
        return ForwardAttempt {
            stage,
            result: Err(e),
        };
    }

    match read_response_frames(stream, request_id, &mut stage) {
        Ok(frames) => match decode_forward_response_with_stderr(&frames, true) {
            Ok(decoded) => ForwardAttempt {
                stage,
                result: Ok(ForwardResponse {
                    stdout: decoded.stdout,
                    app_status: decoded.app_status,
                    protocol_status: decoded.protocol_status,
                }),
            },
            Err(e) => ForwardAttempt {
                stage,
                result: Err(WireError::Decode(e)),
            },
        },
        Err(e) => ForwardAttempt {
            stage,
            result: Err(e),
        },
    }
}

fn apply_write_budget(stream: &mut FcgiStream, budget: &TimeoutBudget) -> Result<(), WireError> {
    let t = budget.write_budget()?;
    stream.apply_rw_timeouts(t)
}

fn apply_read_budget(stream: &mut FcgiStream, budget: &TimeoutBudget) -> Result<(), WireError> {
    let t = budget.read_budget()?;
    stream.apply_rw_timeouts(t)
}

fn is_socket_timeout(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) || err.raw_os_error() == Some(60)
}

fn write_all_timeout(stream: &mut FcgiStream, frame: &[u8]) -> Result<(), (WireError, bool)> {
    let mut offset = 0;
    let mut wrote_any = false;
    while offset < frame.len() {
        match stream.write(&frame[offset..]) {
            Ok(0) => return Err((WireError::ConnectionClosed, wrote_any)),
            Ok(n) => {
                wrote_any = true;
                offset += n;
            }
            Err(e) if is_socket_timeout(&e) => return Err((WireError::Timeout, wrote_any)),
            Err(_) => return Err((WireError::IoFailed, wrote_any)),
        }
    }
    stream.flush().map_err(|e| {
        let err = if is_socket_timeout(&e) {
            WireError::Timeout
        } else {
            WireError::IoFailed
        };
        (err, wrote_any)
    })?;
    Ok(())
}

fn read_response_frames(
    stream: &mut FcgiStream,
    request_id: u16,
    stage: &mut CommitStage,
) -> Result<Vec<Vec<u8>>, WireError> {
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

            let parsed = parse_record(&frame).map_err(WireError::InvalidFrame)?;
            validate_response_record(&parsed.header, request_id)?;

            record_count = check_add_cap(MAX_FCGI_RECORDS, record_count, 1)
                .map_err(|_| WireError::ResponseCapExceeded)?;

            match parsed.header.record_type {
                FCGI_STDOUT => {
                    stdout_bytes =
                        check_add_cap(MAX_FCGI_RESPONSE_BYTES, stdout_bytes, parsed.content.len())
                            .map_err(|_| WireError::ResponseCapExceeded)?;
                }
                FCGI_STDERR => {
                    stderr_bytes =
                        check_add_cap(MAX_FCGI_STDERR_BYTES, stderr_bytes, parsed.content.len())
                            .map_err(|_| WireError::ResponseCapExceeded)?;
                }
                FCGI_END_REQUEST => {
                    saw_end_request = true;
                }
                FCGI_BEGIN_REQUEST | FCGI_PARAMS | FCGI_STDIN => {
                    return Err(WireError::Decode(DecodeError::UnexpectedRecordType {
                        got: parsed.header.record_type,
                    }));
                }
                other => {
                    return Err(WireError::Decode(DecodeError::UnexpectedRecordType {
                        got: other,
                    }));
                }
            }
            frames.push(frame);
        }

        if saw_end_request {
            break;
        }

        let n = match stream.read(&mut scratch) {
            Ok(0) => {
                if frames.is_empty() {
                    return Err(WireError::ConnectionClosed);
                }
                break;
            }
            Ok(n) => {
                *stage = CommitStage::ResponseStarted;
                n
            }
            Err(e) if is_socket_timeout(&e) => return Err(WireError::Timeout),
            Err(_) => return Err(WireError::IoFailed),
        };
        buffer.extend_from_slice(&scratch[..n]);
    }

    if !saw_end_request {
        return Err(WireError::Decode(DecodeError::MissingEndRequest));
    }
    Ok(frames)
}

fn validate_response_record(
    header: &crate::record::RecordHeader,
    expected_id: u16,
) -> Result<(), WireError> {
    if header.version != FCGI_VERSION_1 {
        return Err(WireError::InvalidFrame(ParseError::UnsupportedVersion(
            header.version,
        )));
    }
    if header.request_id != expected_id {
        return Err(WireError::WrongRequestId {
            expected: expected_id,
            got: header.request_id,
        });
    }
    Ok(())
}

fn header_frame_len(buf: &[u8]) -> Result<usize, WireError> {
    if buf.len() < RECORD_HEADER_LEN {
        return Err(WireError::InvalidFrame(ParseError::BufferTooShort {
            need: RECORD_HEADER_LEN,
            have: buf.len(),
        }));
    }
    let content_len = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    let padding = buf[6] as usize;
    Ok(RECORD_HEADER_LEN + content_len + padding)
}
