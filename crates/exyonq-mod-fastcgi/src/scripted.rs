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
//! In-memory scripted FPM transport — PR4-A roundtrip without sockets.

use crate::encode::encode_record_frame;
use crate::parser::parse_record;
use crate::record::{
    END_REQUEST_BODY_LEN, FCGI_BEGIN_REQUEST, FCGI_END_REQUEST, FCGI_PARAMS, FCGI_REQUEST_COMPLETE,
    FCGI_STDIN, FCGI_STDOUT,
};
use crate::transport::{FastcgiRecordTransport, TransportError};
use std::cell::RefCell;

/// Deterministic PR5-B1 scripted body bytes.
pub const PR5B1_BODY: &[u8] = b"exyonq-fastcgi-pr5b1";

/// Deterministic PR5-B1 CGI stdout (Status + headers + body).
pub const PR5B1_STDOUT: &[u8] =
    b"Status: 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 19\r\n\r\nexyonq-fastcgi-pr5b1";

/// Responder configuration for [`ScriptedFpmTransport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptedFpmConfig {
    pub request_id: u16,
    pub stdout_body: Vec<u8>,
    pub app_status: u32,
    pub protocol_status: u8,
}

impl Default for ScriptedFpmConfig {
    fn default() -> Self {
        Self::pr5b1_default()
    }
}

impl ScriptedFpmConfig {
    pub fn pr5b1_default() -> Self {
        Self {
            request_id: 1,
            stdout_body: PR5B1_STDOUT.to_vec(),
            app_status: 0,
            protocol_status: FCGI_REQUEST_COMPLETE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptedPhase {
    Begin,
    Params,
    Stdin,
    Ready,
    Taken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScriptedState {
    phase: ScriptedPhase,
}

/// Stateful in-memory FPM scripted — accumulates PARAMS/STDIN, returns STDOUT + END_REQUEST.
#[derive(Debug, Clone)]
pub struct ScriptedFpmTransport {
    config: ScriptedFpmConfig,
    state: RefCell<ScriptedState>,
}

impl ScriptedFpmTransport {
    pub fn new(config: ScriptedFpmConfig) -> Self {
        Self {
            config,
            state: RefCell::new(ScriptedState {
                phase: ScriptedPhase::Begin,
            }),
        }
    }

    /// Fixed request id for this scripted (PR4-A: no `BEGIN_REQUEST`).
    pub fn request_id(&self) -> u16 {
        self.config.request_id
    }

    /// Reset request accumulation for another forward cycle.
    pub fn reset(&self) {
        *self.state.borrow_mut() = ScriptedState {
            phase: ScriptedPhase::Begin,
        };
    }

    /// Pre-built responder frames: STDOUT(body), empty STDOUT, END_REQUEST.
    pub fn build_response_frames(
        config: &ScriptedFpmConfig,
    ) -> Result<Vec<Vec<u8>>, TransportError> {
        let id = config.request_id;
        let mut frames = Vec::new();
        if !config.stdout_body.is_empty() {
            frames.push(
                encode_record_frame(id, FCGI_STDOUT, &config.stdout_body)
                    .map_err(|_| TransportError::EncodeFailed)?,
            );
        }
        frames.push(
            encode_record_frame(id, FCGI_STDOUT, &[]).map_err(|_| TransportError::EncodeFailed)?,
        );
        let mut end_body = [0u8; END_REQUEST_BODY_LEN];
        end_body[0..4].copy_from_slice(&config.app_status.to_be_bytes());
        end_body[4] = config.protocol_status;
        frames.push(
            encode_record_frame(id, FCGI_END_REQUEST, &end_body)
                .map_err(|_| TransportError::EncodeFailed)?,
        );
        Ok(frames)
    }

    /// Take responder frames after request stream is complete (empty STDIN received).
    pub fn take_response(&self) -> Result<Vec<Vec<u8>>, TransportError> {
        let phase = self.state.borrow().phase;
        match phase {
            ScriptedPhase::Ready => {
                self.state.borrow_mut().phase = ScriptedPhase::Taken;
                Self::build_response_frames(&self.config)
            }
            ScriptedPhase::Taken => Err(TransportError::ResponseAlreadyTaken),
            _ => Err(TransportError::RequestIncomplete),
        }
    }

    fn accept_frame(&self, frame: &[u8]) -> Result<(), TransportError> {
        let parsed = parse_record(frame).map_err(TransportError::InvalidFrame)?;
        if parsed.header.request_id != self.config.request_id {
            return Err(TransportError::WrongRequestId {
                expected: self.config.request_id,
                got: parsed.header.request_id,
            });
        }

        let mut state = self.state.borrow_mut();
        match state.phase {
            ScriptedPhase::Begin => match parsed.header.record_type {
                FCGI_BEGIN_REQUEST => {
                    state.phase = ScriptedPhase::Params;
                }
                FCGI_PARAMS => {
                    if parsed.content.is_empty() {
                        state.phase = ScriptedPhase::Stdin;
                    } else {
                        state.phase = ScriptedPhase::Params;
                    }
                }
                other => {
                    return Err(TransportError::UnexpectedRecordType {
                        phase: "begin",
                        got: other,
                    });
                }
            },
            ScriptedPhase::Params => match parsed.header.record_type {
                FCGI_PARAMS => {
                    if parsed.content.is_empty() {
                        state.phase = ScriptedPhase::Stdin;
                    }
                }
                other => {
                    return Err(TransportError::UnexpectedRecordType {
                        phase: "params",
                        got: other,
                    });
                }
            },
            ScriptedPhase::Stdin => match parsed.header.record_type {
                FCGI_STDIN => {
                    if parsed.content.is_empty() {
                        state.phase = ScriptedPhase::Ready;
                    }
                }
                other => {
                    return Err(TransportError::UnexpectedRecordType {
                        phase: "stdin",
                        got: other,
                    });
                }
            },
            ScriptedPhase::Ready | ScriptedPhase::Taken => {
                return Err(TransportError::RequestAlreadyComplete);
            }
        }
        Ok(())
    }
}

impl FastcgiRecordTransport for ScriptedFpmTransport {
    fn submit_frame(&self, frame: &[u8]) -> Result<(), TransportError> {
        self.accept_frame(frame)
    }
}

impl Default for ScriptedFpmTransport {
    fn default() -> Self {
        Self::new(ScriptedFpmConfig::default())
    }
}
