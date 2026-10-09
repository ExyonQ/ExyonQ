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
//! WC3 — authenticated FPC purge control contract (separate from [`crate::kernel_control`]).

use std::path::PathBuf;
use std::sync::Arc;

/// Purge operation requested by the control plane (no secrets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachePurgeOp {
    Url {
        site_id: u64,
        scheme: String,
        host: String,
        path: String,
        query: String,
    },
    Site {
        site_id: u64,
    },
    Generation {
        site_id: u64,
        generation: u64,
    },
    /// Drop entries stored with this tag for one site.
    Tag {
        site_id: u64,
        tag: String,
    },
}

/// Bounded purge outcome — no keys, URLs, tokens, or cookies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePurgeOutcome {
    pub ok: bool,
    pub operation: &'static str,
    pub site_id: u64,
    pub purged_entries: u64,
    pub purged_bytes: u64,
    pub generation: u64,
    pub error: Option<&'static str>,
}

impl CachePurgeOutcome {
    pub fn success(
        operation: &'static str,
        site_id: u64,
        purged_entries: u64,
        purged_bytes: u64,
        generation: u64,
    ) -> Self {
        Self {
            ok: true,
            operation,
            site_id,
            purged_entries,
            purged_bytes,
            generation,
            error: None,
        }
    }

    pub fn fail(
        operation: &'static str,
        site_id: u64,
        generation: u64,
        error: &'static str,
    ) -> Self {
        Self {
            ok: false,
            operation,
            site_id,
            purged_entries: 0,
            purged_bytes: 0,
            generation,
            error: Some(error),
        }
    }
}

/// Narrow purge capability — implemented by the kernel composition root.
pub trait CachePurgePort: Send + Sync {
    fn purge(&self, op: CachePurgeOp) -> CachePurgeOutcome;
}

/// Auth material for the dedicated purge Unix socket (token never logged).
#[derive(Clone)]
pub struct CachePurgeSocketConfig {
    pub socket_path: PathBuf,
    /// Required shared secret bytes (constant-time compared). Empty = reject all.
    pub token: Arc<[u8]>,
}
