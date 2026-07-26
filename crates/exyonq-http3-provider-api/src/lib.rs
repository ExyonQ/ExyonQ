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
//! Provider-neutral HTTP/3 settings (P13D).
//!
//! No `s2n_quic` / `quiche` / `quinn` types cross this boundary.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Compile-/config-time provider selection (neutral string ids).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Http3ProviderId {
    /// Intended product default after P13D PASS.
    S2n,
    /// Optional advanced provider (`Config::set_max_ack_delay`).
    Quiche,
    /// Interim / rollback path (Quinn).
    QuinnLegacy,
}

impl Http3ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::S2n => "s2n",
            Self::Quiche => "quiche",
            Self::QuinnLegacy => "quinn-legacy",
        }
    }

    pub fn parse(s: &str) -> Result<Self, Http3ProviderError> {
        match s {
            "s2n" | "s2n-quic" => Ok(Self::S2n),
            "quiche" => Ok(Self::Quiche),
            "quinn" | "quinn-legacy" => Ok(Self::QuinnLegacy),
            other => Err(Http3ProviderError::InvalidProvider(other.to_string())),
        }
    }

    /// Public IR / product selection (`[http3].provider`). Quinn is not accepted.
    pub fn parse_product(s: &str) -> Result<Self, Http3ProviderError> {
        match s {
            "s2n" | "s2n-quic" => Ok(Self::S2n),
            "quiche" => Ok(Self::Quiche),
            other => Err(Http3ProviderError::InvalidProvider(other.to_string())),
        }
    }
}

/// Neutral provider listen/TLS/ACK settings.
#[derive(Debug, Clone)]
pub struct Http3ProviderConfig {
    pub provider: Http3ProviderId,
    pub listen: SocketAddr,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    /// Local max_ack_delay transport parameter (milliseconds). Providers that
    /// cannot apply it must document NO-OP.
    pub max_ack_delay_ms: u64,
    pub qlog_enabled: bool,
    pub request_body_drain_cap_bytes: usize,
    pub idle_timeout_ms: u64,
    pub max_concurrent_streams: u64,
    pub initial_stream_window: u64,
    pub initial_connection_window: u64,
}

impl Http3ProviderConfig {
    pub const DEFAULT_MAX_ACK_DELAY_MS: u64 = 1;
    pub const DEFAULT_DRAIN_CAP: usize = 64 * 1024;
    pub const ALPN_H3: &'static [u8] = b"h3";

    pub fn validate(&self) -> Result<(), Http3ProviderError> {
        // RFC 9000 max_ack_delay is encoded in ms; reject absurd values.
        if self.max_ack_delay_ms > 25_000 {
            return Err(Http3ProviderError::InvalidMaxAckDelay(self.max_ack_delay_ms));
        }
        if self.request_body_drain_cap_bytes == 0 {
            return Err(Http3ProviderError::InvalidDrainCap);
        }
        if self.idle_timeout_ms == 0 {
            return Err(Http3ProviderError::InvalidIdleTimeout);
        }
        Ok(())
    }
}

/// Provider-edge errors (no upstream provider types).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Http3ProviderError {
    InvalidProvider(String),
    InvalidMaxAckDelay(u64),
    InvalidDrainCap,
    InvalidIdleTimeout,
    FeatureNotBuilt(&'static str),
    Internal(String),
}

impl std::fmt::Display for Http3ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidProvider(s) => write!(f, "invalid http3 provider: {s}"),
            Self::InvalidMaxAckDelay(v) => write!(f, "invalid max_ack_delay_ms: {v}"),
            Self::InvalidDrainCap => write!(f, "request_body_drain_cap_bytes must be > 0"),
            Self::InvalidIdleTimeout => write!(f, "idle_timeout_ms must be > 0"),
            Self::FeatureNotBuilt(p) => write!(f, "http3 provider {p} not in this build"),
            Self::Internal(s) => write!(f, "http3 provider internal: {s}"),
        }
    }
}

impl std::error::Error for Http3ProviderError {}

/// Backward-compatible alias used by early P13P draft.
#[derive(Debug, Clone)]
pub struct Http3ProviderListen {
    pub listen: SocketAddr,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub max_ack_delay_ms: u64,
}

impl From<Http3ProviderListen> for Http3ProviderConfig {
    fn from(v: Http3ProviderListen) -> Self {
        Self {
            provider: Http3ProviderId::Quiche,
            listen: v.listen,
            cert_path: v.cert_path,
            key_path: v.key_path,
            max_ack_delay_ms: v.max_ack_delay_ms,
            qlog_enabled: false,
            request_body_drain_cap_bytes: Self::DEFAULT_DRAIN_CAP,
            idle_timeout_ms: 30_000,
            max_concurrent_streams: 256,
            initial_stream_window: 1_000_000,
            initial_connection_window: 10_000_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_provider_ids() {
        assert_eq!(Http3ProviderId::parse("s2n").unwrap(), Http3ProviderId::S2n);
        assert_eq!(Http3ProviderId::parse("quiche").unwrap(), Http3ProviderId::Quiche);
        assert!(Http3ProviderId::parse("nginx").is_err());
        assert_eq!(
            Http3ProviderId::parse_product("s2n").unwrap(),
            Http3ProviderId::S2n
        );
        assert!(Http3ProviderId::parse_product("quinn-legacy").is_err());
    }

    #[test]
    fn rejects_absurd_ack_delay() {
        let mut cfg = Http3ProviderConfig {
            provider: Http3ProviderId::S2n,
            listen: "127.0.0.1:0".parse().unwrap(),
            cert_path: PathBuf::from("c"),
            key_path: PathBuf::from("k"),
            max_ack_delay_ms: 30_000,
            qlog_enabled: false,
            request_body_drain_cap_bytes: 1024,
            idle_timeout_ms: 1000,
            max_concurrent_streams: 1,
            initial_stream_window: 1,
            initial_connection_window: 1,
        };
        assert!(cfg.validate().is_err());
        cfg.max_ack_delay_ms = 1;
        assert!(cfg.validate().is_ok());
    }
}
