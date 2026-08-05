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
//! Token-bucket rate limiting per client IP (Fase 2).

use async_trait::async_trait;
use exyonq_addon_sdk::{
    official_core_compat, static_descriptor, Addon, AddonDescriptor, Capability, CostClass,
};
use exyonq_module_api::{Body, HttpRequest, HttpResponse, Module, ModuleInfo, CLIENT_IP_HEADER};
use http::{Response, StatusCode};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

const SHARD_COUNT: usize = 64;

static RATELIMIT_DESCRIPTOR: std::sync::LazyLock<AddonDescriptor> =
    std::sync::LazyLock::new(|| {
        static_descriptor(
            "ratelimit",
            env!("CARGO_PKG_VERSION"),
            official_core_compat(),
            &[Capability::RequestFilter],
            CostClass::LowCostHeaderOnly,
        )
    });

/// Static manifest for handshake registration (addon-api 1.x).
pub fn descriptor() -> &'static AddonDescriptor {
    &RATELIMIT_DESCRIPTOR
}

pub struct RateLimitModule {
    requests_per_second: f64,
    burst: f64,
    shards: Vec<Mutex<HashMap<String, BucketState>>>,
}

struct BucketState {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimitModule {
    pub fn new(requests_per_second: u32, burst: u32) -> Self {
        Self {
            requests_per_second: requests_per_second.max(1) as f64,
            burst: burst.max(1) as f64,
            shards: (0..SHARD_COUNT)
                .map(|_| Mutex::new(HashMap::new()))
                .collect(),
        }
    }

    fn client_key(req: &HttpRequest) -> &str {
        req.headers()
            .get(CLIENT_IP_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown")
    }

    fn shard_index(key: &str) -> usize {
        let mut hash = 0u64;
        for byte in key.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        (hash as usize) % SHARD_COUNT
    }

    /// Hot-path check used by the modules wire loop (P10 bench).
    pub fn allow_key(&self, key: &str) -> bool {
        let shard = Self::shard_index(key);
        let mut guard = self.shards[shard].lock().expect("ratelimit lock");
        let now = Instant::now();
        let state = match guard.get_mut(key) {
            Some(state) => state,
            None => guard.entry(key.to_owned()).or_insert(BucketState {
                tokens: self.burst,
                last_refill: now,
            }),
        };

        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        state.tokens = (state.tokens + elapsed * self.requests_per_second).min(self.burst);
        state.last_refill = now;

        if state.tokens >= 1.0 {
            state.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn allow(&self, key: &str) -> bool {
        self.allow_key(key)
    }
}

impl Addon for RateLimitModule {
    fn descriptor(&self) -> &'static AddonDescriptor {
        descriptor()
    }

    fn self_test(&self) -> Result<(), exyonq_addon_sdk::AddonError> {
        if self.requests_per_second <= 0.0 || self.burst <= 0.0 {
            return Err(exyonq_addon_sdk::AddonError::SelfTestFailed {
                addon: "ratelimit",
                reason: "requests_per_second and burst must be positive".into(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl Module for RateLimitModule {
    fn info(&self) -> ModuleInfo {
        ModuleInfo::from_descriptor(descriptor())
    }

    async fn on_route(
        &self,
        req: &HttpRequest,
    ) -> Result<Option<HttpResponse>, exyonq_module_api::BoxError> {
        let key = Self::client_key(req);
        if self.allow(key) {
            return Ok(None);
        }
        let response = Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("rate limit exceeded"))
            .expect("429 response");
        Ok(Some(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Request;

    #[tokio::test]
    async fn blocks_when_burst_exhausted() {
        let module = RateLimitModule::new(1, 1);
        let req = Request::builder()
            .header(CLIENT_IP_HEADER, "203.0.113.1")
            .uri("/")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        assert!(module.on_route(&req).await.unwrap().is_none());
        let blocked = module.on_route(&req).await.unwrap();
        assert_eq!(
            blocked.expect("429").status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[test]
    fn sharded_buckets_allow_in_parallel_keys() {
        let module = RateLimitModule::new(1, 1);
        assert!(module.allow_key("203.0.113.1"));
        assert!(!module.allow_key("203.0.113.1"));
        assert!(module.allow_key("203.0.113.2"));
    }
}
