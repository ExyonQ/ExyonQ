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
//! Unix domain socket PHP-FPM transport — PR5-B1-real production path.

use crate::caps::{
    check_add_cap, FCGI_CONNECT_TIMEOUT, FCGI_READ_TIMEOUT, FCGI_WRITE_TIMEOUT, MAX_FCGI_RECORDS,
    MAX_FCGI_RESPONSE_BYTES, MAX_FCGI_STDERR_BYTES,
};
use crate::client::ForwardResponse;
use crate::decode_forward_response_with_stderr;
use crate::encode::{
    encode_begin_request_frame_with_flags, encode_params_frames_owned, encode_stdin_frames,
    DecodeError,
};
use crate::parser::{parse_record, ParseError};
use crate::record::{
    FCGI_BEGIN_REQUEST, FCGI_END_REQUEST, FCGI_PARAMS, FCGI_STDERR, FCGI_STDIN, FCGI_STDOUT,
    FCGI_VERSION_1, RECORD_HEADER_LEN,
};
use crate::unix_connect::connect_unix_stream;
use crate::wire::WireError;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// Production Unix socket transport — one connection per request (no pooling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnixFpmTransport {
    pub socket_path: PathBuf,
    pub request_id: u16,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
}

impl UnixFpmTransport {
    pub fn new(socket_path: PathBuf, request_id: u16, connect_timeout: Duration) -> Self {
        Self {
            socket_path,
            request_id,
            connect_timeout,
            read_timeout: FCGI_READ_TIMEOUT,
            write_timeout: FCGI_WRITE_TIMEOUT,
        }
    }

    pub fn with_timeouts(
        socket_path: PathBuf,
        request_id: u16,
        connect_timeout: Duration,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Self {
        Self {
            socket_path,
            request_id,
            connect_timeout,
            read_timeout,
            write_timeout,
        }
    }

    /// Full FastCGI forward: BEGIN_REQUEST + PARAMS + STDIN → STDOUT/STDERR + END_REQUEST.
    pub fn forward_once(
        &self,
        params: &[(String, String)],
        stdin: &[u8],
    ) -> Result<ForwardResponse, WireError> {
        let mut stream = self.connect()?;
        forward_on_stream(&mut stream, self.request_id, 0, params, stdin)
    }

    /// Connect with configured timeouts (pool checkout).
    pub fn connect_only(&self) -> Result<UnixStream, WireError> {
        self.connect()
    }

    fn connect(&self) -> Result<UnixStream, WireError> {
        let stream = connect_unix_stream(&self.socket_path, self.connect_timeout)?;
        stream
            .set_read_timeout(Some(self.read_timeout))
            .map_err(|_| WireError::IoFailed)?;
        stream
            .set_write_timeout(Some(self.write_timeout))
            .map_err(|_| WireError::IoFailed)?;
        Ok(stream)
    }
}

/// Run one FastCGI exchange on an existing stream (`flags` typically `0` or `FCGI_KEEP_CONN`).
pub fn forward_on_stream(
    stream: &mut UnixStream,
    request_id: u16,
    begin_flags: u8,
    params: &[(String, String)],
    stdin: &[u8],
) -> Result<ForwardResponse, WireError> {
    let mut frames = Vec::new();
    frames.push(
        encode_begin_request_frame_with_flags(request_id, begin_flags)
            .map_err(WireError::Encode)?,
    );
    frames.extend(encode_params_frames_owned(request_id, params).map_err(WireError::Encode)?);
    frames.extend(encode_stdin_frames(request_id, stdin).map_err(WireError::Encode)?);

    for frame in &frames {
        write_all_timeout(stream, frame)?;
    }

    let response_frames = read_response_frames(stream, request_id)?;
    let decoded =
        decode_forward_response_with_stderr(&response_frames, true).map_err(WireError::Decode)?;

    Ok(ForwardResponse {
        stdout: decoded.stdout,
        app_status: decoded.app_status,
        protocol_status: decoded.protocol_status,
    })
}

impl Default for UnixFpmTransport {
    fn default() -> Self {
        Self::new(PathBuf::from("/run/php-fpm.sock"), 1, FCGI_CONNECT_TIMEOUT)
    }
}

fn is_socket_timeout(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) || err.raw_os_error() == Some(60)
}

fn write_all_timeout(stream: &mut UnixStream, frame: &[u8]) -> Result<(), WireError> {
    let mut offset = 0;
    while offset < frame.len() {
        match stream.write(&frame[offset..]) {
            Ok(0) => return Err(WireError::ConnectionClosed),
            Ok(n) => offset += n,
            Err(e) if is_socket_timeout(&e) => return Err(WireError::Timeout),
            Err(_) => return Err(WireError::IoFailed),
        }
    }
    stream.flush().map_err(|e| {
        if is_socket_timeout(&e) {
            WireError::Timeout
        } else {
            WireError::IoFailed
        }
    })?;
    Ok(())
}

fn read_response_frames(
    stream: &mut UnixStream,
    request_id: u16,
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
            Ok(n) => n,
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
