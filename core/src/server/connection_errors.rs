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
//! PS3A-F3 I2 — separated connection / worker / policy error classes.
//!
//! **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API** (outcome/error surface for facade).
//! IU2: accepted-socket timeout/WouldBlock → [`ConnectionError`] → worker continues.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::fmt;
use std::io;

/// Outcome of serving one accepted connection inside core policy.
#[derive(Debug)]
pub enum ConnectionServeOutcome {
    /// Connection finished (or was closed after connection-local handling).
    Completed,
    /// Connection-local failure; close client; worker must continue.
    ConnectionClosed(ConnectionError),
    /// Core policy invariant; close/fail-closed without killing worker by default.
    PolicyFailed(CorePolicyError),
}

/// Errors that close only the client connection (CONNECTION_LOCAL).
#[derive(Debug)]
pub enum ConnectionError {
    Io(io::Error),
    DrainRejected,
}

impl ConnectionError {
    pub fn from_io(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "connection io: {e}"),
            Self::DrainRejected => write!(f, "drain rejected"),
        }
    }
}

/// Worker / listener / epoll / ring infrastructure failures (WORKER_FATAL).
#[derive(Debug)]
pub(crate) enum WorkerMechanismError {
    Io(io::Error),
}

impl fmt::Display for WorkerMechanismError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "worker mechanism: {e}"),
        }
    }
}

impl From<io::Error> for WorkerMechanismError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Core policy invariants (not worker-fatal by default).
#[derive(Debug)]
pub enum CorePolicyError {
    WirePlanning,
    HandoffConstruction(io::Error),
    GenerationUnavailable,
}

impl fmt::Display for CorePolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WirePlanning => write!(f, "wire planning failed"),
            Self::HandoffConstruction(e) => write!(f, "handoff construction: {e}"),
            Self::GenerationUnavailable => write!(f, "generation unavailable"),
        }
    }
}

/// Classify an I/O error from an accepted client socket.
///
/// - `Ok(ConnectionError)` → CONNECTION_LOCAL (IU2 path includes WouldBlock/TimedOut).
/// - `Err(io::Error)` → not connection-local (caller may escalate as worker mechanism).
#[cfg(target_os = "linux")]
pub(crate) fn classify_accepted_socket_io(err: io::Error) -> Result<ConnectionError, io::Error> {
    if crate::server::io::is_connection_level_io_error(&err) {
        Ok(ConnectionError::from_io(err))
    } else {
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2_connection_local_kinds_map_via_classifier() {
        #[cfg(target_os = "linux")]
        {
            for kind in [
                io::ErrorKind::WouldBlock,
                io::ErrorKind::TimedOut,
                io::ErrorKind::UnexpectedEof,
                io::ErrorKind::ConnectionReset,
                io::ErrorKind::BrokenPipe,
            ] {
                let mapped = classify_accepted_socket_io(io::Error::from(kind)).expect("local");
                assert!(matches!(mapped, ConnectionError::Io(_)));
            }
            let escalate = classify_accepted_socket_io(io::Error::from(io::ErrorKind::AddrInUse));
            assert!(escalate.is_err());
        }
    }

    #[test]
    fn i2_outcome_variants_are_distinct() {
        let _ = ConnectionServeOutcome::Completed;
        let _ = ConnectionServeOutcome::ConnectionClosed(ConnectionError::DrainRejected);
        let _ = ConnectionServeOutcome::PolicyFailed(CorePolicyError::WirePlanning);
        let _ = WorkerMechanismError::from(io::Error::from(io::ErrorKind::Other));
    }
}
