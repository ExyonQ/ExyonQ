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
//! WC7B1 — provider-neutral L2 coordination contracts (invalidation + generation).
//!
//! No Redis/Lux/RESP types. No body/lease/tag capabilities. Sync ports only
//! (ops-owned subscribe loops; never on L1 HIT path).

use crate::cache_purge::CachePurgeOp;
use std::time::{SystemTime, UNIX_EPOCH};

/// Wire / event schema version for future remote providers.
pub const COORDINATION_PROTOCOL_VERSION: u8 = 1;

/// Max `source_node_id` bytes (UTF-8).
pub const MAX_NODE_ID_BYTES: usize = 64;

/// Max bytes for a single URL field (scheme/host/path/query individually capped).
pub const MAX_URL_FIELD_BYTES: usize = 2048;

/// Default bounded subscriber queue depth.
pub const DEFAULT_MAX_PENDING_EVENTS: usize = 256;

/// Default estimated max event size for validation.
pub const DEFAULT_MAX_EVENT_BYTES: usize = 8192;

/// Default dedup ring capacity.
pub const DEFAULT_DEDUP_CAPACITY: usize = 1024;

/// Default dedup TTL (ms).
pub const DEFAULT_DEDUP_TTL_MS: u64 = 60_000;

/// v0 invalidation operations (no PURGE_TAG).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidationOperation {
    PurgeUrl,
    PurgeSite,
    PurgeGeneration,
}

impl InvalidationOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PurgeUrl => "purge.url",
            Self::PurgeSite => "purge.site",
            Self::PurgeGeneration => "purge.generation",
        }
    }
}

/// Canonical public URL identity (not raw CacheKey internals).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UrlTarget {
    pub scheme: String,
    pub host: String,
    pub path: String,
    pub query: String,
}

/// Versioned invalidation event (local typed form; wire = JSON of this schema).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InvalidationEvent {
    pub protocol_version: u8,
    pub event_id: u128,
    pub source_node_id: String,
    pub site_id: u64,
    pub operation: InvalidationOperation,
    /// Required for [`InvalidationOperation::PurgeUrl`].
    pub url: Option<UrlTarget>,
    /// FPC / site generation related to the op (purge.generation target, or
    /// publisher's observed generation for URL/site ops).
    pub generation: u64,
    pub issued_at_unix_ms: u64,
}

/// Bounded rejection / provider error reasons (metrics labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinationRejectReason {
    UnknownVersion,
    InvalidScope,
    InvalidTarget,
    OversizedEvent,
    ProviderUnavailable,
    QueueFull,
    GenerationRollback,
    InternalError,
    Duplicate,
    UnknownOperation,
}

impl CoordinationRejectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownVersion => "unknown_version",
            Self::InvalidScope => "invalid_scope",
            Self::InvalidTarget => "invalid_target",
            Self::OversizedEvent => "oversized_event",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::QueueFull => "queue_full",
            Self::GenerationRollback => "generation_rollback",
            Self::InternalError => "internal_error",
            Self::Duplicate => "duplicate",
            Self::UnknownOperation => "unknown_operation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinationError {
    pub reason: CoordinationRejectReason,
}

impl CoordinationError {
    pub fn new(reason: CoordinationRejectReason) -> Self {
        Self { reason }
    }
}

/// Publish invalidation events (control plane / after local WC3 success).
pub trait InvalidationPublisher: Send + Sync {
    fn publish(&self, event: InvalidationEvent) -> Result<(), CoordinationError>;
}

/// Pull-based subscriber (ops worker polls; never request hot path).
pub trait InvalidationSubscriber: Send + Sync {
    /// Non-blocking receive. `Ok(None)` = empty.
    fn try_recv(&self) -> Result<Option<InvalidationEvent>, CoordinationError>;
    /// Stop receiving; release resources.
    fn stop(&self);
}

/// Advisory site generation high-water store (never rolls back).
///
/// Canonical RuntimePlan / process FPC generation remains owned by reload.
/// Observed values: `max(local, remote)`.
pub trait GenerationStore: Send + Sync {
    fn get_generation(&self, site_id: u64) -> Result<u64, CoordinationError>;
    /// Monotonic advance to at least `to`. Returns the resulting generation.
    fn advance_generation(&self, site_id: u64, to: u64) -> Result<u64, CoordinationError>;
}

/// Limits / feature flags for coordination (defaults: disabled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistributedCacheCoordConfig {
    pub enabled: bool,
    pub invalidation_enabled: bool,
    pub generation_enabled: bool,
    pub max_pending_events: usize,
    pub max_event_bytes: usize,
    pub dedup_capacity: usize,
    pub dedup_ttl_ms: u64,
}

impl Default for DistributedCacheCoordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            invalidation_enabled: false,
            generation_enabled: false,
            max_pending_events: DEFAULT_MAX_PENDING_EVENTS,
            max_event_bytes: DEFAULT_MAX_EVENT_BYTES,
            dedup_capacity: DEFAULT_DEDUP_CAPACITY,
            dedup_ttl_ms: DEFAULT_DEDUP_TTL_MS,
        }
    }
}

impl DistributedCacheCoordConfig {
    pub fn coordination_active(&self) -> bool {
        self.enabled && (self.invalidation_enabled || self.generation_enabled)
    }
}

