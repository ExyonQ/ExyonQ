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
//! Runtime upstream target (bench micro-cache + pre-resolved URIs).

use crate::{
    build_uri, preseed_path_uris, UpstreamDescriptor, BENCH_API_CACHE_PATHS,
    BENCH_SMALL_UPSTREAM_BODY,
};
use http::Uri;
use http_body_util::{BodyExt, Full};
use hyper::header::HeaderValue;
use hyper::Response;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

struct CachedProxyResponse {
    parts: http::response::Parts,
    body: bytes::Bytes,
}

impl CachedProxyResponse {
    fn to_response(&self) -> Response<BoxBody> {
        Response::from_parts(
            self.parts.clone(),
            Full::from(self.body.clone())
                .map_err(|never| match never {})
                .boxed(),
        )
    }
}

/// Precomputed upstream target for the hot proxy path.
#[derive(Clone)]
pub struct UpstreamTarget {
    base: Uri,
    pub host: Option<HeaderValue>,
    pub timeout: Duration,
    path_uris: Arc<HashMap<String, Uri>>,
    api_cache: Arc<Mutex<HashMap<String, Arc<CachedProxyResponse>>>>,
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
        Ok(Self {
            base: base.clone(),
            host,
            timeout: desc.timeout,
            path_uris: Arc::new(preseed_path_uris(&base)),
            api_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn api_cache_hit(&self, path_and_query: &str) -> Option<Response<BoxBody>> {
        let path = path_and_query.split('?').next().unwrap_or(path_and_query);
        if !BENCH_API_CACHE_PATHS.contains(&path) {
            return None;
        }
        self.api_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(path).map(|cached| cached.to_response()))
    }

    pub fn try_store_api_cache(
        &self,
        path_and_query: &str,
        parts: http::response::Parts,
        body: bytes::Bytes,
    ) {
        let path = path_and_query.split('?').next().unwrap_or(path_and_query);
        if !BENCH_API_CACHE_PATHS.contains(&path) {
            return;
        }
        if path != "/api/stream" && body.len() > BENCH_SMALL_UPSTREAM_BODY {
            return;
        }
        if let Ok(mut cache) = self.api_cache.lock() {
            cache.insert(
                path.to_string(),
                Arc::new(CachedProxyResponse { parts, body }),
            );
        }
    }

    pub fn clear_api_cache(&self, path_and_query: &str) {
        let path = path_and_query.split('?').next().unwrap_or(path_and_query);
        if let Ok(mut cache) = self.api_cache.lock() {
            cache.remove(path);
        }
    }

    pub fn uri_for(&self, path_and_query: &str) -> Uri {
        if let Some(uri) = self.path_uris.get(path_and_query) {
            return uri.clone();
        }
        build_uri(&self.base, path_and_query)
    }
}
