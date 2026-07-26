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
//! WC7B1 — in-memory local coordination provider (no network / Redis / Lux).

use exyonq_module_api::{
    validate_invalidation_event, CoordinationError, CoordinationRejectReason,
    DistributedCacheCoordConfig, GenerationStore, InvalidationEvent, InvalidationPublisher,
    InvalidationSubscriber, COORDINATION_PROTOCOL_VERSION,
};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::metrics::{
    note_l2_generation_advance, note_l2_generation_rollback_rejected, note_l2_invalidation_duplicate,
    note_l2_invalidation_publish, note_l2_invalidation_publish_failure, note_l2_invalidation_receive,
    note_l2_invalidation_rejected, note_l2_subscriber_overflow, note_l2_subscriber_restart,
};

/// Shared in-memory hub used by one or more local node handles.
pub struct LocalCoordinationHub {
    inner: Arc<HubInner>,
}

struct HubInner {
    cfg: DistributedCacheCoordConfig,
    subscribers: Mutex<Vec<SubscriberSlot>>,
    generations: Mutex<HashMap<u64, u64>>,
    next_event_id: AtomicU64,
    publish_fail: AtomicBool,
    /// When set, next publish returns QueueFull without enqueue.
    force_queue_full: AtomicBool,
}

struct SubscriberSlot {
    id: u64,
    node_id: String,
    tx: SyncSender<InvalidationEvent>,
    stopped: Arc<AtomicBool>,
}

impl LocalCoordinationHub {
    pub fn new(cfg: DistributedCacheCoordConfig) -> Self {
        Self {
            inner: Arc::new(HubInner {
                cfg,
                subscribers: Mutex::new(Vec::new()),
                generations: Mutex::new(HashMap::new()),
                next_event_id: AtomicU64::new(1),
                publish_fail: AtomicBool::new(false),
                force_queue_full: AtomicBool::new(false),
            }),
        }
    }

    pub fn config(&self) -> &DistributedCacheCoordConfig {
        &self.inner.cfg
    }

    /// Open a node-scoped handle (publisher + optional subscriber + generation).
    pub fn open_node(&self, node_id: impl Into<String>) -> LocalCoordinationProvider {
        LocalCoordinationProvider::attach(self.inner.clone(), node_id.into())
    }

    pub fn set_publish_unavailable(&self, unavailable: bool) {
        self.inner
            .publish_fail
            .store(unavailable, Ordering::Relaxed);
    }

    pub fn set_force_queue_full(&self, full: bool) {
        self.inner
            .force_queue_full
            .store(full, Ordering::Relaxed);
    }

    pub fn next_event_id(&self) -> u128 {
        self.inner.next_event_id.fetch_add(1, Ordering::Relaxed) as u128
    }
}

/// Per-node local provider implementing the three WC7B1 capabilities.
pub struct LocalCoordinationProvider {
    hub: Arc<HubInner>,
    node_id: String,
    subscriber: Mutex<Option<LocalSubscriberState>>,
    dedup: Mutex<DedupRing>,
    /// Sites marked uncertain after overflow (reconcile by advancing generation).
    uncertain_sites: Mutex<HashMap<u64, ()>>,
    next_sub_id: AtomicU64,
}

struct LocalSubscriberState {
    id: u64,
    rx: Receiver<InvalidationEvent>,
    stopped: Arc<AtomicBool>,
}

struct DedupRing {
    capacity: usize,
    ttl: Duration,
    entries: VecDeque<(u128, Instant)>,
    ids: HashMap<u128, Instant>,
}

impl DedupRing {
    fn new(capacity: usize, ttl_ms: u64) -> Self {
        Self {
            capacity: capacity.max(1),
            ttl: Duration::from_millis(ttl_ms.max(1)),
            entries: VecDeque::new(),
            ids: HashMap::new(),
        }
    }

    fn purge_expired(&mut self, now: Instant) {
        while let Some((id, t)) = self.entries.front().copied() {
            if now.duration_since(t) <= self.ttl {
                break;
            }
            self.entries.pop_front();
            self.ids.remove(&id);
        }
    }

    /// Returns true if this id was already seen (duplicate).
    fn insert_if_new(&mut self, id: u128) -> bool {
        let now = Instant::now();
        self.purge_expired(now);
        if self.ids.contains_key(&id) {
            return true;
        }
        while self.entries.len() >= self.capacity {
            if let Some((old, _)) = self.entries.pop_front() {
                self.ids.remove(&old);
            }
        }
        self.entries.push_back((id, now));
        self.ids.insert(id, now);
        false
    }
}

