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
//! Runtime upstream target (pre-resolved URIs + timeout).
//!
//! No implicit response-body cache. Explicit Plan 12 `[[cache_policy]]` lives
//! outside this type (`serve_proxy_with_cache`).

use crate::selector::{endpoint_transport_identity, EndpointAdmin, PassivePeer};
use crate::{build_uri, UpstreamDescriptor};
use http::Uri;
use hyper::header::HeaderValue;
use std::sync::Arc;
use std::time::Duration;

/// Precomputed upstream target for the hot proxy path.
#[derive(Clone)]
pub struct UpstreamTarget {
    base: Uri,
    pub host: Option<HeaderValue>,
    pub timeout: Duration,
    /// Cap021 connect-only retry budget (bound from compiled slot / generation).
    pub max_connect_retries: u8,
    /// Stable peer key for request-local exclusion on retry (`scheme://host:port`).
    pub peer_key: String,
    /// NG-08: passive health peer state (ban/ramp). Transport paths record
    /// attempt outcomes here; for multi-endpoint clusters the same `Arc` is
    /// shared with the selector's `EndpointSpec`. `disabled()` for targets
    /// built without endpoint health config.
    pub passive: Arc<PassivePeer>,
    /// AP-01: hot admin state shared with the member table — keep-alive pins
    /// must drop a cached target when `member set` disables/drains it.
    pub admin: Arc<EndpointAdmin>,
}

impl UpstreamTarget {
    pub fn from_config(target: &str, timeout_ms: u64) -> anyhow::Result<Self> {
        let desc = UpstreamDescriptor::from_config(0, "upstream", target, timeout_ms)
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        Self::from_descriptor(&desc)
    }

    pub fn from_descriptor(desc: &UpstreamDescriptor) -> anyhow::Result<Self> {
        let base: Uri = desc
            .parsed_base_uri()
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        let host = desc
            .host
            .as_deref()
            .and_then(|value| HeaderValue::from_str(value).ok());
        let peer_key =
            endpoint_transport_identity(&desc.target).unwrap_or_else(|| desc.target.clone());
        Ok(Self {
            base: base.clone(),
            host,
            timeout: desc.timeout,
            max_connect_retries: desc.max_connect_retries,
            peer_key,
            passive: Arc::new(PassivePeer::disabled()),
            admin: Arc::new(EndpointAdmin::new(true, 1)),
        })
    }

    pub fn base_uri(&self) -> &Uri {
        &self.base
    }

    pub fn uri_for(&self, path_and_query: &str) -> Uri {
        build_uri(&self.base, path_and_query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_for_builds_any_path_without_preseed() {
        let target =
            UpstreamTarget::from_config("http://127.0.0.1:9000", 2_000).expect("upstream target");
        for path in ["/", "/v1/health", "/arbitrary/deep?q=1"] {
            assert_eq!(
                target
                    .uri_for(path)
                    .path_and_query()
                    .expect("path")
                    .as_str(),
                path
            );
        }
    }
}
