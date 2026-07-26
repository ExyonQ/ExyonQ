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
use std::sync::atomic::{AtomicU64, Ordering};

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
}
