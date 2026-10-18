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
//! Temporary observational counters for raw-upstream connect / reuse / wait (Phase 8).
//!
//! **Default OFF.** Enable with `EXYONQ_PROXY_UPSTREAM_OBS=1`.
//! Optional JSON dump path: `EXYONQ_PROXY_UPSTREAM_OBS_FILE` (default
//! `/tmp/exyonq-upstream-obs.json`), rewritten ≈1 Hz on a dedicated std thread.
//!
//! Hot path when disabled: one relaxed atomic load (same class as
//! [`crate::raw_upstream::raw_upstream_get_enabled`]). No logs, no locks, no
//! channels, no heap on the request path.
//!
//! Removal plan: delete this module + call sites in `raw_upstream.rs` once the
//! p99/connect hypothesis is closed or rejected; do not leave permanently on.

use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static ENABLED_CACHE: AtomicI8 = AtomicI8::new(-1);
static DUMP_STARTED: AtomicBool = AtomicBool::new(false);

static REUSE_HIT: AtomicU64 = AtomicU64::new(0);
static REUSE_MISS: AtomicU64 = AtomicU64::new(0);
static CONNECT_OK: AtomicU64 = AtomicU64::new(0);
static CONNECT_FAIL: AtomicU64 = AtomicU64::new(0);
static CONNECT_TIMEOUT: AtomicU64 = AtomicU64::new(0);
static IDLE_DROP_DIRTY: AtomicU64 = AtomicU64::new(0);
static CHECKIN_OK: AtomicU64 = AtomicU64::new(0);
static CHECKIN_DROP_FULL: AtomicU64 = AtomicU64::new(0);
static NOT_POOLABLE_DROP: AtomicU64 = AtomicU64::new(0);

static CHECKOUT_WAIT_NS_SUM: AtomicU64 = AtomicU64::new(0);
static CHECKOUT_WAIT_NS_MAX: AtomicU64 = AtomicU64::new(0);
static CHECKOUT_WAIT_SAMPLES: AtomicU64 = AtomicU64::new(0);
static CHECKOUT_WAIT_ATTEMPTS: AtomicU64 = AtomicU64::new(0);

static CONNECT_NS_SUM: AtomicU64 = AtomicU64::new(0);
static CONNECT_NS_MAX: AtomicU64 = AtomicU64::new(0);
static CONNECT_NS_SAMPLES: AtomicU64 = AtomicU64::new(0);
static CONNECT_NS_ATTEMPTS: AtomicU64 = AtomicU64::new(0);

static TTFW_NS_SUM: AtomicU64 = AtomicU64::new(0);
static TTFW_NS_MAX: AtomicU64 = AtomicU64::new(0);
static TTFW_NS_SAMPLES: AtomicU64 = AtomicU64::new(0);
static TTFW_NS_ATTEMPTS: AtomicU64 = AtomicU64::new(0);

/// Last observed idle sockets in pool (sampled under existing pool lock).
static POOL_IDLE: AtomicU64 = AtomicU64::new(0);
static POOL_IDLE_MAX: AtomicU64 = AtomicU64::new(0);

/// Sampling: record timing every Nth event when enabled (1 = all).
const SAMPLE_EVERY: u64 = 1;

/// Gate: env `EXYONQ_PROXY_UPSTREAM_OBS` ∈ {1,true,on}. Cached after first read.
#[inline]
pub fn enabled() -> bool {
    let cached = ENABLED_CACHE.load(Ordering::Relaxed);
    if cached >= 0 {
        return cached != 0;
    }
    let on = matches!(
        std::env::var("EXYONQ_PROXY_UPSTREAM_OBS").ok().as_deref(),
        Some("1") | Some("true") | Some("on")
    );
    ENABLED_CACHE.store(if on { 1 } else { 0 }, Ordering::Relaxed);
    if on {
        ensure_dump_thread();
    }
    on
}

