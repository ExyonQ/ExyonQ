//! Bounded Redis provider metrics (no URL/site/node/endpoint labels).

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

static CONNECT: AtomicU64 = AtomicU64::new(0);
static CONNECT_FAIL: AtomicU64 = AtomicU64::new(0);
static RECONNECT: AtomicU64 = AtomicU64::new(0);
static CMD_TIMEOUT: AtomicU64 = AtomicU64::new(0);
static PUBLISH: AtomicU64 = AtomicU64::new(0);
static PUBLISH_FAIL: AtomicU64 = AtomicU64::new(0);
static RECEIVE: AtomicU64 = AtomicU64::new(0);
static ACK_FAIL: AtomicU64 = AtomicU64::new(0);
static RECONCILE: AtomicU64 = AtomicU64::new(0);
static RECONCILE_FAIL: AtomicU64 = AtomicU64::new(0);
static HMAC_REJECT: AtomicU64 = AtomicU64::new(0);
static REPLAY_REJECT: AtomicU64 = AtomicU64::new(0);
static DEGRADED: AtomicBool = AtomicBool::new(false);

pub(crate) fn note_connect() {
    CONNECT.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn note_connect_fail() {
    CONNECT_FAIL.fetch_add(1, Ordering::Relaxed);
    DEGRADED.store(true, Ordering::Relaxed);
}
pub(crate) fn note_reconnect() {
    RECONNECT.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn note_cmd_timeout() {
    CMD_TIMEOUT.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn note_publish() {
    PUBLISH.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn note_publish_fail() {
    PUBLISH_FAIL.fetch_add(1, Ordering::Relaxed);
    DEGRADED.store(true, Ordering::Relaxed);
}
pub(crate) fn note_receive() {
    RECEIVE.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn note_ack_fail() {
    ACK_FAIL.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn note_reconcile() {
    RECONCILE.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn note_reconcile_fail() {
    RECONCILE_FAIL.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn note_hmac_reject() {
    HMAC_REJECT.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn note_replay_reject() {
    REPLAY_REJECT.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn clear_degraded() {
    DEGRADED.store(false, Ordering::Relaxed);
}

pub fn l2_redis_connect_total() -> u64 {
    CONNECT.load(Ordering::Relaxed)
}
pub fn l2_redis_connect_failure_total() -> u64 {
    CONNECT_FAIL.load(Ordering::Relaxed)
}
pub fn l2_redis_reconnect_total() -> u64 {
    RECONNECT.load(Ordering::Relaxed)
}
pub fn l2_redis_command_timeout_total() -> u64 {
    CMD_TIMEOUT.load(Ordering::Relaxed)
}
pub fn l2_redis_publish_total() -> u64 {
    PUBLISH.load(Ordering::Relaxed)
}
pub fn l2_redis_publish_failure_total() -> u64 {
    PUBLISH_FAIL.load(Ordering::Relaxed)
}
pub fn l2_redis_receive_total() -> u64 {
    RECEIVE.load(Ordering::Relaxed)
}
pub fn l2_redis_ack_failure_total() -> u64 {
    ACK_FAIL.load(Ordering::Relaxed)
}
pub fn l2_redis_reconcile_total() -> u64 {
    RECONCILE.load(Ordering::Relaxed)
}
pub fn l2_redis_reconcile_failure_total() -> u64 {
    RECONCILE_FAIL.load(Ordering::Relaxed)
}
pub fn l2_redis_hmac_reject_total() -> u64 {
    HMAC_REJECT.load(Ordering::Relaxed)
}
pub fn l2_redis_replay_reject_total() -> u64 {
    REPLAY_REJECT.load(Ordering::Relaxed)
}
pub fn l2_redis_provider_degraded() -> bool {
    DEGRADED.load(Ordering::Relaxed)
}

pub fn reset_redis_metrics_for_tests() {
    CONNECT.store(0, Ordering::Relaxed);
    CONNECT_FAIL.store(0, Ordering::Relaxed);
    RECONNECT.store(0, Ordering::Relaxed);
    CMD_TIMEOUT.store(0, Ordering::Relaxed);
    PUBLISH.store(0, Ordering::Relaxed);
    PUBLISH_FAIL.store(0, Ordering::Relaxed);
    RECEIVE.store(0, Ordering::Relaxed);
    ACK_FAIL.store(0, Ordering::Relaxed);
    RECONCILE.store(0, Ordering::Relaxed);
    RECONCILE_FAIL.store(0, Ordering::Relaxed);
    HMAC_REJECT.store(0, Ordering::Relaxed);
    REPLAY_REJECT.store(0, Ordering::Relaxed);
    DEGRADED.store(false, Ordering::Relaxed);
}

thread_local! {
    static METRICS_TEST_LOCK_HELD: Cell<bool> = const { Cell::new(false) };
}

fn metrics_test_mutex() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Serializes reset/assert of process-wide Redis metric counters in unit tests
/// (KF-P16-010). Product counters remain lock-free atomics on the hot path.
pub fn lock_redis_metrics_for_tests() -> RedisMetricsTestGuard {
    let guard = metrics_test_mutex()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    METRICS_TEST_LOCK_HELD.with(|h| h.set(true));
    RedisMetricsTestGuard(guard)
}

pub struct RedisMetricsTestGuard(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for RedisMetricsTestGuard {
    fn drop(&mut self) {
        METRICS_TEST_LOCK_HELD.with(|h| h.set(false));
    }
}
