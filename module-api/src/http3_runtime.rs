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
//! HTTP/3 dispatch contract (KD4.5) — no quinn/h3 types.

use async_trait::async_trait;
use bytes::Bytes;
use std::error::Error;

/// UDP listener + TLS binding for HTTP/3 (compiled at reload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Http3ListenerBinding {
    pub listen_addr: String,
    pub tls: super::tls_runtime::TlsListenerBinding,
}

/// Materialized HTTP response for H3 write path (streaming bodies collected at adapter boundary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Http3MaterializedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

pub type Http3DispatchError = Box<dyn Error + Send + Sync>;

/// Admission rejected while the kernel is draining (no connection counter increment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Http3DrainRejected;

/// Kernel-issued RAII lease for one admitted QUIC connection (PS1A-H3).
///
/// Implementations live in core; mod-http3 holds the lease for the connection task lifetime.
pub trait Http3ConnectionLifecycle: Send + Sync {
    /// Opaque lease type — must be `Send`, non-`Clone`, dropped exactly once per connection.
    type Lease: Send + 'static;

    /// Admit one QUIC connection after transport handshake (POST_HANDSHAKE policy).
    fn try_enter_connection(&self) -> Result<Self::Lease, Http3DrainRejected>;
}

/// Core-owned dispatch — H3 runtime calls this; no routing/policy in mod-http3.
#[async_trait]
pub trait Http3DispatchService: Send + Sync {
    async fn dispatch(
        &self,
        req: http::Request<Bytes>,
        peer_ip: &str,
    ) -> Result<Http3MaterializedResponse, Http3DispatchError>;
}
