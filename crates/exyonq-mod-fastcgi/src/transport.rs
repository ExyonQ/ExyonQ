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
//! Transport trait surface — module-local until PR-7 composition.
//!
//! Live sockets use [`crate::wire::WireTransport`] / [`crate::unix_transport::UnixFpmTransport`].
//! [`InertTransport`] is fail-closed when no wire is bound. [`crate::ScriptedFpmTransport`] is an
//! in-memory record peer for protocol unit tests only (not a product substitute).

use crate::ParseError;

/// Transport-level errors for the module seam (no live HTTP mapping in PR4-A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// Transport not wired — fail-closed for [`InertTransport`] (not used by production executor).
    InertUnavailable,
    /// Encoded frame failed bounded record validation before submit.
    InvalidFrame(ParseError),
    /// Responder frame encoding failed (scripted peer internal).
    EncodeFailed,
    /// Record type not expected in the current scripted peer phase.
    UnexpectedRecordType { phase: &'static str, got: u8 },
    /// `request_id` mismatch vs scripted peer configuration.
    WrongRequestId { expected: u16, got: u16 },
    /// PARAMS/STDIN stream not finished before response take.
    RequestIncomplete,
    /// Response taken twice.
    ResponseAlreadyTaken,
    /// Request stream already complete; extra frame rejected.
    RequestAlreadyComplete,
    /// Wire connect failed (PR5-A-min).
    ConnectionFailed,
    /// Peer closed connection before complete response (PR5-A-min).
    ConnectionClosed,
    /// Wire read/write timeout (PR5-A-min).
    Timeout,
    /// Aggregate STDOUT cap exceeded (PR5-A-min).
    ResponseCapExceeded,
    /// Underlying I/O error on wire (PR5-A-min).
    IoFailed,
}

/// Module-local trait for submitting encoded FastCGI record frames (in-memory only).
pub trait FastcgiRecordTransport {
    /// Accept one encoded record frame. Implementations must not perform socket I/O.
    fn submit_frame(&self, frame: &[u8]) -> Result<(), TransportError>;
}

/// Default inert transport — always returns [`TransportError::InertUnavailable`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InertTransport;

impl FastcgiRecordTransport for InertTransport {
    fn submit_frame(&self, _frame: &[u8]) -> Result<(), TransportError> {
        Err(TransportError::InertUnavailable)
    }
}

/// Validates frames with the bounded record parser before accepting (unit-test helper).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ValidatingScriptedTransport;

impl FastcgiRecordTransport for ValidatingScriptedTransport {
    fn submit_frame(&self, frame: &[u8]) -> Result<(), TransportError> {
        crate::parse_record(frame)
            .map(|_| ())
            .map_err(TransportError::InvalidFrame)
    }
}
