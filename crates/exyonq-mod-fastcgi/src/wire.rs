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
//! Real unix/tcp wire transport — PR5-A-min module tests; unix delegates to [`UnixFpmTransport`].

use crate::caps::{
    check_add_cap, FCGI_CONNECT_TIMEOUT, FCGI_READ_TIMEOUT, FCGI_WRITE_TIMEOUT,
    MAX_FCGI_RESPONSE_BYTES,
};
use crate::client::ForwardResponse;
use crate::decode_forward_response;
use crate::encode::{
    encode_begin_request_frame, encode_params_frames_owned, encode_stdin_frames, DecodeError,
    EncodeError,
};
use crate::parser::{parse_record, ParseError};
use crate::record::{FCGI_END_REQUEST, FCGI_STDOUT, RECORD_HEADER_LEN};
use crate::transport::TransportError;
#[cfg(unix)]
use crate::unix_transport::UnixFpmTransport;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// Wire endpoint for module-level forward (tests and future composition).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireEndpoint {
    Unix(PathBuf),
    Tcp { host: String, port: u16 },
}

/// Real wire transport configuration — connect per forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTransport {
    pub endpoint: WireEndpoint,
    pub request_id: u16,
}

impl WireTransport {
    pub fn unix(path: PathBuf, request_id: u16) -> Self {
        Self {
            endpoint: WireEndpoint::Unix(path),
            request_id,
        }
    }

    pub fn tcp(host: impl Into<String>, port: u16, request_id: u16) -> Self {
        Self {
            endpoint: WireEndpoint::Tcp {
                host: host.into(),
                port,
            },
            request_id,
        }
    }

    /// Encode PARAMS + STDIN, write to peer, read bounded response until END_REQUEST.
    pub fn forward_once(
        &self,
        params: &[(String, String)],
        stdin: &[u8],
    ) -> Result<ForwardResponse, WireError> {
        match &self.endpoint {
            WireEndpoint::Unix(path) => {
                #[cfg(unix)]
                {
                    let transport =
                        UnixFpmTransport::new(path.clone(), self.request_id, FCGI_CONNECT_TIMEOUT);
                    transport.forward_once(params, stdin)
                }
                #[cfg(not(unix))]
                {
                    let _ = path;
                    Err(WireError::ConnectionFailed)
                }
            }
            WireEndpoint::Tcp { host, port } => self.forward_once_tcp(host, *port, params, stdin),
        }
    }

    fn forward_once_tcp(
        &self,
        host: &str,
        port: u16,
        params: &[(String, String)],
        stdin: &[u8],
    ) -> Result<ForwardResponse, WireError> {
        let begin = encode_begin_request_frame(self.request_id).map_err(WireError::Encode)?;
        let param_frames =
            encode_params_frames_owned(self.request_id, params).map_err(WireError::Encode)?;
        let stdin_frames =
            encode_stdin_frames(self.request_id, stdin).map_err(WireError::Encode)?;

        let mut stream = self.connect_tcp(host, port)?;
        write_all_timeout(&mut stream, &begin)?;
        for frame in param_frames.iter().chain(stdin_frames.iter()) {
            write_all_timeout(&mut stream, frame)?;
        }

        let response_frames = read_response_frames(&mut stream)?;
        let decoded = decode_forward_response(&response_frames).map_err(WireError::Decode)?;

        Ok(ForwardResponse {
            stdout: decoded.stdout,
            app_status: decoded.app_status,
            protocol_status: decoded.protocol_status,
        })
    }

    fn connect_tcp(&self, host: &str, port: u16) -> Result<WireStream, WireError> {
        let addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .map_err(|_| WireError::ConnectionFailed)?;
        let stream = TcpStream::connect_timeout(&addr, FCGI_CONNECT_TIMEOUT).map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                WireError::Timeout
            } else {
                WireError::ConnectionFailed
            }
        })?;
        configure_timeouts_tcp(&stream)?;
        Ok(WireStream::Tcp(stream))
    }
}

enum WireStream {
    #[cfg(unix)]
    #[allow(dead_code)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Read for WireStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            WireStream::Unix(s) => s.read(buf),
            WireStream::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for WireStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            WireStream::Unix(s) => s.write(buf),
            WireStream::Tcp(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            WireStream::Unix(s) => s.flush(),
            WireStream::Tcp(s) => s.flush(),
        }
    }
}

fn configure_timeouts_tcp(stream: &TcpStream) -> Result<(), WireError> {
    stream
        .set_read_timeout(Some(FCGI_READ_TIMEOUT))
        .map_err(|_| WireError::IoFailed)?;
    stream
        .set_write_timeout(Some(FCGI_WRITE_TIMEOUT))
        .map_err(|_| WireError::IoFailed)?;
    Ok(())
}

fn write_all_timeout(stream: &mut WireStream, frame: &[u8]) -> Result<(), WireError> {
    let mut offset = 0;
    while offset < frame.len() {
        match stream.write(&frame[offset..]) {
            Ok(0) => return Err(WireError::ConnectionClosed),
            Ok(n) => offset += n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Err(WireError::Timeout),
            Err(_) => return Err(WireError::IoFailed),
        }
    }
    stream.flush().map_err(|_| WireError::IoFailed)?;
    Ok(())
}

fn read_response_frames(stream: &mut WireStream) -> Result<Vec<Vec<u8>>, WireError> {
    let mut buffer = Vec::new();
    let mut scratch = [0u8; 4096];
    let mut stdout_bytes = 0usize;
    let mut frames = Vec::new();
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
            if parsed.header.record_type == FCGI_STDOUT {
                stdout_bytes =
                    check_add_cap(MAX_FCGI_RESPONSE_BYTES, stdout_bytes, parsed.content.len())
                        .map_err(|_| WireError::ResponseCapExceeded)?;
            }
            if parsed.header.record_type == FCGI_END_REQUEST {
                saw_end_request = true;
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
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Err(WireError::Timeout),
            Err(_) => return Err(WireError::IoFailed),
        };
        buffer.extend_from_slice(&scratch[..n]);
    }

    if frames.is_empty() {
        return Err(WireError::ConnectionClosed);
    }
    Ok(frames)
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

/// Wire-level errors (mapped to [`TransportError`] at client boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    ConnectionFailed,
    ConnectionClosed,
    Timeout,
    ResponseCapExceeded,
    IoFailed,
    /// Connection pool at `max_connections` with no idle socket available.
    PoolBusy,
    Encode(EncodeError),
    Decode(DecodeError),
    InvalidFrame(ParseError),
    WrongRequestId {
        expected: u16,
        got: u16,
    },
}

impl From<WireError> for TransportError {
    fn from(err: WireError) -> Self {
        match err {
            WireError::ConnectionFailed => TransportError::ConnectionFailed,
            WireError::ConnectionClosed => TransportError::ConnectionClosed,
            WireError::Timeout => TransportError::Timeout,
            WireError::ResponseCapExceeded => TransportError::ResponseCapExceeded,
            WireError::IoFailed => TransportError::IoFailed,
            WireError::PoolBusy => TransportError::ConnectionFailed,
            WireError::InvalidFrame(e) => TransportError::InvalidFrame(e),
            WireError::WrongRequestId { expected, got } => {
                TransportError::WrongRequestId { expected, got }
            }
            WireError::Encode(_) => TransportError::EncodeFailed,
            WireError::Decode(_) => TransportError::RequestIncomplete,
        }
    }
}
