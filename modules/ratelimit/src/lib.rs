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
//! Token-bucket rate limiting per client IP (Fase 2 / Cap055).
//!
//! Client identity = peer IP via `x-exyonq-client-ip` (never request XFF/Forwarded).
//! Module limiter state is process-lifetime (survives config reload) with bounded
//! per-shard maps and idle eviction.

use async_trait::async_trait;
use exyonq_addon_sdk::{
    official_core_compat, static_descriptor, Addon, AddonDescriptor, Capability, CostClass,
};
use exyonq_module_api::{Body, HttpRequest, HttpResponse, Module, ModuleInfo, CLIENT_IP_HEADER};
use http::{Response, StatusCode};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const SHARD_COUNT: usize = 64;
/// Cap055: bound unique IP growth (64 × this).
const MAX_KEYS_PER_SHARD: usize = 2048;
/// Idle entries at full capacity may be evicted after this.
const IDLE_EVICT_AFTER: Duration = Duration::from_secs(300);

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

struct BucketState {
    tokens: f64,
    last_refill: Instant,
}

/// Sharded synchronous token bucket (shared by the ratelimit module and WAF abuse seam).
pub struct SyncTokenBucket {
    requests_per_second: f64,
    burst: f64,
    shards: Vec<Mutex<HashMap<String, BucketState>>>,
}

impl SyncTokenBucket {
    pub fn new(requests_per_second: u32, burst: u32) -> Self {
        Self {
            requests_per_second: requests_per_second.max(1) as f64,
            burst: burst.max(1) as f64,
            shards: (0..SHARD_COUNT)
                .map(|_| Mutex::new(HashMap::new()))
                .collect(),
        }
    }

    fn shard_index(key: &str) -> usize {
        let mut hash = 0u64;
        for byte in key.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        (hash as usize) % SHARD_COUNT
    }

    #[cfg(test)]
    fn test_shard_index(key: &str) -> usize {
        Self::shard_index(key)
    }

    fn projected_tokens(state: &BucketState, now: Instant, rps: f64, burst: f64) -> f64 {
        let elapsed = now
            .checked_duration_since(state.last_refill)
            .unwrap_or(Duration::ZERO)
            .as_secs_f64();
        (state.tokens + elapsed * rps).min(burst)
    }

    fn evict_idle_full(
        guard: &mut HashMap<String, BucketState>,
        now: Instant,
        rps: f64,
        burst: f64,
    ) -> bool {
        // Only idle full buckets — never evict throttled entries (LA-CAP055-002).
        let mut idle_full: Option<(String, Instant)> = None;
        for (k, st) in guard.iter() {
            let tokens = Self::projected_tokens(st, now, rps, burst);
            let age = now
                .checked_duration_since(st.last_refill)
                .unwrap_or(Duration::ZERO);
            if tokens >= burst - f64::EPSILON && age >= IDLE_EVICT_AFTER {
                match &idle_full {
                    Some((_, t)) if st.last_refill >= *t => {}
                    _ => idle_full = Some((k.clone(), st.last_refill)),
                }
            }
        }
        if let Some((k, _)) = idle_full {
            guard.remove(&k);
            true
        } else {
            false
        }
    }

    /// Hot-path check used by the modules wire loop and WAF abuse gate.
    pub fn allow_key(&self, key: &str) -> bool {
        self.allow_key_with(key, self.requests_per_second, self.burst)
    }

