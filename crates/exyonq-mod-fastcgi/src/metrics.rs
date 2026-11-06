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
//! FastCGI module metrics — owned by exyonq-mod-fastcgi (KD1).

use exyonq_module_api::fcgi_dispatch::FcgiMetricsSnapshot;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

static FCGI_RESPONSES_501: AtomicU64 = AtomicU64::new(0);
static FCGI_RESPONSES_501_SUCCESS_NOT_AUTHORIZED: AtomicU64 = AtomicU64::new(0);
static FCGI_RESPONSES_200: AtomicU64 = AtomicU64::new(0);
static FCGI_RESPONSES_502: AtomicU64 = AtomicU64::new(0);
static FCGI_RESPONSES_503: AtomicU64 = AtomicU64::new(0);
static FCGI_RESPONSES_504: AtomicU64 = AtomicU64::new(0);
static FCGI_SATURATION_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static FCGI_INFLIGHT: AtomicU64 = AtomicU64::new(0);
static FCGI_CONN_CREATED: AtomicU64 = AtomicU64::new(0);
static FCGI_CONN_REUSED: AtomicU64 = AtomicU64::new(0);
static FCGI_CONN_DISCARDED: AtomicU64 = AtomicU64::new(0);
static FCGI_STALE_IDLE: AtomicU64 = AtomicU64::new(0);
static FCGI_SAFE_RETRIES: AtomicU64 = AtomicU64::new(0);
static FCGI_CHECKOUT_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
static FCGI_POOL_GENERATIONS: AtomicU64 = AtomicU64::new(0);
static FCGI_POOL_GENERATIONS_CREATED: AtomicU64 = AtomicU64::new(0);
static FCGI_POOL_GENERATIONS_DRAINED: AtomicU64 = AtomicU64::new(0);
static FCGI_POOL_GENERATIONS_RETIRED: AtomicU64 = AtomicU64::new(0);
static FCGI_POOL_GENERATION_ACTIVE: AtomicU64 = AtomicU64::new(0);
static FCGI_TIMEOUT_PHASE: AtomicU64 = AtomicU64::new(0);
static FCGI_CONNECT_FAILURES: AtomicU64 = AtomicU64::new(0);
static FCGI_FPM_DISCONNECTS: AtomicU64 = AtomicU64::new(0);
static FCGI_FPM_RECONNECTS: AtomicU64 = AtomicU64::new(0);
/// Last observed recovery latency in milliseconds (gauge; no high-cardinality labels).
static FCGI_RECOVERY_MS: AtomicU64 = AtomicU64::new(0);

/// Config-defined pool names are bounded; excess names share one overflow label.
const MAX_FCGI_POOL_METRIC_SERIES: usize = 64;

#[derive(Default)]
pub(crate) struct FcgiPoolMetrics {
    responses_501: AtomicU64,
    responses_200: AtomicU64,
    responses_502: AtomicU64,
    responses_503: AtomicU64,
    responses_504: AtomicU64,
    saturation_rejections: AtomicU64,
    checkout_timeouts: AtomicU64,
    inflight_current: AtomicI64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FcgiPoolSnapshot {
    pub responses_501: u64,
    pub responses_200: u64,
    pub responses_502: u64,
    pub responses_503: u64,
    pub responses_504: u64,
    pub saturation_rejections: u64,
    pub checkout_timeouts: u64,
    pub inflight_current: i64,
}

impl FcgiPoolMetrics {
    pub(crate) fn note_response(&self, status: u16) {
        match status {
            501 => self.responses_501.fetch_add(1, Ordering::Relaxed),
            200 => self.responses_200.fetch_add(1, Ordering::Relaxed),
            502 => self.responses_502.fetch_add(1, Ordering::Relaxed),
            503 => self.responses_503.fetch_add(1, Ordering::Relaxed),
            504 => self.responses_504.fetch_add(1, Ordering::Relaxed),
            _ => return,
        };
    }

