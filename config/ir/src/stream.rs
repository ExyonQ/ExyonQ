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
//! NG-20: `[[stream]]` — L4 TCP/UDP forwarding to a named upstream.

use serde::{Deserialize, Serialize};

/// Default idle timeout for an L4 connection/session.
pub const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = 60_000;
/// Lower bound for `stream.idle_timeout_ms`.
pub const MIN_STREAM_IDLE_TIMEOUT_MS: u64 = 1;
/// Upper bound for `stream.idle_timeout_ms` (24 hours).
pub const MAX_STREAM_IDLE_TIMEOUT_MS: u64 = 86_400_000;
/// Default per-listener UDP session cap.
pub const DEFAULT_STREAM_MAX_SESSIONS: usize = 4_096;
/// Upper bound for `stream.max_sessions`.
pub const MAX_STREAM_MAX_SESSIONS: usize = 65_536;
/// Upper bound for the number of `[[stream]]` listeners in one config.
pub const MAX_STREAMS: usize = 128;

/// L4 transport carried by one `[[stream]]` listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamProtocol {
    /// Byte stream forwarded with a bidirectional TCP relay.
    #[default]
    Tcp,
    /// Datagram stream with client-to-backend affinity per source address.
    Udp,
}

/// NG-20: one L4 listener forwarding to an existing `[[upstream]]`.
///
/// The upstream supplies the endpoint set and weights. Stream selection uses
/// the same smooth weighted round-robin as the HTTP proxy; `priority` bands
/// and `admin_state = "disabled"` are honored. A TCP connection or UDP source
/// address is pinned to one endpoint for its lifetime.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StreamConfig {
    /// Stable listener identity used in logs and validation.
    pub name: String,
    /// Local bind address, e.g. `0.0.0.0:3306` or `[::]:6379`.
    pub listen: String,
    /// Transport. Defaults to `tcp`.
    #[serde(default)]
    pub protocol: StreamProtocol,
    /// Name of an existing `[[upstream]]` table.
    pub upstream: String,
    /// Close an idle TCP relay or UDP session after this many milliseconds.
    #[serde(default = "default_stream_idle_timeout_ms")]
    pub idle_timeout_ms: u64,
    /// Maximum concurrent UDP sessions. Ignored for TCP listeners.
    #[serde(default = "default_stream_max_sessions")]
    pub max_sessions: usize,
}

fn default_stream_idle_timeout_ms() -> u64 {
    DEFAULT_STREAM_IDLE_TIMEOUT_MS
}

fn default_stream_max_sessions() -> usize {
    DEFAULT_STREAM_MAX_SESSIONS
}