fn ensure_dump_thread() {
    if DUMP_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let path = std::env::var("EXYONQ_PROXY_UPSTREAM_OBS_FILE")
        .unwrap_or_else(|_| "/tmp/exyonq-upstream-obs.json".to_string());
    std::thread::Builder::new()
        .name("exyonq-up-obs".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            if !enabled() {
                continue;
            }
            let body = snapshot_json();
            let _ = std::fs::write(&path, body);
        })
        .ok();
}

#[inline]
fn record_ns(sum: &AtomicU64, max: &AtomicU64, samples: &AtomicU64, attempts: &AtomicU64, ns: u64) {
    // Gate on attempts; only accepted samples increment `samples` so
    // mean = sum/samples stays valid when SAMPLE_EVERY > 1.
    let attempt = attempts.fetch_add(1, Ordering::Relaxed) + 1;
    if SAMPLE_EVERY > 1 && !attempt.is_multiple_of(SAMPLE_EVERY) {
        return;
    }
    samples.fetch_add(1, Ordering::Relaxed);
    sum.fetch_add(ns, Ordering::Relaxed);
    max.fetch_max(ns, Ordering::Relaxed);
}

#[inline]
pub fn note_reuse_hit(checkout_wait: Duration) {
    if !enabled() {
        return;
    }
    REUSE_HIT.fetch_add(1, Ordering::Relaxed);
    record_ns(
        &CHECKOUT_WAIT_NS_SUM,
        &CHECKOUT_WAIT_NS_MAX,
        &CHECKOUT_WAIT_SAMPLES,
        &CHECKOUT_WAIT_ATTEMPTS,
        checkout_wait.as_nanos() as u64,
    );
}

#[inline]
pub fn note_idle_drop_dirty() {
    if !enabled() {
        return;
    }
    IDLE_DROP_DIRTY.fetch_add(1, Ordering::Relaxed);
}

pub enum ConnectOutcome {
    Ok,
    Fail,
    Timeout,
}

