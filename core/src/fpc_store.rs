//! WC2C — bounded L1 insert after complete origin response (fail-open).

use crate::fpc_lookup::FpcMissContext;
use bytes::Bytes;
use exyonq_cache::{
    cache_evictions_total, note_fpc_store_attempt, note_fpc_store_error, note_fpc_store_eviction,
    note_fpc_store_rejected, note_fpc_store_success, CacheNamespace, NamespaceMetrics,
};
use exyonq_module_api::{filter_storable_headers, fpc_assess_store};

/// Attempt to store a completed origin response into the FPC L1.
///
/// Fail-open: never alters the origin response. HEAD never stores. Only GET may populate.
pub fn try_fpc_store_response(
    ctx: &FpcMissContext,
    method: &str,
    status: u16,
    response_headers: &[(String, String)],
    body: Bytes,
) {
    note_fpc_store_attempt();

    let ttl = match fpc_assess_store(
        method,
        status,
        response_headers,
        body.len(),
        ctx.max_object_bytes,
        ctx.default_ttl,
        ctx.max_ttl,
    ) {
        Ok(ttl) => ttl,
        Err(reason) => {
            note_fpc_store_rejected(reason.as_metric_label());
            return;
        }
    };

    let mut headers = filter_storable_headers(response_headers);
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-length"));
    headers.push(("content-length".into(), body.len().to_string()));
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("age"));

    let body_len = body.len();
    let before_evict = cache_evictions_total();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.cache.insert_entry(
            ctx.key.clone(),
            ctx.site_id,
            ctx.route_idx,
            ctx.runtime_generation,
            ttl,
            status,
            headers,
            body,
            None,
            CacheNamespace::new(ctx.namespace),
            std::sync::Arc::from([]),
            NamespaceMetrics::NONE,
        );
    })) {
        Ok(()) => {
            let evicted = cache_evictions_total().saturating_sub(before_evict);
            for _ in 0..evicted {
                note_fpc_store_eviction();
            }
            note_fpc_store_success(body_len);
        }
        Err(_) => {
            note_fpc_store_error();
        }
    }
}
