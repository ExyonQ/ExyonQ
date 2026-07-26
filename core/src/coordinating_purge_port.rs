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
//! WC7B1 — best-effort invalidation publish after local WC3 purge success.

use exyonq_module_api::cache_purge::{CachePurgeOp, CachePurgeOutcome, CachePurgePort};
use exyonq_module_api::{event_from_purge_op, InvalidationPublisher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Wraps a local [`CachePurgePort`]: local purge is authoritative; publish is best-effort.
pub struct CoordinatingCachePurgePort {
    pub inner: Arc<dyn CachePurgePort>,
    pub publisher: Option<Arc<dyn InvalidationPublisher>>,
    pub node_id: String,
    pub invalidation_enabled: bool,
    event_seq: AtomicU64,
}

impl CoordinatingCachePurgePort {
    pub fn new(
        inner: Arc<dyn CachePurgePort>,
        publisher: Option<Arc<dyn InvalidationPublisher>>,
        node_id: impl Into<String>,
        invalidation_enabled: bool,
    ) -> Self {
        Self {
            inner,
            publisher,
            node_id: node_id.into(),
            invalidation_enabled,
            event_seq: AtomicU64::new(1),
        }
    }

    fn next_id(&self) -> u128 {
        self.event_seq.fetch_add(1, Ordering::Relaxed) as u128
    }
}

impl CachePurgePort for CoordinatingCachePurgePort {
    fn purge(&self, op: CachePurgeOp) -> CachePurgeOutcome {
        let outcome = self.inner.purge(op.clone());
        if outcome.ok && self.invalidation_enabled {
            if let Some(pub_) = self.publisher.as_ref() {
                if let Some(event) = event_from_purge_op(
                    &op,
                    self.next_id(),
                    self.node_id.clone(),
                    outcome.generation,
                ) {
                    // Best-effort: never change local purge outcome.
                    let _ = pub_.publish(event);
                }
            }
        }
        outcome
    }
}
