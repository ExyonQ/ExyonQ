pub mod coordination;
pub mod direct_purge;
pub mod global;
pub mod key;
pub mod metrics;
pub mod namespace;
pub mod serve;
pub mod singleflight;
pub mod store;

pub use coordination::{recv_timeout, LocalCoordinationHub, LocalCoordinationProvider};
pub use direct_purge::{test_has, test_insert, DirectL1PurgePort};
pub use global::{global_response_cache, global_singleflight, reset_global_for_tests, GlobalCache};
pub use key::{
    build_cache_key, build_storage_cache_key, canonicalize_query, canonicalize_query_safe,
    is_tracking_query_param, normalize_host, CacheKey, CacheKeyParts,
};
pub use metrics::{
    cache_bytes_current, cache_entries_current, cache_evictions_total, cache_expired_total,
    cache_hits_total, cache_insertions_total, cache_misses_total, cache_rejections_total,
    cache_singleflight_followers_total, cache_singleflight_leaders_total, fpc_bypass_total,
    fpc_eviction_total, fpc_hit_total, fpc_lookup_error_total, fpc_lookup_total, fpc_miss_total,
    fpc_purge_bytes_total, fpc_purge_duration_us_total, fpc_purge_entries_total,
    fpc_purge_rejected_total, fpc_purge_requests_total, fpc_purge_success_total,
    fpc_store_attempt_total, fpc_store_body_bytes_total, fpc_store_error_total,
    fpc_store_rejected_total, fpc_store_success_total, l2_generation_advance_total,
    l2_generation_rollback_rejected_total, l2_invalidation_duplicate_total,
    l2_invalidation_publish_failure_total, l2_invalidation_publish_total,
    l2_invalidation_receive_total, l2_invalidation_rejected_total, l2_subscriber_overflow_total,
    l2_subscriber_restart_total, note_fpc_bypass, note_fpc_hit, note_fpc_lookup,
    note_fpc_lookup_error, note_fpc_miss, note_fpc_purge_rejected, note_fpc_purge_request,
    note_fpc_purge_success, note_fpc_store_attempt, note_fpc_store_error, note_fpc_store_eviction,
    note_fpc_store_rejected, note_fpc_store_success, reset_metrics_for_tests,
};
pub use namespace::{CacheNamespace, HitValidation, HitValidator, NamespaceMetrics, ServeHooks};
pub use serve::{
    entry_to_served, serve_with_cache, CacheLoadOutcome, CacheServeOutcome, ServedResponse,
};
pub use singleflight::{set_singleflight_follower_hook_for_tests, Singleflight};
pub use store::{CacheStoreSnapshot, CachedEntry, PurgeStats, ResponseCache};
