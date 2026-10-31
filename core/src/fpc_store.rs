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

    let vary_allowlist: Vec<&str> = ctx.vary_allowlist.iter().map(String::as_str).collect();
    let ttl = match fpc_assess_store(
        method,
        status,
        response_headers,
        body.len(),
        ctx.max_object_bytes,
        ctx.default_ttl,
        ctx.max_ttl,
        &vary_allowlist,
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
            std::sync::Arc::clone(&ctx.tags),
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