/// Validate event before publish or apply.
pub fn validate_invalidation_event(
    event: &InvalidationEvent,
    max_event_bytes: usize,
) -> Result<(), CoordinationError> {
    if event.protocol_version != COORDINATION_PROTOCOL_VERSION {
        return Err(CoordinationError::new(
            CoordinationRejectReason::UnknownVersion,
        ));
    }
    if event.site_id == 0 {
        return Err(CoordinationError::new(
            CoordinationRejectReason::InvalidScope,
        ));
    }
    if event.source_node_id.is_empty() || event.source_node_id.len() > MAX_NODE_ID_BYTES {
        return Err(CoordinationError::new(
            CoordinationRejectReason::InvalidTarget,
        ));
    }
    if event.source_node_id.bytes().any(|b| b == 0) {
        return Err(CoordinationError::new(
            CoordinationRejectReason::InvalidTarget,
        ));
    }
    match event.operation {
        InvalidationOperation::PurgeUrl => {
            let Some(url) = event.url.as_ref() else {
                return Err(CoordinationError::new(
                    CoordinationRejectReason::InvalidTarget,
                ));
            };
            validate_url_target(url)?;
        }
        InvalidationOperation::PurgeSite => {
            if event.url.is_some() {
                return Err(CoordinationError::new(
                    CoordinationRejectReason::InvalidTarget,
                ));
            }
        }
        InvalidationOperation::PurgeGeneration => {
            if event.url.is_some() {
                return Err(CoordinationError::new(
                    CoordinationRejectReason::InvalidTarget,
                ));
            }
            if event.generation == 0 {
                return Err(CoordinationError::new(
                    CoordinationRejectReason::InvalidTarget,
                ));
            }
        }
    }
    if estimate_event_bytes(event) > max_event_bytes {
        return Err(CoordinationError::new(
            CoordinationRejectReason::OversizedEvent,
        ));
    }
    Ok(())
}

fn validate_url_target(url: &UrlTarget) -> Result<(), CoordinationError> {
    for (field, name_ok) in [
        (url.scheme.as_str(), !url.scheme.is_empty()),
        (url.host.as_str(), !url.host.is_empty()),
        (url.path.as_str(), url.path.starts_with('/')),
        (url.query.as_str(), true),
    ] {
        if !name_ok || field.len() > MAX_URL_FIELD_BYTES || field.bytes().any(|b| b == 0) {
            return Err(CoordinationError::new(
                CoordinationRejectReason::InvalidTarget,
            ));
        }
    }
    if url.scheme != "http" && url.scheme != "https" {
        return Err(CoordinationError::new(
            CoordinationRejectReason::InvalidTarget,
        ));
    }
    Ok(())
}

fn estimate_event_bytes(event: &InvalidationEvent) -> usize {
    let mut n = 64 + event.source_node_id.len();
    if let Some(u) = &event.url {
        n += u.scheme.len() + u.host.len() + u.path.len() + u.query.len();
    }
    n
}

/// Map a validated event to [`CachePurgeOp`] for local L1 apply (reuse WC3 path).
pub fn invalidation_event_to_purge_op(
    event: &InvalidationEvent,
) -> Result<CachePurgeOp, CoordinationError> {
    validate_invalidation_event(event, DEFAULT_MAX_EVENT_BYTES)?;
    Ok(match event.operation {
        InvalidationOperation::PurgeUrl => {
            let url = event.url.as_ref().expect("validated");
            CachePurgeOp::Url {
                site_id: event.site_id,
                scheme: url.scheme.clone(),
                host: url.host.clone(),
                path: url.path.clone(),
                query: url.query.clone(),
            }
        }
        InvalidationOperation::PurgeSite => CachePurgeOp::Site {
            site_id: event.site_id,
        },
        InvalidationOperation::PurgeGeneration => CachePurgeOp::Generation {
            site_id: event.site_id,
            generation: event.generation,
        },
    })
}

/// Build an event from a successful local purge (publisher side).
pub fn event_from_purge_op(
    op: &CachePurgeOp,
    event_id: u128,
    source_node_id: impl Into<String>,
    generation: u64,
) -> Option<InvalidationEvent> {
    let source_node_id = source_node_id.into();
    let issued_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    match op {
        CachePurgeOp::Tag { .. } => None,
        CachePurgeOp::Url {
            site_id,
            scheme,
            host,
            path,
            query,
        } => Some(InvalidationEvent {
            protocol_version: COORDINATION_PROTOCOL_VERSION,
            event_id,
            source_node_id,
            site_id: *site_id,
            operation: InvalidationOperation::PurgeUrl,
            url: Some(UrlTarget {
                scheme: scheme.clone(),
                host: host.clone(),
                path: path.clone(),
                query: query.clone(),
            }),
            generation,
            issued_at_unix_ms,
        }),
        CachePurgeOp::Site { site_id } => Some(InvalidationEvent {
            protocol_version: COORDINATION_PROTOCOL_VERSION,
            event_id,
            source_node_id,
            site_id: *site_id,
            operation: InvalidationOperation::PurgeSite,
            url: None,
            generation,
            issued_at_unix_ms,
        }),
        CachePurgeOp::Generation {
            site_id,
            generation: gen,
        } => Some(InvalidationEvent {
            protocol_version: COORDINATION_PROTOCOL_VERSION,
            event_id,
            source_node_id,
            site_id: *site_id,
            operation: InvalidationOperation::PurgeGeneration,
            url: None,
            generation: *gen,
            issued_at_unix_ms,
        }),
    }
}

/// Wall-clock helper for tests.
pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