    fn allow_key_with(&self, key: &str, rps: f64, burst: f64) -> bool {
        let shard = Self::shard_index(key);
        let mut guard = match self.shards[shard].lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let now = Instant::now();
        if !guard.contains_key(key) && guard.len() >= MAX_KEYS_PER_SHARD {
            // Cap growth without resetting active/throttled clients (LA-CAP055-002).
            if !Self::evict_idle_full(&mut guard, now, rps, burst) {
                return false;
            }
        }
        let state = match guard.get_mut(key) {
            Some(state) => state,
            None => guard.entry(key.to_owned()).or_insert(BucketState {
                tokens: burst,
                last_refill: now,
            }),
        };

        state.tokens = Self::projected_tokens(state, now, rps, burst);
        state.last_refill = now;

        if state.tokens >= 1.0 {
            state.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Seconds until one token is available (ceil), after simulating refill at `now`.
    pub fn retry_after_secs(&self, key: &str) -> u64 {
        self.retry_after_secs_with(key, self.requests_per_second, self.burst)
    }

    fn retry_after_secs_with(&self, key: &str, rps: f64, burst: f64) -> u64 {
        let shard = Self::shard_index(key);
        let guard = match self.shards[shard].lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let now = Instant::now();
        let Some(state) = guard.get(key) else {
            return 0;
        };
        let tokens = Self::projected_tokens(state, now, rps, burst);
        if tokens >= 1.0 {
            return 0;
        }
        let deficit = 1.0 - tokens;
        ((deficit / rps).ceil() as u64).max(1)
    }

    #[cfg(test)]
    fn shard_len(&self, key: &str) -> usize {
        let shard = Self::shard_index(key);
        self.shards[shard].lock().map(|g| g.len()).unwrap_or(0)
    }
}

/// Process-lifetime Cap055 module limiter (params update on reload; map persists).
struct ProcessModuleLimiter {
    rps: AtomicU32,
    burst: AtomicU32,
    bucket: SyncTokenBucket,
}

impl ProcessModuleLimiter {
    fn new() -> Self {
        Self {
            rps: AtomicU32::new(100),
            burst: AtomicU32::new(200),
            // Placeholder rates; allow_key uses Atomic params.
            bucket: SyncTokenBucket::new(100, 200),
        }
    }

    fn set_params(&self, requests_per_second: u32, burst: u32) {
        self.rps
            .store(requests_per_second.max(1), Ordering::Relaxed);
        self.burst.store(burst.max(1), Ordering::Relaxed);
    }

    fn allow_key(&self, key: &str) -> bool {
        let rps = self.rps.load(Ordering::Relaxed).max(1) as f64;
        let burst = self.burst.load(Ordering::Relaxed).max(1) as f64;
        self.bucket.allow_key_with(key, rps, burst)
    }

    fn retry_after_secs(&self, key: &str) -> u64 {
        let rps = self.rps.load(Ordering::Relaxed).max(1) as f64;
        let burst = self.burst.load(Ordering::Relaxed).max(1) as f64;
        self.bucket.retry_after_secs_with(key, rps, burst)
    }
}

static PROCESS_MODULE_LIMITER: LazyLock<ProcessModuleLimiter> =
    LazyLock::new(ProcessModuleLimiter::new);
static PROCESS_WIRE_RATELIMIT_ENABLED: AtomicBool = AtomicBool::new(false);

/// Limiter backend: process-global Cap055 state, or an isolated bucket (tests).
///
/// `local == None` with isolated mode is unrepresentable — eliminates the former
/// `expect("isolated")` panic path on the request hot path.
enum RateLimitBackend {
    ProcessGlobal,
    #[cfg(test)]
    Isolated(SyncTokenBucket),
}

pub struct RateLimitModule {
    backend: RateLimitBackend,
}

impl Default for RateLimitModule {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimitModule {
    pub fn publish_wire_config(enabled: bool, requests_per_second: u32, burst: u32) {
        if enabled {
            PROCESS_MODULE_LIMITER.set_params(requests_per_second, burst);
        }
        PROCESS_WIRE_RATELIMIT_ENABLED.store(enabled, Ordering::Release);
    }

    pub fn new() -> Self {
        Self {
            backend: RateLimitBackend::ProcessGlobal,
        }
    }

    /// Isolated limiter for unit tests (does not touch process-global state).
    #[cfg(test)]
    fn new_isolated(requests_per_second: u32, burst: u32) -> Self {
        Self {
            backend: RateLimitBackend::Isolated(SyncTokenBucket::new(requests_per_second, burst)),
        }
    }

    fn client_key(req: &HttpRequest) -> String {
        let raw = req
            .headers()
            .get(CLIENT_IP_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown");
        Self::canonical_client_ip(raw)
    }

    /// Canonical IpAddr Display — mapped IPv4 folds to IPv4 (LA-CAP055-001).
    fn canonical_client_ip(raw: &str) -> String {
        if let Ok(ip) = raw.parse::<IpAddr>() {
            ip.to_canonical().to_string()
        } else {
            raw.to_string()
        }
    }

    /// Hot-path check used by the modules wire loop (P10 bench).
    pub fn allow_key(&self, key: &str) -> bool {
        match &self.backend {
            RateLimitBackend::ProcessGlobal => PROCESS_MODULE_LIMITER.allow_key(key),
            #[cfg(test)]
            RateLimitBackend::Isolated(bucket) => bucket.allow_key(key),
        }
    }

    /// True while Cap067 wire admit can reject. Static workers skip IP formatting when false.
    pub fn wire_admit_active() -> bool {
        PROCESS_WIRE_RATELIMIT_ENABLED.load(Ordering::Acquire)
    }

    /// Cap067 / proxy-wire admit using process-global Cap055 limiter (peer IP key).
    pub fn wire_admit_client_ip(client_ip: &str) -> exyonq_module_api::WireAdmit {
        if !Self::wire_admit_active() {
            return exyonq_module_api::WireAdmit::Allow;
        }
        let key = Self::canonical_client_ip(client_ip);
        if PROCESS_MODULE_LIMITER.allow_key(&key) {
            exyonq_module_api::WireAdmit::Allow
        } else {
            exyonq_module_api::WireAdmit::Reject429 {
                retry_after_secs: PROCESS_MODULE_LIMITER.retry_after_secs(&key).max(1),
            }
        }
    }

    fn allow(&self, key: &str) -> bool {
        self.allow_key(key)
    }

    fn retry_after_secs(&self, key: &str) -> u64 {
        match &self.backend {
            RateLimitBackend::ProcessGlobal => PROCESS_MODULE_LIMITER.retry_after_secs(key),
            #[cfg(test)]
            RateLimitBackend::Isolated(bucket) => bucket.retry_after_secs(key),
        }
    }
}

impl Addon for RateLimitModule {
    fn descriptor(&self) -> &'static AddonDescriptor {
        descriptor()
    }

    fn self_test(&self) -> Result<(), exyonq_addon_sdk::AddonError> {
        // Do not consume process-limiter tokens (LA-CAP055-005).
        let _ = Self::client_key(
            &http::Request::builder()
                .header(CLIENT_IP_HEADER, "127.0.0.1")
                .uri("/")
                .body(Body::from(bytes::Bytes::new()))
                .expect("self_test request"),
        );
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
        if self.allow(&key) {
            return Ok(None);
        }
        let retry_after = self.retry_after_secs(&key).max(1);
        let response = Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header(http::header::RETRY_AFTER, retry_after.to_string())
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

    #[test]
    fn wire_admit_active_follows_published_flag() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let previous = PROCESS_WIRE_RATELIMIT_ENABLED.load(std::sync::atomic::Ordering::Acquire);
        RateLimitModule::publish_wire_config(false, 1, 1);
        assert!(!RateLimitModule::wire_admit_active());
        assert!(matches!(
            RateLimitModule::wire_admit_client_ip("203.0.113.9"),
            exyonq_module_api::WireAdmit::Allow
        ));
        RateLimitModule::publish_wire_config(previous, 1, 1);
    }

    #[tokio::test]
    async fn blocks_when_burst_exhausted() {
        let module = RateLimitModule::new_isolated(1, 1);
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
        let module = RateLimitModule::new_isolated(1, 1);
        assert!(module.allow_key("203.0.113.1"));
        assert!(!module.allow_key("203.0.113.1"));
        assert!(module.allow_key("203.0.113.2"));
    }

    #[test]
    fn retry_after_secs_ceil_when_denied() {
        let bucket = SyncTokenBucket::new(2, 2);
        assert!(bucket.allow_key("203.0.113.1"));
        assert!(bucket.allow_key("203.0.113.1"));
        assert!(!bucket.allow_key("203.0.113.1"));
        assert_eq!(bucket.retry_after_secs("203.0.113.1"), 1);
    }

    #[test]
    fn ipv6_canonical_key() {
        let module = RateLimitModule::new_isolated(1, 1);
        let req_a = Request::builder()
            .header(CLIENT_IP_HEADER, "2001:db8::1")
            .uri("/")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let req_b = Request::builder()
            .header(CLIENT_IP_HEADER, "2001:0db8:0000:0000:0000:0000:0000:0001")
            .uri("/")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        assert_eq!(
            RateLimitModule::client_key(&req_a),
            RateLimitModule::client_key(&req_b)
        );
        assert!(module.allow(&RateLimitModule::client_key(&req_a)));
        assert!(!module.allow(&RateLimitModule::client_key(&req_b)));
    }

    #[test]
    fn ipv4_mapped_folds_to_ipv4_key() {
        let module = RateLimitModule::new_isolated(1, 1);
        let req_v4 = Request::builder()
            .header(CLIENT_IP_HEADER, "203.0.113.9")
            .uri("/")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let req_mapped = Request::builder()
            .header(CLIENT_IP_HEADER, "::ffff:203.0.113.9")
            .uri("/")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        assert_eq!(
            RateLimitModule::client_key(&req_v4),
            RateLimitModule::client_key(&req_mapped)
        );
        assert_eq!(RateLimitModule::client_key(&req_v4), "203.0.113.9");
        assert!(module.allow(&RateLimitModule::client_key(&req_v4)));
        assert!(!module.allow(&RateLimitModule::client_key(&req_mapped)));
    }

    #[test]
    fn eviction_caps_shard_growth() {
        let bucket = SyncTokenBucket::new(100, 100);
        let target = 0usize;
        let mut n = 0u64;
        let mut inserted = 0;
        while inserted < MAX_KEYS_PER_SHARD + 64 {
            let key = format!("evict-key-{n}");
            n += 1;
            if SyncTokenBucket::test_shard_index(&key) != target {
                continue;
            }
            let _ = bucket.allow_key(&key);
            inserted += 1;
        }
        let probe = (0..n)
            .map(|i| format!("evict-key-{i}"))
            .find(|k| SyncTokenBucket::test_shard_index(k) == target)
            .unwrap();
        let len = bucket.shard_len(&probe);
        assert!(len <= MAX_KEYS_PER_SHARD, "shard grew unbounded: {len}");
    }

    #[test]
    fn shard_full_does_not_reset_throttled_via_oldest_evict() {
        // LA-CAP055-002: when shard is full of non-idle entries, new keys are denied;
        // existing throttled keys must not regain a full burst via eviction.
        let bucket = SyncTokenBucket::new(1, 1);
        let target = 0usize;
        let mut n = 0u64;
        let mut keys = Vec::new();
        while keys.len() < MAX_KEYS_PER_SHARD {
            let key = format!("full-key-{n}");
            n += 1;
            if SyncTokenBucket::test_shard_index(&key) != target {
                continue;
            }
            assert!(bucket.allow_key(&key), "seed allow {key}");
            // Exhaust to tokens=0 (throttled, not idle-full).
            assert!(!bucket.allow_key(&key));
            keys.push(key);
        }
        let victim = &keys[0];
        // Seed of MAX_KEYS_PER_SHARD entries can exceed 1s wall time; rps=1 may refill.
        // Consume any refilled token so the pressure asserts remain meaningful.
        let _ = bucket.allow_key(victim);
        assert!(!bucket.allow_key(victim), "still throttled before pressure");
        // New identity into same shard must be refused (no idle-full to evict).
        let mut extra = None;
        while extra.is_none() {
            let key = format!("full-key-{n}");
            n += 1;
            if SyncTokenBucket::test_shard_index(&key) == target {
                extra = Some(key);
            }
        }
        let extra = extra.unwrap();
        assert!(
            !bucket.allow_key(&extra),
            "new key must be denied at capacity"
        );
        // Victim still throttled — not wiped to a fresh burst.
        let _ = bucket.allow_key(victim);
        assert!(
            !bucket.allow_key(victim),
            "throttled victim must not regain burst via eviction"
        );
        assert_eq!(bucket.shard_len(victim), MAX_KEYS_PER_SHARD);
    }
}
