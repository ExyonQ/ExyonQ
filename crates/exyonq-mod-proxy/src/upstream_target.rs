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

use crate::selector::endpoint_transport_identity;
use crate::{build_uri, preseed_path_uris, UpstreamDescriptor};
use http::Uri;
use hyper::header::HeaderValue;
use std::collections::HashMap;
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
    path_uris: Arc<HashMap<String, Uri>>,
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
            path_uris: Arc::new(preseed_path_uris(&base)),
        })
    }

    pub fn uri_for(&self, path_and_query: &str) -> Uri {
        if let Some(uri) = self.uri_for_preseeded(path_and_query) {
            return uri.clone();
        }
        build_uri(&self.base, path_and_query)
    }

    /// Borrow the configured upstream base URI (scheme/authority).
    pub fn base_uri(&self) -> &Uri {
        &self.base
    }

    /// Borrow a preseeded URI (P4 `/api/` hot path) — avoids Uri clone when present.
    pub fn uri_for_preseeded(&self, path_and_query: &str) -> Option<&Uri> {
        self.path_uris.get(path_and_query)
    }
}