    pub(crate) fn note_saturation_rejection(&self) {
        self.saturation_rejections.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_checkout_timeout(&self) {
        self.checkout_timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inflight_acquire(&self) {
        self.inflight_current.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inflight_release(&self) {
        self.inflight_current.fetch_sub(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> FcgiPoolSnapshot {
        FcgiPoolSnapshot {
            responses_501: self.responses_501.load(Ordering::Relaxed),
            responses_200: self.responses_200.load(Ordering::Relaxed),
            responses_502: self.responses_502.load(Ordering::Relaxed),
            responses_503: self.responses_503.load(Ordering::Relaxed),
            responses_504: self.responses_504.load(Ordering::Relaxed),
            saturation_rejections: self.saturation_rejections.load(Ordering::Relaxed),
            checkout_timeouts: self.checkout_timeouts.load(Ordering::Relaxed),
            inflight_current: self.inflight_current.load(Ordering::Relaxed),
        }
    }

    fn has_data(&self) -> bool {
        let snap = self.snapshot();
        snap.responses_501 != 0
            || snap.responses_200 != 0
            || snap.responses_502 != 0
            || snap.responses_503 != 0
            || snap.responses_504 != 0
            || snap.saturation_rejections != 0
            || snap.checkout_timeouts != 0
            || snap.inflight_current != 0
    }
}

static FCGI_POOL_METRICS: LazyLock<Mutex<BTreeMap<String, Arc<FcgiPoolMetrics>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static FCGI_POOL_OVERFLOW: LazyLock<Arc<FcgiPoolMetrics>> =
    LazyLock::new(|| Arc::new(FcgiPoolMetrics::default()));

/// Return the stable per-pool counter handle, registering it on first use.
pub(crate) fn pool_metrics(pool: &str) -> Arc<FcgiPoolMetrics> {
    let mut registry = FCGI_POOL_METRICS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(metrics) = registry.get(pool) {
        return Arc::clone(metrics);
    }
    if registry.len() >= MAX_FCGI_POOL_METRIC_SERIES {
        return Arc::clone(&FCGI_POOL_OVERFLOW);
    }
    let metrics = Arc::new(FcgiPoolMetrics::default());
    registry.insert(pool.to_string(), Arc::clone(&metrics));
    metrics
}

#[cfg(test)]
static FCGI_TEST_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub fn fcgi_metric_test_gate() -> FcgiMetricTestGateGuard {
    FcgiMetricTestGateGuard {
        _inner: FCGI_TEST_GATE
            .lock()
            .expect("fcgi metric test gate mutex poisoned"),
    }
}

#[cfg(test)]
pub struct FcgiMetricTestGateGuard {
    _inner: std::sync::MutexGuard<'static, ()>,
}

#[inline]
pub fn note_fcgi_response_501() {
    FCGI_RESPONSES_501.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_response_501_success_not_authorized() {
    FCGI_RESPONSES_501_SUCCESS_NOT_AUTHORIZED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_response_200() {
    FCGI_RESPONSES_200.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_response_502() {
    FCGI_RESPONSES_502.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_response_503() {
    FCGI_RESPONSES_503.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_response_504() {
    FCGI_RESPONSES_504.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_saturation_rejection() {
    FCGI_SATURATION_REJECTIONS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_inflight_acquire() {
    FCGI_INFLIGHT.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_inflight_release() {
    FCGI_INFLIGHT.fetch_sub(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_connection_created() {
    FCGI_CONN_CREATED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_connection_reused() {
    FCGI_CONN_REUSED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_connection_discarded() {
    FCGI_CONN_DISCARDED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_stale_idle() {
    FCGI_STALE_IDLE.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_safe_retry() {
    FCGI_SAFE_RETRIES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_checkout_timeout() {
    FCGI_CHECKOUT_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_pool_generation() {
    FCGI_POOL_GENERATIONS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_pool_generation_created() {
    FCGI_POOL_GENERATIONS_CREATED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_pool_generation_drained() {
    FCGI_POOL_GENERATIONS_DRAINED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_pool_generation_retired() {
    FCGI_POOL_GENERATIONS_RETIRED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn set_fcgi_pool_generation_active(generation: u64) {
    FCGI_POOL_GENERATION_ACTIVE.store(generation, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_timeout_phase_total() {
    FCGI_TIMEOUT_PHASE.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_connect_failure() {
    FCGI_CONNECT_FAILURES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_fpm_disconnect() {
    FCGI_FPM_DISCONNECTS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_fcgi_fpm_reconnect() {
    FCGI_FPM_RECONNECTS.fetch_add(1, Ordering::Relaxed);
}

/// Record recovery latency sample (`fcgi_recovery_seconds` gauge, stored as ms).
#[inline]
pub fn note_fcgi_recovery_seconds_sample(seconds: f64) {
    let ms = if seconds <= 0.0 {
        0
    } else {
        (seconds * 1000.0).round().clamp(0.0, u64::MAX as f64) as u64
    };
    FCGI_RECOVERY_MS.store(ms, Ordering::Relaxed);
}

/// Last recovery sample in seconds (`fcgi_recovery_seconds`).
#[inline]
pub fn fcgi_recovery_seconds() -> f64 {
    FCGI_RECOVERY_MS.load(Ordering::Relaxed) as f64 / 1000.0
}

pub fn pool_ops_snapshot() -> FcgiPoolOpsSnapshot {
    FcgiPoolOpsSnapshot {
        connections_created: FCGI_CONN_CREATED.load(Ordering::Relaxed),
        connections_reused: FCGI_CONN_REUSED.load(Ordering::Relaxed),
        connections_discarded: FCGI_CONN_DISCARDED.load(Ordering::Relaxed),
        stale_idle: FCGI_STALE_IDLE.load(Ordering::Relaxed),
        safe_retries: FCGI_SAFE_RETRIES.load(Ordering::Relaxed),
        checkout_timeouts: FCGI_CHECKOUT_TIMEOUTS.load(Ordering::Relaxed),
        pool_generations: FCGI_POOL_GENERATIONS.load(Ordering::Relaxed),
        pool_generations_created: FCGI_POOL_GENERATIONS_CREATED.load(Ordering::Relaxed),
        pool_generations_drained: FCGI_POOL_GENERATIONS_DRAINED.load(Ordering::Relaxed),
        pool_generations_retired: FCGI_POOL_GENERATIONS_RETIRED.load(Ordering::Relaxed),
        pool_generation_active: FCGI_POOL_GENERATION_ACTIVE.load(Ordering::Relaxed),
        timeout_phase_total: FCGI_TIMEOUT_PHASE.load(Ordering::Relaxed),
        connect_failures: FCGI_CONNECT_FAILURES.load(Ordering::Relaxed),
        fpm_disconnects: FCGI_FPM_DISCONNECTS.load(Ordering::Relaxed),
        fpm_reconnects: FCGI_FPM_RECONNECTS.load(Ordering::Relaxed),
        recovery_seconds_millis: FCGI_RECOVERY_MS.load(Ordering::Relaxed),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FcgiPoolOpsSnapshot {
    pub connections_created: u64,
    pub connections_reused: u64,
    pub connections_discarded: u64,
    pub stale_idle: u64,
    pub safe_retries: u64,
    pub checkout_timeouts: u64,
    pub pool_generations: u64,
    pub pool_generations_created: u64,
    pub pool_generations_drained: u64,
    pub pool_generations_retired: u64,
    pub pool_generation_active: u64,
    pub timeout_phase_total: u64,
    pub connect_failures: u64,
    pub fpm_disconnects: u64,
    pub fpm_reconnects: u64,
    /// Last recovery sample in milliseconds (`fcgi_recovery_seconds` * 1000).
    pub recovery_seconds_millis: u64,
}

pub fn snapshot() -> FcgiMetricsSnapshot {
    FcgiMetricsSnapshot {
        responses_501: FCGI_RESPONSES_501.load(Ordering::Relaxed),
        responses_501_success_not_authorized: FCGI_RESPONSES_501_SUCCESS_NOT_AUTHORIZED
            .load(Ordering::Relaxed),
        responses_200: FCGI_RESPONSES_200.load(Ordering::Relaxed),
        responses_502: FCGI_RESPONSES_502.load(Ordering::Relaxed),
        responses_503: FCGI_RESPONSES_503.load(Ordering::Relaxed),
        responses_504: FCGI_RESPONSES_504.load(Ordering::Relaxed),
        saturation_rejections: FCGI_SATURATION_REJECTIONS.load(Ordering::Relaxed),
        inflight_current: FCGI_INFLIGHT.load(Ordering::Relaxed),
    }
}

pub fn fcgi_responses_501_total() -> u64 {
    FCGI_RESPONSES_501.load(Ordering::Relaxed)
}

pub fn fcgi_responses_501_success_not_authorized_total() -> u64 {
    FCGI_RESPONSES_501_SUCCESS_NOT_AUTHORIZED.load(Ordering::Relaxed)
}

pub fn fcgi_responses_200_total() -> u64 {
    FCGI_RESPONSES_200.load(Ordering::Relaxed)
}

pub fn fcgi_responses_502_total() -> u64 {
    FCGI_RESPONSES_502.load(Ordering::Relaxed)
}

pub fn fcgi_responses_503_total() -> u64 {
    FCGI_RESPONSES_503.load(Ordering::Relaxed)
}

pub fn fcgi_responses_504_total() -> u64 {
    FCGI_RESPONSES_504.load(Ordering::Relaxed)
}

pub fn fcgi_saturation_rejections_total() -> u64 {
    FCGI_SATURATION_REJECTIONS.load(Ordering::Relaxed)
}

pub fn fcgi_inflight_current() -> u64 {
    FCGI_INFLIGHT.load(Ordering::Relaxed)
}

/// Q-08 / ADR-060 — export FastCGI counters as `exyonq_fcgi_*` for `/metrics`.
pub fn append_fcgi_prometheus(out: &mut String) {
    let snap = snapshot();
    let ops = pool_ops_snapshot();
    out.push_str("# TYPE exyonq_fcgi_responses_total counter\n");
    out.push_str(&format!(
        "exyonq_fcgi_responses_total{{status=\"200\"}} {}\n",
        snap.responses_200
    ));
    out.push_str(&format!(
        "exyonq_fcgi_responses_total{{status=\"502\"}} {}\n",
        snap.responses_502
    ));
    out.push_str(&format!(
        "exyonq_fcgi_responses_total{{status=\"503\"}} {}\n",
        snap.responses_503
    ));
    out.push_str(&format!(
        "exyonq_fcgi_responses_total{{status=\"504\"}} {}\n",
        snap.responses_504
    ));
    out.push_str("# TYPE exyonq_fcgi_saturation_rejections_total counter\n");
    out.push_str(&format!(
        "exyonq_fcgi_saturation_rejections_total {}\n",
        snap.saturation_rejections
    ));
    out.push_str("# TYPE exyonq_fcgi_checkout_timeouts_total counter\n");
    out.push_str(&format!(
        "exyonq_fcgi_checkout_timeouts_total {}\n",
        ops.checkout_timeouts
    ));
    out.push_str("# TYPE exyonq_fcgi_inflight_current gauge\n");
    out.push_str(&format!(
        "exyonq_fcgi_inflight_current {}\n",
        snap.inflight_current
    ));
    out.push_str("# TYPE exyonq_fcgi_connections_created_total counter\n");
    out.push_str(&format!(
        "exyonq_fcgi_connections_created_total {}\n",
        ops.connections_created
    ));
    out.push_str("# TYPE exyonq_fcgi_connections_reused_total counter\n");
    out.push_str(&format!(
        "exyonq_fcgi_connections_reused_total {}\n",
        ops.connections_reused
    ));
    out.push_str("# TYPE exyonq_fcgi_connect_failures_total counter\n");
    out.push_str(&format!(
        "exyonq_fcgi_connect_failures_total {}\n",
        ops.connect_failures
    ));
    append_fcgi_pool_prometheus(out);
}

fn append_fcgi_pool_prometheus(out: &mut String) {
    let snapshots: Vec<(String, FcgiPoolSnapshot)> = {
        let registry = FCGI_POOL_METRICS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry
            .iter()
            .map(|(name, metrics)| (name.clone(), metrics.snapshot()))
            .collect()
    };
    if snapshots.is_empty() {
        return;
    }

    out.push_str("# HELP exyonq_fcgi_pool_responses_total FastCGI responses by configured pool and status (process lifetime).\n");
    out.push_str("# TYPE exyonq_fcgi_pool_responses_total counter\n");
    for (pool, snap) in &snapshots {
        let label = escape_prometheus_label(pool);
        for (status, count) in [
            ("200", snap.responses_200),
            ("501", snap.responses_501),
            ("502", snap.responses_502),
            ("503", snap.responses_503),
            ("504", snap.responses_504),
        ] {
            out.push_str(&format!(
                "exyonq_fcgi_pool_responses_total{{pool=\"{label}\",status=\"{status}\"}} {count}\n"
            ));
        }
    }
    out.push_str("# TYPE exyonq_fcgi_pool_saturation_rejections_total counter\n");
    for (pool, snap) in &snapshots {
        out.push_str(&format!(
            "exyonq_fcgi_pool_saturation_rejections_total{{pool=\"{}\"}} {}\n",
            escape_prometheus_label(pool),
            snap.saturation_rejections
        ));
    }
    out.push_str("# TYPE exyonq_fcgi_pool_checkout_timeouts_total counter\n");
    for (pool, snap) in &snapshots {
        out.push_str(&format!(
            "exyonq_fcgi_pool_checkout_timeouts_total{{pool=\"{}\"}} {}\n",
            escape_prometheus_label(pool),
            snap.checkout_timeouts
        ));
    }
    out.push_str("# TYPE exyonq_fcgi_pool_inflight_current gauge\n");
    for (pool, snap) in &snapshots {
        out.push_str(&format!(
            "exyonq_fcgi_pool_inflight_current{{pool=\"{}\"}} {}\n",
            escape_prometheus_label(pool),
            snap.inflight_current
        ));
    }

    let overflow = &*FCGI_POOL_OVERFLOW;
    if overflow.has_data() {
        append_pool_series(out, "__overflow__", &overflow.snapshot());
    }
}

fn append_pool_series(out: &mut String, pool: &str, snap: &FcgiPoolSnapshot) {
    let label = escape_prometheus_label(pool);
    for (status, count) in [
        ("200", snap.responses_200),
        ("501", snap.responses_501),
        ("502", snap.responses_502),
        ("503", snap.responses_503),
        ("504", snap.responses_504),
    ] {
        out.push_str(&format!(
            "exyonq_fcgi_pool_responses_total{{pool=\"{label}\",status=\"{status}\"}} {count}\n"
        ));
    }
    out.push_str(&format!(
        "exyonq_fcgi_pool_saturation_rejections_total{{pool=\"{label}\"}} {}\n",
        snap.saturation_rejections
    ));
    out.push_str(&format!(
        "exyonq_fcgi_pool_checkout_timeouts_total{{pool=\"{label}\"}} {}\n",
        snap.checkout_timeouts
    ));
    out.push_str(&format!(
        "exyonq_fcgi_pool_inflight_current{{pool=\"{label}\"}} {}\n",
        snap.inflight_current
    ));
}

fn escape_prometheus_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increments_counters() {
        let _gate = fcgi_metric_test_gate();
        let before = snapshot();
        note_fcgi_response_502();
        note_fcgi_response_503();
        note_fcgi_saturation_rejection();
        note_fcgi_inflight_acquire();
        note_fcgi_inflight_release();
        let after = snapshot();
        assert_eq!(after.responses_502, before.responses_502 + 1);
        assert_eq!(after.responses_503, before.responses_503 + 1);
        assert_eq!(
            after.saturation_rejections,
            before.saturation_rejections + 1
        );
    }

    #[test]
    fn append_fcgi_prometheus_emits_q08_series() {
        let _gate = fcgi_metric_test_gate();
        note_fcgi_saturation_rejection();
        note_fcgi_checkout_timeout();
        let mut out = String::new();
        append_fcgi_prometheus(&mut out);
        assert!(out.contains("exyonq_fcgi_saturation_rejections_total"));
        assert!(out.contains("exyonq_fcgi_checkout_timeouts_total"));
        assert!(out.contains("exyonq_fcgi_inflight_current"));
        assert!(out.contains("exyonq_fcgi_responses_total"));
    }

    #[test]
    fn pool_metrics_are_exported_with_pool_label() {
        let _gate = fcgi_metric_test_gate();
        let name = format!("php-unit-{}", std::process::id());
        let metric = pool_metrics(&name);
        metric.note_response(200);
        metric.note_response(503);
        metric.note_saturation_rejection();
        metric.note_checkout_timeout();
        metric.inflight_acquire();
        metric.inflight_release();

        let mut out = String::new();
        append_fcgi_prometheus(&mut out);
        assert!(out.contains(&format!(
            "exyonq_fcgi_pool_responses_total{{pool=\"{name}\",status=\"200\"}} 1"
        )));
        assert!(out.contains(&format!(
            "exyonq_fcgi_pool_responses_total{{pool=\"{name}\",status=\"503\"}} 1"
        )));
        assert!(out.contains(&format!(
            "exyonq_fcgi_pool_saturation_rejections_total{{pool=\"{name}\"}} 1"
        )));
        assert!(out.contains(&format!(
            "exyonq_fcgi_pool_checkout_timeouts_total{{pool=\"{name}\"}} 1"
        )));
        assert!(out.contains(&format!(
            "exyonq_fcgi_pool_inflight_current{{pool=\"{name}\"}} 0"
        )));
    }
}