#[inline]
pub fn note_reuse_miss(checkout_wait: Duration, connect: Duration, outcome: ConnectOutcome) {
    if !enabled() {
        return;
    }
    REUSE_MISS.fetch_add(1, Ordering::Relaxed);
    record_ns(
        &CHECKOUT_WAIT_NS_SUM,
        &CHECKOUT_WAIT_NS_MAX,
        &CHECKOUT_WAIT_SAMPLES,
        &CHECKOUT_WAIT_ATTEMPTS,
        checkout_wait.as_nanos() as u64,
    );
    record_ns(
        &CONNECT_NS_SUM,
        &CONNECT_NS_MAX,
        &CONNECT_NS_SAMPLES,
        &CONNECT_NS_ATTEMPTS,
        connect.as_nanos() as u64,
    );
    match outcome {
        ConnectOutcome::Ok => {
            CONNECT_OK.fetch_add(1, Ordering::Relaxed);
        }
        ConnectOutcome::Fail => {
            CONNECT_FAIL.fetch_add(1, Ordering::Relaxed);
        }
        ConnectOutcome::Timeout => {
            CONNECT_TIMEOUT.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[inline]
pub fn note_checkin(ok: bool) {
    if !enabled() {
        return;
    }
    if ok {
        CHECKIN_OK.fetch_add(1, Ordering::Relaxed);
    } else {
        CHECKIN_DROP_FULL.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn note_not_poolable_drop() {
    if !enabled() {
        return;
    }
    NOT_POOLABLE_DROP.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_ttfw(since_checkout: Duration) {
    if !enabled() {
        return;
    }
    record_ns(
        &TTFW_NS_SUM,
        &TTFW_NS_MAX,
        &TTFW_NS_SAMPLES,
        &TTFW_NS_ATTEMPTS,
        since_checkout.as_nanos() as u64,
    );
}

#[inline]
pub fn note_pool_idle(idle: usize) {
    if !enabled() {
        return;
    }
    let v = idle as u64;
    POOL_IDLE.store(v, Ordering::Relaxed);
    POOL_IDLE_MAX.fetch_max(v, Ordering::Relaxed);
}

/// Mark start of an observed checkout (no-op / Instant::now only when enabled).
#[inline]
pub fn mark_start() -> Option<Instant> {
    if enabled() {
        Some(Instant::now())
    } else {
        None
    }
}

pub fn reset() {
    for a in [
        &REUSE_HIT,
        &REUSE_MISS,
        &CONNECT_OK,
        &CONNECT_FAIL,
        &CONNECT_TIMEOUT,
        &IDLE_DROP_DIRTY,
        &CHECKIN_OK,
        &CHECKIN_DROP_FULL,
        &NOT_POOLABLE_DROP,
        &CHECKOUT_WAIT_NS_SUM,
        &CHECKOUT_WAIT_NS_MAX,
        &CHECKOUT_WAIT_SAMPLES,
        &CHECKOUT_WAIT_ATTEMPTS,
        &CONNECT_NS_SUM,
        &CONNECT_NS_MAX,
        &CONNECT_NS_SAMPLES,
        &CONNECT_NS_ATTEMPTS,
        &TTFW_NS_SUM,
        &TTFW_NS_MAX,
        &TTFW_NS_SAMPLES,
        &TTFW_NS_ATTEMPTS,
        &POOL_IDLE,
        &POOL_IDLE_MAX,
    ] {
        a.store(0, Ordering::Relaxed);
    }
}

fn mean_ns(sum: u64, n: u64) -> f64 {
    if n == 0 {
        0.0
    } else {
        (sum as f64) / (n as f64)
    }
}

pub fn snapshot_json() -> String {
    let wait_n = CHECKOUT_WAIT_SAMPLES.load(Ordering::Relaxed);
    let conn_n = CONNECT_NS_SAMPLES.load(Ordering::Relaxed);
    let ttfw_n = TTFW_NS_SAMPLES.load(Ordering::Relaxed);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // Manual JSON — avoid pulling serde into hot obs path / extra deps.
    format!(
        concat!(
            "{{",
            "\"enabled\":{enabled},",
            "\"now_unix_ms\":{now},",
            "\"upstream_reuse_hit\":{hit},",
            "\"upstream_reuse_miss\":{miss},",
            "\"connect_ok\":{cok},",
            "\"connect_fail\":{cfail},",
            "\"connect_timeout\":{cto},",
            "\"idle_drop_dirty\":{dirty},",
            "\"checkin_ok\":{cin},",
            "\"checkin_drop_full\":{cfull},",
            "\"not_poolable_drop\":{npool},",
            "\"pool_idle\":{pidle},",
            "\"pool_idle_max\":{pidle_max},",
            "\"checkout_wait_ns_sum\":{wsum},",
            "\"checkout_wait_ns_max\":{wmax},",
            "\"checkout_wait_ns_mean\":{wmean:.3},",
            "\"checkout_wait_samples\":{wn},",
            "\"connect_ns_sum\":{csum},",
            "\"connect_ns_max\":{cmax},",
            "\"connect_ns_mean\":{cmean:.3},",
            "\"connect_ns_samples\":{cn},",
            "\"ttfw_ns_sum\":{tsum},",
            "\"ttfw_ns_max\":{tmax},",
            "\"ttfw_ns_mean\":{tmean:.3},",
            "\"ttfw_samples\":{tn}",
            "}}\n"
        ),
        enabled = enabled(),
        now = now_ms,
        hit = REUSE_HIT.load(Ordering::Relaxed),
        miss = REUSE_MISS.load(Ordering::Relaxed),
        cok = CONNECT_OK.load(Ordering::Relaxed),
        cfail = CONNECT_FAIL.load(Ordering::Relaxed),
        cto = CONNECT_TIMEOUT.load(Ordering::Relaxed),
        dirty = IDLE_DROP_DIRTY.load(Ordering::Relaxed),
        cin = CHECKIN_OK.load(Ordering::Relaxed),
        cfull = CHECKIN_DROP_FULL.load(Ordering::Relaxed),
        npool = NOT_POOLABLE_DROP.load(Ordering::Relaxed),
        pidle = POOL_IDLE.load(Ordering::Relaxed),
        pidle_max = POOL_IDLE_MAX.load(Ordering::Relaxed),
        wsum = CHECKOUT_WAIT_NS_SUM.load(Ordering::Relaxed),
        wmax = CHECKOUT_WAIT_NS_MAX.load(Ordering::Relaxed),
        wmean = mean_ns(CHECKOUT_WAIT_NS_SUM.load(Ordering::Relaxed), wait_n),
        wn = wait_n,
        csum = CONNECT_NS_SUM.load(Ordering::Relaxed),
        cmax = CONNECT_NS_MAX.load(Ordering::Relaxed),
        cmean = mean_ns(CONNECT_NS_SUM.load(Ordering::Relaxed), conn_n),
        cn = conn_n,
        tsum = TTFW_NS_SUM.load(Ordering::Relaxed),
        tmax = TTFW_NS_MAX.load(Ordering::Relaxed),
        tmean = mean_ns(TTFW_NS_SUM.load(Ordering::Relaxed), ttfw_n),
        tn = ttfw_n,
    )
}

/// Test/helper: force-enable without env (does not start dump thread).
#[cfg(test)]
pub fn force_enable_for_test(on: bool) {
    ENABLED_CACHE.store(if on { 1 } else { 0 }, Ordering::Relaxed);
}

#[cfg(test)]
pub fn test_counters() -> TestCounters {
    TestCounters {
        reuse_hit: REUSE_HIT.load(Ordering::Relaxed),
        reuse_miss: REUSE_MISS.load(Ordering::Relaxed),
        connect_ok: CONNECT_OK.load(Ordering::Relaxed),
        checkin_ok: CHECKIN_OK.load(Ordering::Relaxed),
        checkin_drop_full: CHECKIN_DROP_FULL.load(Ordering::Relaxed),
        checkout_wait_ns_max: CHECKOUT_WAIT_NS_MAX.load(Ordering::Relaxed),
        checkout_wait_ns_sum: CHECKOUT_WAIT_NS_SUM.load(Ordering::Relaxed),
        checkout_wait_samples: CHECKOUT_WAIT_SAMPLES.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub struct TestCounters {
    pub reuse_hit: u64,
    pub reuse_miss: u64,
    pub connect_ok: u64,
    pub checkin_ok: u64,
    pub checkin_drop_full: u64,
    pub checkout_wait_ns_max: u64,
    pub checkout_wait_ns_sum: u64,
    pub checkout_wait_samples: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn disabled_skips_counters() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        force_enable_for_test(false);
        reset();
        assert!(!enabled());
        note_reuse_hit(Duration::from_micros(10));
        assert_eq!(REUSE_HIT.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn counters_move_when_enabled() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        force_enable_for_test(true);
        reset();
        note_reuse_hit(Duration::from_micros(100));
        note_reuse_miss(
            Duration::from_micros(50),
            Duration::from_millis(1),
            ConnectOutcome::Ok,
        );
        note_idle_drop_dirty();
        note_checkin(true);
        note_checkin(false);
        note_not_poolable_drop();
        note_ttfw(Duration::from_micros(200));
        note_pool_idle(7);
        assert_eq!(REUSE_HIT.load(Ordering::Relaxed), 1);
        assert_eq!(REUSE_MISS.load(Ordering::Relaxed), 1);
        assert_eq!(CONNECT_OK.load(Ordering::Relaxed), 1);
        assert_eq!(IDLE_DROP_DIRTY.load(Ordering::Relaxed), 1);
        assert_eq!(CHECKIN_OK.load(Ordering::Relaxed), 1);
        assert_eq!(CHECKIN_DROP_FULL.load(Ordering::Relaxed), 1);
        assert_eq!(NOT_POOLABLE_DROP.load(Ordering::Relaxed), 1);
        assert_eq!(POOL_IDLE.load(Ordering::Relaxed), 7);
        let json = snapshot_json();
        assert!(json.contains("\"upstream_reuse_hit\":1"));
        assert!(json.contains("\"upstream_reuse_miss\":1"));
        force_enable_for_test(false);
        reset();
    }
}
