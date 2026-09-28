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
//! `PhpFpmClient` — module-local FastCGI client.
//!
//! Default [`InertTransport`] is fail-closed. Production uses wire/unix transports via
//! [`crate::adapter::FcgiModuleExecutor`]. [`crate::ScriptedFpmTransport`] is for protocol unit tests.

use crate::encode::{
    decode_forward_response, encode_params_frames, encode_stdin_frames, DecodeError, EncodeError,
};
use crate::params::{MinForwardRequest, ParamsError};
use crate::scripted::ScriptedFpmTransport;
use crate::transport::{FastcgiRecordTransport, InertTransport, TransportError};
use crate::wire::{WireError, WireTransport};

/// Client-level errors (no live 502/503/504 mapping in PR4-A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    InertUnavailable,
    Transport(TransportError),
    Encode(EncodeError),
    Decode(DecodeError),
    Params(ParamsError),
    Wire(WireError),
}

/// Compile-time pool label for planning — not a live socket endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolLabel {
    pub name: &'static str,
}

/// One completed in-memory forward (stdout + END_REQUEST fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardResponse {
    pub stdout: Vec<u8>,
    pub app_status: u32,
    pub protocol_status: u8,
}

/// PHP-FPM client bound to a transport implementation.
#[derive(Debug)]
pub struct PhpFpmClient<T = InertTransport> {
    pool: PoolLabel,
    transport: T,
}

impl PhpFpmClient<InertTransport> {
    /// Create an inert client bound to a static pool name (metadata only).
    pub fn new(pool_name: &'static str) -> Self {
        Self::with_transport(pool_name, InertTransport)
    }
}

impl<T> PhpFpmClient<T> {
    /// Create a client with an injected transport (scripted, inert, or wire).
    pub fn with_transport(pool_name: &'static str, transport: T) -> Self {
        Self {
            pool: PoolLabel { name: pool_name },
            transport,
        }
    }

    /// Static pool label configured at construction (not a live endpoint).
    pub fn pool_label(&self) -> PoolLabel {
        self.pool
    }
}

impl<T: FastcgiRecordTransport> PhpFpmClient<T> {
    /// Submit one encoded record via the transport seam (in-memory only).
    pub fn submit_frame(&self, frame: &[u8]) -> Result<(), ClientError> {
        self.transport
            .submit_frame(frame)
            .map_err(ClientError::Transport)
    }
}

impl PhpFpmClient<InertTransport> {
    /// Request forwarding — PR3-A/PR4-A inert default: always [`ClientError::InertUnavailable`].
    pub fn forward_request(&self, _request_frames: &[&[u8]]) -> Result<(), ClientError> {
        Err(ClientError::InertUnavailable)
    }
}

impl PhpFpmClient<ScriptedFpmTransport> {
    /// Encode PARAMS + STDIN, drive the scripted transport, decode STDOUT + END_REQUEST.
    ///
    /// Uses the scripted peer's configured `request_id`. Does not emit `BEGIN_REQUEST`.
    pub fn forward_once(
        &self,
        params: &[(&str, &str)],
        stdin: &[u8],
    ) -> Result<ForwardResponse, ClientError> {
        self.transport.reset();
        let request_id = self.transport.request_id();

        for frame in encode_params_frames(request_id, params).map_err(ClientError::Encode)? {
            self.submit_frame(&frame)?;
        }
        for frame in encode_stdin_frames(request_id, stdin).map_err(ClientError::Encode)? {
            self.submit_frame(&frame)?;
        }

        let response_frames = self
            .transport
            .take_response()
            .map_err(ClientError::Transport)?;
        let decoded = decode_forward_response(&response_frames).map_err(ClientError::Decode)?;

        Ok(ForwardResponse {
            stdout: decoded.stdout,
            app_status: decoded.app_status,
            protocol_status: decoded.protocol_status,
        })
    }
}

impl PhpFpmClient<WireTransport> {
    /// Wire forward via unix/tcp — PR5-A-min module boundary only.
    pub fn forward_min_request(
        &self,
        request: &MinForwardRequest,
    ) -> Result<ForwardResponse, ClientError> {
        let params = request.to_fcgi_params().map_err(ClientError::Params)?;
        self.transport
            .forward_once(&params, &request.stdin)
            .map_err(ClientError::Wire)
    }

    /// Wire forward with explicit owned PARAMS pairs.
    pub fn forward_wire_once(
        &self,
        params: &[(String, String)],
        stdin: &[u8],
    ) -> Result<ForwardResponse, ClientError> {
        self.transport
            .forward_once(params, stdin)
            .map_err(ClientError::Wire)
    }
}

impl Default for PhpFpmClient<InertTransport> {
    fn default() -> Self {
        Self::new("default")
    }
}
