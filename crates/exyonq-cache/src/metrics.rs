use std::cell::Cell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static CACHE_INSERTIONS: AtomicU64 = AtomicU64::new(0);
static CACHE_EVICTIONS: AtomicU64 = AtomicU64::new(0);
static CACHE_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static CACHE_EXPIRED: AtomicU64 = AtomicU64::new(0);
static CACHE_SINGLEFLIGHT_LEADERS: AtomicU64 = AtomicU64::new(0);
static CACHE_SINGLEFLIGHT_FOLLOWERS: AtomicU64 = AtomicU64::new(0);
static CACHE_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static CACHE_BYTES: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn sync_size_metrics(entries: usize, bytes: usize) {
    // Under `cfg(test)`, size gauges are process-singleton; take the same lock
    // used by metric-assert tests so parallel ResponseCache instances cannot
    // overwrite gauges mid-assert. Re-entrant for the thread that already holds
    // `lock_size_metrics_for_tests` (KF-P16-008).
    #[cfg(test)]
    let _guard = size_metrics_lock_for_sync();
    CACHE_ENTRIES.store(entries, Ordering::Relaxed);
    CACHE_BYTES.store(bytes, Ordering::Relaxed);
}

thread_local! {
    static SIZE_METRICS_LOCK_HELD: Cell<bool> = const { Cell::new(false) };
}

fn size_metrics_mutex() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
fn size_metrics_lock_for_sync() -> Option<MutexGuard<'static, ()>> {
    if SIZE_METRICS_LOCK_HELD.with(|h| h.get()) {
        None
    } else {
        Some(
            size_metrics_mutex()
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        )
    }
}

/// Process-wide size gauges are a deliberate product singleton (one active cache).
/// Hold across create→mutate→assert when calling `metrics_match_store` in tests.
#[doc(hidden)]
pub fn lock_size_metrics_for_tests() -> SizeMetricsTestGuard {
    let guard = size_metrics_mutex()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    SIZE_METRICS_LOCK_HELD.with(|h| h.set(true));
    SizeMetricsTestGuard(guard)
}

