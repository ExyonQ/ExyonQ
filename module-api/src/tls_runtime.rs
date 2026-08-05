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
//! TLS listener binding contract (KD4.5) — no rustls types.

/// Compiled TLS material paths for one listener (reload-time immutable strings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsListenerBinding {
    pub cert_path: String,
    pub key_path: String,
}

/// ALPN profile identifiers for TLS reload (contract-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsAlpnProfile {
    Http11AndH2,
    Http3,
}
