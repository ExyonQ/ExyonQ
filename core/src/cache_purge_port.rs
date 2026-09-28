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
//! WC3 — CachePurgePort over SharedServerState.fpc_cache (off request hot path).

use crate::reload::{self, SharedServerState};
use exyonq_cache::{
    build_storage_cache_key, normalize_host, note_fpc_purge_rejected, note_fpc_purge_request,
    note_fpc_purge_success, CacheKeyParts,
};
use exyonq_module_api::cache_purge::{CachePurgeOp, CachePurgeOutcome, CachePurgePort};
use exyonq_module_api::{fpc_canonicalize_path, fpc_query_decision};
use std::sync::Arc;
use std::time::Instant;

pub struct CoreCachePurgePort {
    pub shared: SharedServerState,
}

impl CoreCachePurgePort {
    fn resolve_route_for_site(
        state: &crate::server::state::ServerState,
        site_id: u64,
    ) -> Option<usize> {
        state
            .snapshot
            .full_page_cache
            .route_site_ids
            .iter()
            .position(|&id| id == site_id)
    }

    fn purge_committed(&self, op: CachePurgeOp) -> CachePurgeOutcome {
        note_fpc_purge_request();
        let started = Instant::now();
        let state = reload::read_state(&self.shared);
        let generation = state.generation;

        let Some(cache) = state.fpc_cache.as_ref() else {
            note_fpc_purge_rejected("internal_error");
            let site = match &op {
                CachePurgeOp::Url { site_id, .. }
                | CachePurgeOp::Site { site_id }
                | CachePurgeOp::Generation { site_id, .. }
                | CachePurgeOp::Tag { site_id, .. } => *site_id,
            };
            return CachePurgeOutcome::fail(op_name(&op), site, generation, "internal_error");
        };

        match op {
            CachePurgeOp::Tag { site_id, .. } => {
                note_fpc_purge_rejected("unsupported_operation");
                CachePurgeOutcome::fail("purge.tag", site_id, generation, "unsupported_operation")
            }
            CachePurgeOp::Site { site_id } => {
                if Self::resolve_route_for_site(&state, site_id).is_none() {
                    note_fpc_purge_rejected("unauthorized");
                    return CachePurgeOutcome::fail(
                        "purge.site",
                        site_id,
                        generation,
                        "unauthorized",
                    );
                }
                let stats = cache.invalidate_site(site_id);
                let us = started.elapsed().as_micros() as u64;
                note_fpc_purge_success(stats.purged_entries, stats.purged_bytes, us);
                CachePurgeOutcome::success(
                    "purge.site",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    generation,
                )
            }
            CachePurgeOp::Generation {
                site_id,
                generation: target_gen,
            } => {
                if Self::resolve_route_for_site(&state, site_id).is_none() {
                    note_fpc_purge_rejected("unauthorized");
                    return CachePurgeOutcome::fail(
                        "purge.generation",
                        site_id,
                        generation,
                        "unauthorized",
                    );
                }
                let stats = cache.invalidate_site_runtime_generation(site_id, target_gen);
                let us = started.elapsed().as_micros() as u64;
                note_fpc_purge_success(stats.purged_entries, stats.purged_bytes, us);
                CachePurgeOutcome::success(
                    "purge.generation",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    generation,
                )
            }
            CachePurgeOp::Url {
                site_id,
                scheme,
                host,
                path,
                query,
            } => {
                let Some(route_idx) = Self::resolve_route_for_site(&state, site_id) else {
                    note_fpc_purge_rejected("unauthorized");
                    return CachePurgeOutcome::fail(
                        "purge.url",
                        site_id,
                        generation,
                        "unauthorized",
                    );
                };
                let Some(backend_id) = state.snapshot.route_backend_id(route_idx) else {
                    note_fpc_purge_rejected("invalid_scope");
                    return CachePurgeOutcome::fail(
                        "purge.url",
                        site_id,
                        generation,
                        "invalid_scope",
                    );
                };
                let Some(canon_path) = fpc_canonicalize_path(&path) else {
                    note_fpc_purge_rejected("invalid_key");
                    return CachePurgeOutcome::fail(
                        "purge.url",
                        site_id,
                        generation,
                        "invalid_key",
                    );
                };
                let canon_query = match fpc_query_decision(&query, &[]) {
                    exyonq_module_api::FpcQueryDecision::Eligible { canonical } => canonical,
                    exyonq_module_api::FpcQueryDecision::Bypass(_) => {
                        note_fpc_purge_rejected("invalid_key");
                        return CachePurgeOutcome::fail(
                            "purge.url",
                            site_id,
                            generation,
                            "invalid_key",
                        );
                    }
                };
                let key = build_storage_cache_key(CacheKeyParts {
                    site_id,
                    namespace: state.snapshot.full_page_cache.namespace,
                    backend_id: backend_id.index(),
                    runtime_generation: generation,
                    policy_generation: 0,
                    route_idx,
                    method: "GET".into(),
                    scheme,
                    host: normalize_host(Some(&host)),
                    path: canon_path,
                    query: canon_query,
                    content_encoding: "identity".into(),
                });
                let stats = cache.invalidate_key(&key);
                let us = started.elapsed().as_micros() as u64;
                // Cap057 LA-CAP057-003: purge.url must not report operator success when
                // zero live entries were removed (wrong scheme/host/key → no-op).
                if stats.purged_entries == 0 {
                    note_fpc_purge_rejected("not_found");
                    return CachePurgeOutcome::fail("purge.url", site_id, generation, "not_found");
                }
                note_fpc_purge_success(stats.purged_entries, stats.purged_bytes, us);
                CachePurgeOutcome::success(
                    "purge.url",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    generation,
                )
            }
        }
    }
}

impl CachePurgePort for CoreCachePurgePort {
    fn purge(&self, op: CachePurgeOp) -> CachePurgeOutcome {
        let outcome = self.purge_committed(op);
        // Cap061: AUDIT_SUCCESS only after invalidate_* returned (committed deletion stats).
        let detail = format!(
            "site_id={} purged_entries={} err={}",
            outcome.site_id,
            outcome.purged_entries,
            outcome.error.unwrap_or("")
        );
        crate::observability::emit_audit(crate::observability::AuditEvent {
            action: outcome.operation,
            result: if outcome.ok { "success" } else { "failure" },
            detail: Some(&detail),
            request_id: None,
        });
        outcome
    }
}

fn op_name(op: &CachePurgeOp) -> &'static str {
    match op {
        CachePurgeOp::Url { .. } => "purge.url",
        CachePurgeOp::Site { .. } => "purge.site",
        CachePurgeOp::Generation { .. } => "purge.generation",
        CachePurgeOp::Tag { .. } => "purge.tag",
    }
}

/// Build a purge port handle for the live shared state (tests / composition).
pub fn purge_port(shared: SharedServerState) -> Arc<dyn CachePurgePort> {
    Arc::new(CoreCachePurgePort { shared })
}

/// WC7B1 — wrap local purge with best-effort invalidation publish (fail-open).
pub fn purge_port_with_coordination(
    shared: SharedServerState,
    publisher: Option<Arc<dyn exyonq_module_api::InvalidationPublisher>>,
    node_id: impl Into<String>,
    invalidation_enabled: bool,
) -> Arc<dyn CachePurgePort> {
    let inner = purge_port(shared);
    if !invalidation_enabled || publisher.is_none() {
        return inner;
    }
    Arc::new(
        crate::coordinating_purge_port::CoordinatingCachePurgePort::new(
            inner, publisher, node_id, true,
        ),
    )
}
