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
//! Compiled upstream metadata (no connection pool, no response-body cache).

use crate::errors::ProxyConfigError;
use exyonq_module_api::proxy_dispatch::ProxyCompiledSlot;
use http::Uri;
use std::time::Duration;

/// Declared Content-Length threshold (bytes) for optional single-response
/// materialization vs streaming. This is **not** a cross-request response cache.
pub const BENCH_SMALL_UPSTREAM_BODY: usize = 16 * 1024;

/// Immutable upstream descriptor parsed from config (maps to [`ProxyCompiledSlot`] at compile time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamDescriptor {
    pub cluster_id: u32,
    pub upstream_name: String,
    pub target: String,
    pub timeout: Duration,
    pub host: Option<String>,
}

impl UpstreamDescriptor {
    pub fn from_config(
        cluster_id: u32,
        upstream_name: impl Into<String>,
        target: &str,
        timeout_ms: u64,
    ) -> Result<Self, ProxyConfigError> {
        let target: Uri = target.parse().map_err(|err: http::uri::InvalidUri| {
            ProxyConfigError::InvalidTarget(err.to_string())
        })?;
        let host = target.host().map(str::to_string);
        Ok(Self {
            cluster_id,
            upstream_name: upstream_name.into(),
            target: target.to_string(),
            timeout: Duration::from_millis(timeout_ms),
            host,
        })
    }

    pub fn to_compiled_slot(&self) -> ProxyCompiledSlot {
        ProxyCompiledSlot::legacy_single(
            self.cluster_id,
            self.upstream_name.clone(),
            self.target.clone(),
            self.timeout,
        )
    }

    pub fn parsed_base_uri(&self) -> Result<Uri, ProxyConfigError> {
        self.target
            .parse()
            .map_err(|err: http::uri::InvalidUri| ProxyConfigError::InvalidTarget(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_extracts_host() {
        let desc = UpstreamDescriptor::from_config(0, "backend", "http://127.0.0.1:8080", 30_000)
            .expect("valid");
        assert_eq!(desc.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(desc.timeout, Duration::from_millis(30_000));
    }

    #[test]
    fn compiled_slot_roundtrip() {
        let desc = UpstreamDescriptor::from_config(2, "api", "http://upstream", 1_000).expect("ok");
        let slot = desc.to_compiled_slot();
        assert_eq!(slot.cluster_id, 2);
        assert_eq!(slot.upstream_name, "api");
    }
}