impl LocalCoordinationProvider {
    fn attach(hub: Arc<HubInner>, node_id: String) -> Self {
        let dedup = DedupRing::new(hub.cfg.dedup_capacity, hub.cfg.dedup_ttl_ms);
        Self {
            hub,
            node_id,
            subscriber: Mutex::new(None),
            dedup: Mutex::new(dedup),
            uncertain_sites: Mutex::new(HashMap::new()),
            next_sub_id: AtomicU64::new(1),
        }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn next_event_id(&self) -> u128 {
        self.hub.next_event_id.fetch_add(1, Ordering::Relaxed) as u128
    }

    /// Start (or restart) a bounded subscriber for this node.
    pub fn start_subscriber(&self) -> Result<(), CoordinationError> {
        self.stop_subscriber();
        note_l2_subscriber_restart();
        let (tx, rx) = sync_channel(self.hub.cfg.max_pending_events.max(1));
        let stopped = Arc::new(AtomicBool::new(false));
        let id = self.next_sub_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut subs = self
                .hub
                .subscribers
                .lock()
                .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
            subs.push(SubscriberSlot {
                id,
                node_id: self.node_id.clone(),
                tx,
                stopped: Arc::clone(&stopped),
            });
        }
        *self
            .subscriber
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))? =
            Some(LocalSubscriberState { id, rx, stopped });
        Ok(())
    }

    pub fn stop_subscriber(&self) {
        if let Ok(mut guard) = self.subscriber.lock() {
            if let Some(state) = guard.take() {
                state.stopped.store(true, Ordering::Relaxed);
                if let Ok(mut subs) = self.hub.subscribers.lock() {
                    subs.retain(|s| s.id != state.id);
                }
            }
        }
    }

    /// Test helper: inject event into this node's queue only.
    pub fn inject_for_tests(&self, event: InvalidationEvent) -> Result<(), CoordinationError> {
        let guard = self
            .subscriber
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
        let Some(_state) = guard.as_ref() else {
            return Err(CoordinationError::new(CoordinationRejectReason::InternalError));
        };
        // Re-publish via hub targeting only this node by temporary fanout — use direct channel
        // through hub publish filtered: instead send via a one-shot re-queue using publish
        // with force. Simplest: lock hub and send on matching slot.
        drop(guard);
        let subs = self
            .hub
            .subscribers
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
        for slot in subs.iter() {
            if slot.node_id == self.node_id && !slot.stopped.load(Ordering::Relaxed) {
                match slot.tx.try_send(event.clone()) {
                    Ok(()) => return Ok(()),
                    Err(_) => {
                        note_l2_subscriber_overflow();
                        note_l2_invalidation_rejected(
                            CoordinationRejectReason::QueueFull.as_str(),
                        );
                        self.mark_uncertain(event.site_id);
                        return Err(CoordinationError::new(CoordinationRejectReason::QueueFull));
                    }
                }
            }
        }
        Err(CoordinationError::new(CoordinationRejectReason::InternalError))
    }

    fn mark_uncertain(&self, site_id: u64) {
        if let Ok(mut m) = self.uncertain_sites.lock() {
            m.insert(site_id, ());
        }
        // Safe fallback: bump generation so stale L1 cannot linger without notice.
        let _ = self.advance_generation(site_id, self.get_generation(site_id).unwrap_or(0).saturating_add(1));
    }

    pub fn take_uncertain_sites(&self) -> Vec<u64> {
        self.uncertain_sites
            .lock()
            .map(|mut m| m.drain().map(|(k, _)| k).collect())
            .unwrap_or_default()
    }

    /// Process one received event: dedupe, validate, optionally skip self, return apply-ready event.
    pub fn accept_received(
        &self,
        event: InvalidationEvent,
        ignore_self: bool,
    ) -> Result<Option<InvalidationEvent>, CoordinationError> {
        note_l2_invalidation_receive();
        if let Err(e) = validate_invalidation_event(&event, self.hub.cfg.max_event_bytes) {
            note_l2_invalidation_rejected(e.reason.as_str());
            return Err(e);
        }
        if ignore_self && event.source_node_id == self.node_id {
            return Ok(None);
        }
        let dup = self
            .dedup
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?
            .insert_if_new(event.event_id);
        if dup {
            note_l2_invalidation_duplicate();
            return Ok(None);
        }
        if event.protocol_version != COORDINATION_PROTOCOL_VERSION {
            note_l2_invalidation_rejected(
                CoordinationRejectReason::UnknownVersion.as_str(),
            );
            return Err(CoordinationError::new(CoordinationRejectReason::UnknownVersion));
        }
        Ok(Some(event))
    }
}