#[doc(hidden)]
pub struct SizeMetricsTestGuard(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for SizeMetricsTestGuard {
    fn drop(&mut self) {
        SIZE_METRICS_LOCK_HELD.with(|h| h.set(false));
    }
}

pub fn note_hit() {
    CACHE_HITS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_miss() {
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
}

pub fn note_insertion() {
    CACHE_INSERTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_eviction() {
    CACHE_EVICTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_rejection(_reason: exyonq_module_api::CacheRejection) {
    CACHE_REJECTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_expired() {
    CACHE_EXPIRED.fetch_add(1, Ordering::Relaxed);
}

pub fn note_singleflight_leader() {
    CACHE_SINGLEFLIGHT_LEADERS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_singleflight_follower() {
    CACHE_SINGLEFLIGHT_FOLLOWERS.fetch_add(1, Ordering::Relaxed);
    #[cfg(any(debug_assertions, feature = "test-utils"))]
    if let Some(hook) = super::singleflight::singleflight_follower_hook() {
        hook();
    }
}

pub fn cache_hits_total() -> u64 {
    CACHE_HITS.load(Ordering::Relaxed)
}

pub fn cache_misses_total() -> u64 {
    CACHE_MISSES.load(Ordering::Relaxed)
}

pub fn cache_insertions_total() -> u64 {
    CACHE_INSERTIONS.load(Ordering::Relaxed)
}

pub fn cache_evictions_total() -> u64 {
    CACHE_EVICTIONS.load(Ordering::Relaxed)
}

pub fn cache_rejections_total() -> u64 {
    CACHE_REJECTIONS.load(Ordering::Relaxed)
}

pub fn cache_expired_total() -> u64 {
    CACHE_EXPIRED.load(Ordering::Relaxed)
}

pub fn cache_singleflight_leaders_total() -> u64 {
    CACHE_SINGLEFLIGHT_LEADERS.load(Ordering::Relaxed)
}

pub fn cache_singleflight_followers_total() -> u64 {
    CACHE_SINGLEFLIGHT_FOLLOWERS.load(Ordering::Relaxed)
}

pub fn cache_entries_current() -> usize {
    CACHE_ENTRIES.load(Ordering::Relaxed)
}

pub fn cache_bytes_current() -> usize {
    CACHE_BYTES.load(Ordering::Relaxed)
}

static FPC_LOOKUP: AtomicU64 = AtomicU64::new(0);
static FPC_HIT: AtomicU64 = AtomicU64::new(0);
static FPC_MISS: AtomicU64 = AtomicU64::new(0);
static FPC_BYPASS: AtomicU64 = AtomicU64::new(0);
static FPC_LOOKUP_ERROR: AtomicU64 = AtomicU64::new(0);

pub fn note_fpc_lookup() {
    FPC_LOOKUP.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fpc_hit() {
    FPC_HIT.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fpc_miss() {
    FPC_MISS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fpc_bypass() {
    FPC_BYPASS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fpc_lookup_error() {
    FPC_LOOKUP_ERROR.fetch_add(1, Ordering::Relaxed);
}

pub fn fpc_lookup_total() -> u64 {
    FPC_LOOKUP.load(Ordering::Relaxed)
}

pub fn fpc_hit_total() -> u64 {
    FPC_HIT.load(Ordering::Relaxed)
}

pub fn fpc_miss_total() -> u64 {
    FPC_MISS.load(Ordering::Relaxed)
}

pub fn fpc_bypass_total() -> u64 {
    FPC_BYPASS.load(Ordering::Relaxed)
}

pub fn fpc_lookup_error_total() -> u64 {
    FPC_LOOKUP_ERROR.load(Ordering::Relaxed)
}

static FPC_STORE_ATTEMPT: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_SUCCESS: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_ERROR: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_BODY_BYTES: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_EVICTION: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_PRIVATE: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_NO_STORE: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_NO_CACHE: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_SET_COOKIE: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_UNSUPPORTED_VARY: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_STATUS: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_STREAMING: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_BODY_TOO_LARGE: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_TTL_INVALID: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_CACHE_DISABLED: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_METHOD: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_HEAD: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_HOP_BY_HOP: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_CONTENT_RANGE: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_GENERATION_MISMATCH: AtomicU64 = AtomicU64::new(0);
static FPC_STORE_REJECT_OTHER: AtomicU64 = AtomicU64::new(0);

static FPC_PURGE_REQUESTS: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_SUCCESS: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_ENTRIES: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_BYTES: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_DURATION_US: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_REJECT_UNAUTHENTICATED: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_REJECT_UNAUTHORIZED: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_REJECT_INVALID_SCOPE: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_REJECT_INVALID_KEY: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_REJECT_RATE_LIMITED: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_REJECT_INTERNAL: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_REJECT_UNSUPPORTED: AtomicU64 = AtomicU64::new(0);
static FPC_PURGE_REJECT_OTHER: AtomicU64 = AtomicU64::new(0);

pub fn note_fpc_store_attempt() {
    FPC_STORE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fpc_store_success(body_bytes: usize) {
    FPC_STORE_SUCCESS.fetch_add(1, Ordering::Relaxed);
    FPC_STORE_BODY_BYTES.fetch_add(body_bytes as u64, Ordering::Relaxed);
}

pub fn note_fpc_store_error() {
    FPC_STORE_ERROR.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fpc_store_eviction() {
    FPC_STORE_EVICTION.fetch_add(1, Ordering::Relaxed);
}

/// Bounded store-reject reasons (WC2C). Unknown labels fold into `other`.
pub fn note_fpc_store_rejected(reason: &str) {
    match reason {
        "private" => {
            FPC_STORE_REJECT_PRIVATE.fetch_add(1, Ordering::Relaxed);
        }
        "no_store" => {
            FPC_STORE_REJECT_NO_STORE.fetch_add(1, Ordering::Relaxed);
        }
        "no_cache" => {
            FPC_STORE_REJECT_NO_CACHE.fetch_add(1, Ordering::Relaxed);
        }
        "set_cookie" => {
            FPC_STORE_REJECT_SET_COOKIE.fetch_add(1, Ordering::Relaxed);
        }
        "unsupported_vary" => {
            FPC_STORE_REJECT_UNSUPPORTED_VARY.fetch_add(1, Ordering::Relaxed);
        }
        "status" => {
            FPC_STORE_REJECT_STATUS.fetch_add(1, Ordering::Relaxed);
        }
        "streaming" => {
            FPC_STORE_REJECT_STREAMING.fetch_add(1, Ordering::Relaxed);
        }
        "body_too_large" => {
            FPC_STORE_REJECT_BODY_TOO_LARGE.fetch_add(1, Ordering::Relaxed);
        }
        "ttl_invalid" => {
            FPC_STORE_REJECT_TTL_INVALID.fetch_add(1, Ordering::Relaxed);
        }
        "cache_disabled" => {
            FPC_STORE_REJECT_CACHE_DISABLED.fetch_add(1, Ordering::Relaxed);
        }
        "method" => {
            FPC_STORE_REJECT_METHOD.fetch_add(1, Ordering::Relaxed);
        }
        "head_must_not_store" => {
            FPC_STORE_REJECT_HEAD.fetch_add(1, Ordering::Relaxed);
        }
        "hop_by_hop" => {
            FPC_STORE_REJECT_HOP_BY_HOP.fetch_add(1, Ordering::Relaxed);
        }
        "content_range" => {
            FPC_STORE_REJECT_CONTENT_RANGE.fetch_add(1, Ordering::Relaxed);
        }
        "generation_mismatch" => {
            FPC_STORE_REJECT_GENERATION_MISMATCH.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            FPC_STORE_REJECT_OTHER.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn note_fpc_purge_request() {
    FPC_PURGE_REQUESTS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fpc_purge_success(entries: u64, bytes: u64, duration_us: u64) {
    FPC_PURGE_SUCCESS.fetch_add(1, Ordering::Relaxed);
    FPC_PURGE_ENTRIES.fetch_add(entries, Ordering::Relaxed);
    FPC_PURGE_BYTES.fetch_add(bytes, Ordering::Relaxed);
    FPC_PURGE_DURATION_US.fetch_add(duration_us, Ordering::Relaxed);
}

/// Bounded purge-reject reasons (WC3). Unknown labels fold into `other`.
pub fn note_fpc_purge_rejected(reason: &str) {
    match reason {
        "unauthenticated" => {
            FPC_PURGE_REJECT_UNAUTHENTICATED.fetch_add(1, Ordering::Relaxed);
        }
        "unauthorized" => {
            FPC_PURGE_REJECT_UNAUTHORIZED.fetch_add(1, Ordering::Relaxed);
        }
        "invalid_scope" => {
            FPC_PURGE_REJECT_INVALID_SCOPE.fetch_add(1, Ordering::Relaxed);
        }
        "invalid_key" => {
            FPC_PURGE_REJECT_INVALID_KEY.fetch_add(1, Ordering::Relaxed);
        }
        "rate_limited" => {
            FPC_PURGE_REJECT_RATE_LIMITED.fetch_add(1, Ordering::Relaxed);
        }
        "internal_error" => {
            FPC_PURGE_REJECT_INTERNAL.fetch_add(1, Ordering::Relaxed);
        }
        "unsupported_operation" => {
            FPC_PURGE_REJECT_UNSUPPORTED.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            FPC_PURGE_REJECT_OTHER.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn fpc_purge_requests_total() -> u64 {
    FPC_PURGE_REQUESTS.load(Ordering::Relaxed)
}

pub fn fpc_purge_success_total() -> u64 {
    FPC_PURGE_SUCCESS.load(Ordering::Relaxed)
}

pub fn fpc_purge_entries_total() -> u64 {
    FPC_PURGE_ENTRIES.load(Ordering::Relaxed)
}

pub fn fpc_purge_bytes_total() -> u64 {
    FPC_PURGE_BYTES.load(Ordering::Relaxed)
}

pub fn fpc_purge_duration_us_total() -> u64 {
    FPC_PURGE_DURATION_US.load(Ordering::Relaxed)
}

pub fn fpc_purge_rejected_total(reason: &str) -> u64 {
    match reason {
        "unauthenticated" => FPC_PURGE_REJECT_UNAUTHENTICATED.load(Ordering::Relaxed),
        "unauthorized" => FPC_PURGE_REJECT_UNAUTHORIZED.load(Ordering::Relaxed),
        "invalid_scope" => FPC_PURGE_REJECT_INVALID_SCOPE.load(Ordering::Relaxed),
        "invalid_key" => FPC_PURGE_REJECT_INVALID_KEY.load(Ordering::Relaxed),
        "rate_limited" => FPC_PURGE_REJECT_RATE_LIMITED.load(Ordering::Relaxed),
        "internal_error" => FPC_PURGE_REJECT_INTERNAL.load(Ordering::Relaxed),
        "unsupported_operation" => FPC_PURGE_REJECT_UNSUPPORTED.load(Ordering::Relaxed),
        _ => FPC_PURGE_REJECT_OTHER.load(Ordering::Relaxed),
    }
}

// --- WC7B1 L2 coordination metrics (bounded reasons; no URL/site/node labels) ---

static L2_INV_PUBLISH: AtomicU64 = AtomicU64::new(0);
static L2_INV_PUBLISH_FAIL: AtomicU64 = AtomicU64::new(0);
static L2_INV_RECEIVE: AtomicU64 = AtomicU64::new(0);
static L2_INV_REJECT: AtomicU64 = AtomicU64::new(0);
static L2_INV_DUP: AtomicU64 = AtomicU64::new(0);
static L2_GEN_ADVANCE: AtomicU64 = AtomicU64::new(0);
static L2_GEN_ROLLBACK: AtomicU64 = AtomicU64::new(0);
static L2_SUB_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static L2_SUB_RESTART: AtomicU64 = AtomicU64::new(0);

pub fn note_l2_invalidation_publish() {
    L2_INV_PUBLISH.fetch_add(1, Ordering::Relaxed);
}

pub fn note_l2_invalidation_publish_failure(_reason: &str) {
    L2_INV_PUBLISH_FAIL.fetch_add(1, Ordering::Relaxed);
}

pub fn note_l2_invalidation_receive() {
    L2_INV_RECEIVE.fetch_add(1, Ordering::Relaxed);
}

pub fn note_l2_invalidation_rejected(_reason: &str) {
    L2_INV_REJECT.fetch_add(1, Ordering::Relaxed);
}

pub fn note_l2_invalidation_duplicate() {
    L2_INV_DUP.fetch_add(1, Ordering::Relaxed);
}

pub fn note_l2_generation_advance() {
    L2_GEN_ADVANCE.fetch_add(1, Ordering::Relaxed);
}

pub fn note_l2_generation_rollback_rejected() {
    L2_GEN_ROLLBACK.fetch_add(1, Ordering::Relaxed);
}

pub fn note_l2_subscriber_overflow() {
    L2_SUB_OVERFLOW.fetch_add(1, Ordering::Relaxed);
}

pub fn note_l2_subscriber_restart() {
    L2_SUB_RESTART.fetch_add(1, Ordering::Relaxed);
}

pub fn l2_invalidation_publish_total() -> u64 {
    L2_INV_PUBLISH.load(Ordering::Relaxed)
}

pub fn l2_invalidation_publish_failure_total() -> u64 {
    L2_INV_PUBLISH_FAIL.load(Ordering::Relaxed)
}

pub fn l2_invalidation_receive_total() -> u64 {
    L2_INV_RECEIVE.load(Ordering::Relaxed)
}

pub fn l2_invalidation_rejected_total() -> u64 {
    L2_INV_REJECT.load(Ordering::Relaxed)
}

pub fn l2_invalidation_duplicate_total() -> u64 {
    L2_INV_DUP.load(Ordering::Relaxed)
}

pub fn l2_generation_advance_total() -> u64 {
    L2_GEN_ADVANCE.load(Ordering::Relaxed)
}

pub fn l2_generation_rollback_rejected_total() -> u64 {
    L2_GEN_ROLLBACK.load(Ordering::Relaxed)
}

pub fn l2_subscriber_overflow_total() -> u64 {
    L2_SUB_OVERFLOW.load(Ordering::Relaxed)
}

pub fn l2_subscriber_restart_total() -> u64 {
    L2_SUB_RESTART.load(Ordering::Relaxed)
}

pub fn fpc_store_attempt_total() -> u64 {
    FPC_STORE_ATTEMPT.load(Ordering::Relaxed)
}

pub fn fpc_store_success_total() -> u64 {
    FPC_STORE_SUCCESS.load(Ordering::Relaxed)
}

pub fn fpc_store_error_total() -> u64 {
    FPC_STORE_ERROR.load(Ordering::Relaxed)
}

pub fn fpc_store_body_bytes_total() -> u64 {
    FPC_STORE_BODY_BYTES.load(Ordering::Relaxed)
}

pub fn fpc_eviction_total() -> u64 {
    FPC_STORE_EVICTION.load(Ordering::Relaxed)
}

pub fn fpc_store_rejected_total(reason: &str) -> u64 {
    match reason {
        "private" => FPC_STORE_REJECT_PRIVATE.load(Ordering::Relaxed),
        "no_store" => FPC_STORE_REJECT_NO_STORE.load(Ordering::Relaxed),
        "no_cache" => FPC_STORE_REJECT_NO_CACHE.load(Ordering::Relaxed),
        "set_cookie" => FPC_STORE_REJECT_SET_COOKIE.load(Ordering::Relaxed),
        "unsupported_vary" => FPC_STORE_REJECT_UNSUPPORTED_VARY.load(Ordering::Relaxed),
        "status" => FPC_STORE_REJECT_STATUS.load(Ordering::Relaxed),
        "streaming" => FPC_STORE_REJECT_STREAMING.load(Ordering::Relaxed),
        "body_too_large" => FPC_STORE_REJECT_BODY_TOO_LARGE.load(Ordering::Relaxed),
        "ttl_invalid" => FPC_STORE_REJECT_TTL_INVALID.load(Ordering::Relaxed),
        "cache_disabled" => FPC_STORE_REJECT_CACHE_DISABLED.load(Ordering::Relaxed),
        "method" => FPC_STORE_REJECT_METHOD.load(Ordering::Relaxed),
        "head_must_not_store" => FPC_STORE_REJECT_HEAD.load(Ordering::Relaxed),
        "hop_by_hop" => FPC_STORE_REJECT_HOP_BY_HOP.load(Ordering::Relaxed),
        "content_range" => FPC_STORE_REJECT_CONTENT_RANGE.load(Ordering::Relaxed),
        "generation_mismatch" => FPC_STORE_REJECT_GENERATION_MISMATCH.load(Ordering::Relaxed),
        _ => FPC_STORE_REJECT_OTHER.load(Ordering::Relaxed),
    }
}

pub fn reset_metrics_for_tests() {
    CACHE_HITS.store(0, Ordering::Relaxed);
    CACHE_MISSES.store(0, Ordering::Relaxed);
    CACHE_INSERTIONS.store(0, Ordering::Relaxed);
    CACHE_EVICTIONS.store(0, Ordering::Relaxed);
    CACHE_REJECTIONS.store(0, Ordering::Relaxed);
    CACHE_EXPIRED.store(0, Ordering::Relaxed);
    CACHE_SINGLEFLIGHT_LEADERS.store(0, Ordering::Relaxed);
    CACHE_SINGLEFLIGHT_FOLLOWERS.store(0, Ordering::Relaxed);
    CACHE_ENTRIES.store(0, Ordering::Relaxed);
    CACHE_BYTES.store(0, Ordering::Relaxed);
    FPC_LOOKUP.store(0, Ordering::Relaxed);
    FPC_HIT.store(0, Ordering::Relaxed);
    FPC_MISS.store(0, Ordering::Relaxed);
    FPC_BYPASS.store(0, Ordering::Relaxed);
    FPC_LOOKUP_ERROR.store(0, Ordering::Relaxed);
    FPC_STORE_ATTEMPT.store(0, Ordering::Relaxed);
    FPC_STORE_SUCCESS.store(0, Ordering::Relaxed);
    FPC_STORE_ERROR.store(0, Ordering::Relaxed);
    FPC_STORE_BODY_BYTES.store(0, Ordering::Relaxed);
    FPC_STORE_EVICTION.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_PRIVATE.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_NO_STORE.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_NO_CACHE.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_SET_COOKIE.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_UNSUPPORTED_VARY.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_STATUS.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_STREAMING.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_BODY_TOO_LARGE.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_TTL_INVALID.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_CACHE_DISABLED.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_METHOD.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_HEAD.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_HOP_BY_HOP.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_CONTENT_RANGE.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_GENERATION_MISMATCH.store(0, Ordering::Relaxed);
    FPC_STORE_REJECT_OTHER.store(0, Ordering::Relaxed);
    FPC_PURGE_REQUESTS.store(0, Ordering::Relaxed);
    FPC_PURGE_SUCCESS.store(0, Ordering::Relaxed);
    FPC_PURGE_ENTRIES.store(0, Ordering::Relaxed);
    FPC_PURGE_BYTES.store(0, Ordering::Relaxed);
    FPC_PURGE_DURATION_US.store(0, Ordering::Relaxed);
    FPC_PURGE_REJECT_UNAUTHENTICATED.store(0, Ordering::Relaxed);
    FPC_PURGE_REJECT_UNAUTHORIZED.store(0, Ordering::Relaxed);
    FPC_PURGE_REJECT_INVALID_SCOPE.store(0, Ordering::Relaxed);
    FPC_PURGE_REJECT_INVALID_KEY.store(0, Ordering::Relaxed);
    FPC_PURGE_REJECT_RATE_LIMITED.store(0, Ordering::Relaxed);
    FPC_PURGE_REJECT_INTERNAL.store(0, Ordering::Relaxed);
    FPC_PURGE_REJECT_UNSUPPORTED.store(0, Ordering::Relaxed);
    FPC_PURGE_REJECT_OTHER.store(0, Ordering::Relaxed);
    L2_INV_PUBLISH.store(0, Ordering::Relaxed);
    L2_INV_PUBLISH_FAIL.store(0, Ordering::Relaxed);
    L2_INV_RECEIVE.store(0, Ordering::Relaxed);
    L2_INV_REJECT.store(0, Ordering::Relaxed);
    L2_INV_DUP.store(0, Ordering::Relaxed);
    L2_GEN_ADVANCE.store(0, Ordering::Relaxed);
    L2_GEN_ROLLBACK.store(0, Ordering::Relaxed);
    L2_SUB_OVERFLOW.store(0, Ordering::Relaxed);
    L2_SUB_RESTART.store(0, Ordering::Relaxed);
}
