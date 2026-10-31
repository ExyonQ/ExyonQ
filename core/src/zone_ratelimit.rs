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
//! 0.5 — nginx `limit_req_zone` per-route token buckets.
//!
//! Key sources: `ip`, `host`, `header:<name>`. Process-lifetime, bounded by
//! the existing zone map; `limit_req` semantics are reject-on-empty (nodelay).

use exyonq_config_ir::RateLimitConfig;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

struct Bucket {
    tokens: f64,
    last: Instant,
}

static ZONES: LazyLock<Mutex<HashMap<String, HashMap<String, Bucket>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// shortcut: fixed caps — a fully-refilled bucket equals an absent one, so
// eviction is always safe; LRU covers the pathological flood case.
const MAX_KEYS_PER_ZONE: usize = 10_000;
const MAX_KEY_CHARS: usize = 128;

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// Returns `Some(retry_after_secs)` when the request must be rejected.
pub(crate) fn check(
    config: &RateLimitConfig,
    route_name: &str,
    headers: &[(String, String)],
    host: Option<&str>,
    client_ip: &str,
) -> Option<u64> {
    let zone_name = config.routes.get(route_name)?;
    let zone = config.zones.get(zone_name)?;
    let key = match zone.key.as_str() {
        "ip" => client_ip.to_string(),
        "host" => host.unwrap_or("").to_string(),
        key if key.starts_with("header:") => header_value(headers, &key["header:".len()..])
            .unwrap_or("")
            .to_string(),
        _ => return None,
    };
    let key: String = key.chars().take(MAX_KEY_CHARS).collect();
    let now = Instant::now();
    let mut zones = ZONES.lock().unwrap_or_else(|e| e.into_inner());
    let map = zones.entry(zone_name.clone()).or_default();
    if map.len() >= MAX_KEYS_PER_ZONE && !map.contains_key(&key) {
        let full = zone.burst as f64;
        map.retain(|_, b| b.tokens < full);
        if map.len() >= MAX_KEYS_PER_ZONE {
            if let Some(oldest) = map
                .iter()
                .max_by_key(|(_, b)| b.last.elapsed())
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
    }
    let bucket = map.entry(key).or_insert(Bucket {
        tokens: zone.burst as f64,
        last: now,
    });
    let elapsed = now.duration_since(bucket.last).as_secs_f64();
    bucket.tokens =
        (bucket.tokens + elapsed * zone.requests_per_second as f64).min(zone.burst as f64);
    bucket.last = now;
    if bucket.tokens >= 1.0 {
        bucket.tokens -= 1.0;
        None
    } else {
        let retry = ((1.0 - bucket.tokens) / zone.requests_per_second.max(1) as f64).ceil() as u64;
        Some(retry.max(1))
    }
}