impl Drop for LocalCoordinationProvider {
    fn drop(&mut self) {
        self.stop_subscriber();
    }
}

impl InvalidationPublisher for LocalCoordinationProvider {
    fn publish(&self, event: InvalidationEvent) -> Result<(), CoordinationError> {
        if !self.hub.cfg.enabled || !self.hub.cfg.invalidation_enabled {
            return Ok(());
        }
        if self.hub.publish_fail.load(Ordering::Relaxed) {
            note_l2_invalidation_publish_failure(
                CoordinationRejectReason::ProviderUnavailable.as_str(),
            );
            return Err(CoordinationError::new(
                CoordinationRejectReason::ProviderUnavailable,
            ));
        }
        if let Err(e) = validate_invalidation_event(&event, self.hub.cfg.max_event_bytes) {
            note_l2_invalidation_publish_failure(e.reason.as_str());
            return Err(e);
        }
        if self.hub.force_queue_full.load(Ordering::Relaxed) {
            note_l2_subscriber_overflow();
            note_l2_invalidation_publish_failure(CoordinationRejectReason::QueueFull.as_str());
            return Err(CoordinationError::new(CoordinationRejectReason::QueueFull));
        }

        note_l2_invalidation_publish();
        let mut overflow_sites = Vec::new();
        {
            let mut subs = self
                .hub
                .subscribers
                .lock()
                .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
            subs.retain(|s| !s.stopped.load(Ordering::Relaxed));
            for slot in subs.iter() {
                match slot.tx.try_send(event.clone()) {
                    Ok(()) => {}
                    Err(_) => {
                        note_l2_subscriber_overflow();
                        overflow_sites.push(event.site_id);
                    }
                }
            }
        }
        for site in overflow_sites {
            self.mark_uncertain(site);
            note_l2_invalidation_publish_failure(CoordinationRejectReason::QueueFull.as_str());
            return Err(CoordinationError::new(CoordinationRejectReason::QueueFull));
        }
        Ok(())
    }
}

impl InvalidationSubscriber for LocalCoordinationProvider {
    fn try_recv(&self) -> Result<Option<InvalidationEvent>, CoordinationError> {
        let guard = self
            .subscriber
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
        let Some(state) = guard.as_ref() else {
            return Ok(None);
        };
        if state.stopped.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match state.rx.try_recv() {
            Ok(ev) => Ok(Some(ev)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn stop(&self) {
        self.stop_subscriber();
    }
}

impl GenerationStore for LocalCoordinationProvider {
    fn get_generation(&self, site_id: u64) -> Result<u64, CoordinationError> {
        if !self.hub.cfg.enabled || !self.hub.cfg.generation_enabled {
            return Ok(0);
        }
        let gens = self
            .hub
            .generations
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
        Ok(*gens.get(&site_id).unwrap_or(&0))
    }

    fn advance_generation(&self, site_id: u64, to: u64) -> Result<u64, CoordinationError> {
        if !self.hub.cfg.enabled || !self.hub.cfg.generation_enabled {
            return Ok(to);
        }
        if site_id == 0 {
            return Err(CoordinationError::new(CoordinationRejectReason::InvalidScope));
        }
        let mut gens = self
            .hub
            .generations
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
        let cur = *gens.get(&site_id).unwrap_or(&0);
        if to < cur {
            note_l2_generation_rollback_rejected();
            return Err(CoordinationError::new(
                CoordinationRejectReason::GenerationRollback,
            ));
        }
        let next = cur.max(to);
        gens.insert(site_id, next);
        if next > cur {
            note_l2_generation_advance();
        }
        Ok(next)
    }
}

/// Blocking recv with timeout for tests / lab workers.
pub fn recv_timeout(
    sub: &LocalCoordinationProvider,
    timeout: Duration,
) -> Result<Option<InvalidationEvent>, CoordinationError> {
    let guard = sub
        .subscriber
        .lock()
        .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
    let Some(state) = guard.as_ref() else {
        return Ok(None);
    };
    match state.rx.recv_timeout(timeout) {
        Ok(ev) => Ok(Some(ev)),
        Err(RecvTimeoutError::Timeout) => Ok(None),
        Err(RecvTimeoutError::Disconnected) => Ok(None),
    }
}
