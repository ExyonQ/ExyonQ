//! Redis Streams coordination provider (sync control-plane; no L1 hot path).

use crate::config::{RedisCoordConfig, RedisCoordSecrets};
use crate::health::{CoordinationHealthSnapshot, CoordinationProviderHealth, HealthCell};
use crate::metrics;
use crate::wire::{decode_signed_event, encode_signed_event};
use crate::EventSigningKeys;
use exyonq_module_api::{
    validate_invalidation_event, CoordinationError, CoordinationRejectReason, GenerationStore,
    InvalidationEvent, InvalidationOperation, InvalidationPublisher, InvalidationSubscriber,
    COORDINATION_PROTOCOL_VERSION,
};
use redis::streams::{StreamId, StreamKey, StreamMaxlen, StreamReadOptions, StreamReadReply};
use redis::{Client, Commands, Connection, RedisResult, Script};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Fixed Lua: `new = max(current, proposed)` — never rolls back.
const ADVANCE_GENERATION_LUA: &str = r#"
local key = KEYS[1]
local proposed = tonumber(ARGV[1])
if proposed == nil then
  return redis.error_reply('invalid proposed generation')
end
local current = tonumber(redis.call('GET', key) or '0')
if current == nil then current = 0 end
local next_gen = current
if proposed > current then
  next_gen = proposed
end
redis.call('SET', key, next_gen)
return next_gen
"#;

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

struct SharedState {
    cfg: RedisCoordConfig,
    keys: EventSigningKeys,
    client: Client,
    /// Command connection (publish + generation). Control plane only.
    cmd: Mutex<Option<Connection>>,
    local_generations: Mutex<HashMap<u64, u64>>,
    dedup: Mutex<DedupRing>,
    stop: AtomicBool,
    reconcile_needed: AtomicBool,
    next_event_id: AtomicU64,
    health: HealthCell,
    /// Injected into local queue on reconcile (synthetic generation events).
    local_tx: Mutex<Option<SyncSender<InvalidationEvent>>>,
}

/// Redis Streams adapter implementing WC7B1 capabilities.
#[derive(Clone)]
pub struct RedisCoordinationProvider {
    shared: Arc<SharedState>,
    node_id: String,
    subscriber: Arc<Mutex<Option<SubscriberHandle>>>,
}

struct SubscriberHandle {
    rx: Receiver<InvalidationEvent>,
    join: Option<JoinHandle<()>>,
}

impl RedisCoordinationProvider {
    /// Connect and prepare stream/group. Does not start the subscriber loop.
    ///
    /// WC7C: `keys` must be present for signed publish/subscribe (lab/production fabric).
    pub fn connect(
        cfg: RedisCoordConfig,
        secrets: RedisCoordSecrets,
        keys: EventSigningKeys,
    ) -> Result<Self, CoordinationError> {
        if cfg.endpoint.is_empty() {
            return Err(CoordinationError::new(
                CoordinationRejectReason::ProviderUnavailable,
            ));
        }
        if keys.active_key.is_empty() || keys.active_key_id.is_empty() {
            return Err(CoordinationError::new(
                CoordinationRejectReason::ProviderUnavailable,
            ));
        }
        if cfg.deployment_id.is_empty() || cfg.deployment_id.len() > 64 {
            return Err(CoordinationError::new(
                CoordinationRejectReason::InvalidTarget,
            ));
        }
        if cfg.node_id.is_empty() || cfg.node_id.len() > 64 {
            return Err(CoordinationError::new(
                CoordinationRejectReason::InvalidTarget,
            ));
        }
        // AUTH material only for Client::open — never written back to cfg.endpoint.
        let open_url = match secrets.password.as_ref() {
            Some(pw) => inject_password(&cfg.endpoint, pw),
            None => cfg.endpoint.clone(),
        };
        metrics::note_connect();
        // Connecting while opening Redis — not exposed until SharedState exists.
        let client = Client::open(open_url.as_str()).map_err(|_| {
            metrics::note_connect_fail();
            CoordinationError::new(CoordinationRejectReason::ProviderUnavailable)
        })?;
        let mut conn = client
            .get_connection_with_timeout(cfg.connect_timeout)
            .map_err(|_| {
                metrics::note_connect_fail();
                CoordinationError::new(CoordinationRejectReason::ProviderUnavailable)
            })?;
        ensure_stream_group(&mut conn, &cfg).map_err(|_| {
            metrics::note_connect_fail();
            CoordinationError::new(CoordinationRejectReason::ProviderUnavailable)
        })?;
        metrics::clear_degraded();
        let node_id = cfg.node_id.clone();
        let shared = Arc::new(SharedState {
            cfg,
            keys,
            client,
            cmd: Mutex::new(Some(conn)),
            local_generations: Mutex::new(HashMap::new()),
            dedup: Mutex::new(DedupRing::new(1024, 60_000)),
            stop: AtomicBool::new(false),
            reconcile_needed: AtomicBool::new(false),
            next_event_id: AtomicU64::new(1),
            health: HealthCell::new(CoordinationProviderHealth::Healthy),
            local_tx: Mutex::new(None),
        });
        Ok(Self {
            shared,
            node_id,
            subscriber: Arc::new(Mutex::new(None)),
        })
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Provider health only — never gates dataplane readiness.
    pub fn health_snapshot(&self) -> CoordinationHealthSnapshot {
        let reconcile = self.shared.reconcile_needed.load(Ordering::Relaxed);
        let mut state = self.shared.health.get();
        if reconcile && state == CoordinationProviderHealth::Healthy {
            state = CoordinationProviderHealth::Reconciling;
        }
        if metrics::l2_redis_provider_degraded() && state == CoordinationProviderHealth::Healthy {
            state = CoordinationProviderHealth::Degraded;
        }
        CoordinationHealthSnapshot {
            state,
            degraded: metrics::l2_redis_provider_degraded(),
            reconcile_needed: reconcile,
        }
    }

    pub fn set_health(&self, state: CoordinationProviderHealth) {
        self.shared.health.set(state);
    }

    pub fn next_event_id(&self) -> u128 {
        self.shared.next_event_id.fetch_add(1, Ordering::Relaxed) as u128
    }

    pub fn config(&self) -> &RedisCoordConfig {
        &self.shared.cfg
    }

    /// Start (or restart) Streams consumer thread.
    pub fn start_subscriber(&self) -> Result<(), CoordinationError> {
        self.stop_subscriber();
        self.shared.stop.store(false, Ordering::Relaxed);
        let (tx, rx) = sync_channel(self.shared.cfg.max_pending_events.max(1));
        *self
            .shared
            .local_tx
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))? =
            Some(tx.clone());
        let shared = Arc::clone(&self.shared);
        let join = thread::Builder::new()
            .name(format!("exyonq-redis-coord-{}", self.node_id))
            .spawn(move || subscriber_loop(shared, tx))
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
        // Reconcile after start (covers missed events while offline).
        let _ = self.reconcile_known_sites();
        *self
            .subscriber
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))? =
            Some(SubscriberHandle {
                rx,
                join: Some(join),
            });
        Ok(())
    }

    pub fn stop_subscriber(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Ok(mut tx_guard) = self.shared.local_tx.lock() {
            *tx_guard = None;
        }
        if let Ok(mut guard) = self.subscriber.lock() {
            if let Some(mut handle) = guard.take() {
                if let Some(join) = handle.join.take() {
                    let _ = join.join();
                }
            }
        }
    }

    /// Read remote generations for configured sites; advance local high-water;
    /// enqueue synthetic purge.generation when remote is newer.
    pub fn reconcile_known_sites(&self) -> Result<(), CoordinationError> {
        metrics::note_reconcile();
        let sites: Vec<u64> = self.shared.cfg.known_site_ids.clone();
        if sites.is_empty() {
            return Ok(());
        }
        // Bound: never iterate unbounded — config list is the authority.
        if sites.len() > 10_000 {
            metrics::note_reconcile_fail();
            return Err(CoordinationError::new(
                CoordinationRejectReason::InternalError,
            ));
        }
        for site_id in sites {
            match self.fetch_remote_generation(site_id) {
                Ok(remote) => {
                    let local = self
                        .shared
                        .local_generations
                        .lock()
                        .map(|g| *g.get(&site_id).unwrap_or(&0))
                        .unwrap_or(0);
                    if remote > local {
                        let _ = self.remember_local_generation(site_id, remote);
                        self.enqueue_synthetic_generation(site_id, remote);
                    }
                }
                Err(_) => {
                    metrics::note_reconcile_fail();
                    self.shared.reconcile_needed.store(true, Ordering::Relaxed);
                    return Err(CoordinationError::new(
                        CoordinationRejectReason::ProviderUnavailable,
                    ));
                }
            }
        }
        self.shared.reconcile_needed.store(false, Ordering::Relaxed);
        Ok(())
    }

    pub fn reconcile_needed(&self) -> bool {
        self.shared.reconcile_needed.load(Ordering::Relaxed)
    }

    /// Process one received event (dedupe / validate / skip self).
    pub fn accept_received(
        &self,
        event: InvalidationEvent,
        ignore_self: bool,
    ) -> Result<Option<InvalidationEvent>, CoordinationError> {
        validate_invalidation_event(&event, self.shared.cfg.max_event_bytes)?;
        if ignore_self && event.source_node_id == self.node_id {
            return Ok(None);
        }
        let dup = self
            .shared
            .dedup
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?
            .insert_if_new(event.event_id);
        if dup {
            return Ok(None);
        }
        // Observe generation from event (monotonic).
        let _ = self.remember_local_generation(event.site_id, event.generation);
        Ok(Some(event))
    }

    fn remember_local_generation(&self, site_id: u64, gen: u64) -> Result<u64, CoordinationError> {
        let mut gens = self
            .shared
            .local_generations
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
        let cur = *gens.get(&site_id).unwrap_or(&0);
        let next = cur.max(gen);
        gens.insert(site_id, next);
        Ok(next)
    }

    fn enqueue_synthetic_generation(&self, site_id: u64, generation: u64) {
        let event = InvalidationEvent {
            protocol_version: COORDINATION_PROTOCOL_VERSION,
            event_id: self.next_event_id(),
            source_node_id: "reconcile".into(),
            site_id,
            operation: InvalidationOperation::PurgeGeneration,
            url: None,
            generation,
            issued_at_unix_ms: exyonq_module_api::now_unix_ms(),
        };
        if let Ok(guard) = self.shared.local_tx.lock() {
            if let Some(tx) = guard.as_ref() {
                if tx.try_send(event).is_err() {
                    metrics::note_publish_fail();
                    self.shared.reconcile_needed.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    fn with_cmd<F, T>(&self, f: F) -> Result<T, CoordinationError>
    where
        F: FnOnce(&mut Connection) -> RedisResult<T>,
    {
        let mut guard = self
            .shared
            .cmd
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
        if guard.is_none() {
            match self
                .shared
                .client
                .get_connection_with_timeout(self.shared.cfg.connect_timeout)
            {
                Ok(c) => *guard = Some(c),
                Err(_) => {
                    metrics::note_connect_fail();
                    return Err(CoordinationError::new(
                        CoordinationRejectReason::ProviderUnavailable,
                    ));
                }
            }
        }
        let conn = guard.as_mut().expect("conn present");
        // Best-effort command timeout via redis connection read/write timeout.
        let _ = conn.set_read_timeout(Some(self.shared.cfg.command_timeout));
        let _ = conn.set_write_timeout(Some(self.shared.cfg.command_timeout));
        match f(conn) {
            Ok(v) => Ok(v),
            Err(e) => {
                if e.is_timeout() {
                    metrics::note_cmd_timeout();
                }
                *guard = None;
                metrics::note_publish_fail();
                Err(CoordinationError::new(
                    CoordinationRejectReason::ProviderUnavailable,
                ))
            }
        }
    }

    fn fetch_remote_generation(&self, site_id: u64) -> Result<u64, CoordinationError> {
        let key = self.shared.cfg.generation_key(site_id);
        self.with_cmd(|conn| {
            let v: Option<u64> = conn.get(key)?;
            Ok(v.unwrap_or(0))
        })
    }
}

impl InvalidationPublisher for RedisCoordinationProvider {
    fn publish(&self, event: InvalidationEvent) -> Result<(), CoordinationError> {
        if !self.shared.cfg.invalidation_enabled {
            return Ok(());
        }
        if let Err(e) = validate_invalidation_event(&event, self.shared.cfg.max_event_bytes) {
            metrics::note_publish_fail();
            return Err(e);
        }
        let payload = encode_signed_event(
            &event,
            &self.shared.cfg.deployment_id,
            &self.shared.keys,
            self.shared.cfg.max_event_bytes,
        )
        .inspect_err(|_| {
            metrics::note_publish_fail();
        })?;
        let stream = self.shared.cfg.stream_key();
        let maxlen = StreamMaxlen::Approx(self.shared.cfg.stream_maxlen);
        let result = self.with_cmd(|conn| {
            conn.xadd_maxlen::<_, _, _, _, String>(stream.as_str(), maxlen, "*", &[("e", payload)])
        });
        match result {
            Ok(_) => {
                metrics::note_publish();
                Ok(())
            }
            Err(e) => {
                metrics::note_publish_fail();
                Err(e)
            }
        }
    }
}

impl InvalidationSubscriber for RedisCoordinationProvider {
    fn try_recv(&self) -> Result<Option<InvalidationEvent>, CoordinationError> {
        if self.shared.reconcile_needed.load(Ordering::Relaxed) {
            let _ = self.reconcile_known_sites();
        }
        let guard = self
            .subscriber
            .lock()
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
        let Some(state) = guard.as_ref() else {
            return Ok(None);
        };
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

impl GenerationStore for RedisCoordinationProvider {
    fn get_generation(&self, site_id: u64) -> Result<u64, CoordinationError> {
        if !self.shared.cfg.generation_enabled {
            return Ok(0);
        }
        // Local high-water only — never remote I/O on this path.
        // Remote fetch is reconcile/advance only (composition must not call
        // fetch_remote_generation / reconcile from L1 HIT).
        Ok(self
            .shared
            .local_generations
            .lock()
            .map(|g| *g.get(&site_id).unwrap_or(&0))
            .unwrap_or(0))
    }

    fn advance_generation(&self, site_id: u64, to: u64) -> Result<u64, CoordinationError> {
        if !self.shared.cfg.generation_enabled {
            return Ok(to);
        }
        if site_id == 0 {
            return Err(CoordinationError::new(
                CoordinationRejectReason::InvalidScope,
            ));
        }
        let local = self
            .shared
            .local_generations
            .lock()
            .map(|g| *g.get(&site_id).unwrap_or(&0))
            .unwrap_or(0);
        if to < local {
            return Err(CoordinationError::new(
                CoordinationRejectReason::GenerationRollback,
            ));
        }
        let key = self.shared.cfg.generation_key(site_id);
        let script = Script::new(ADVANCE_GENERATION_LUA);
        match self.with_cmd(|conn| script.key(key.as_str()).arg(to).invoke(conn)) {
            Ok(remote) => {
                let next = local.max(remote);
                let _ = self.remember_local_generation(site_id, next);
                Ok(next)
            }
            Err(_) => {
                // Fail-open: keep local high-water; do not roll back.
                // Local advance is OK for L1; peers reconcile when Redis returns.
                metrics::note_publish_fail();
                let next = local.max(to);
                let _ = self.remember_local_generation(site_id, next);
                Ok(next)
            }
        }
    }
}

fn inject_password(endpoint: &str, password: &str) -> String {
    // redis://[:password@]host:port/db — never log result.
    // Reject endpoints that already embed userinfo (IR should also reject).
    if endpoint.contains('@') {
        return endpoint.to_string();
    }
    let enc: String = password
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    if let Some(rest) = endpoint.strip_prefix("redis://") {
        return format!("redis://:{enc}@{rest}");
    }
    if let Some(rest) = endpoint.strip_prefix("rediss://") {
        return format!("rediss://:{enc}@{rest}");
    }
    endpoint.to_string()
}

fn ensure_stream_group(conn: &mut Connection, cfg: &RedisCoordConfig) -> RedisResult<()> {
    let stream = cfg.stream_key();
    let group = format!("{}:{}", cfg.consumer_group(), cfg.node_id);
    let result: RedisResult<String> = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(&stream)
        .arg(&group)
        .arg("0")
        .arg("MKSTREAM")
        .query(conn);
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("BUSYGROUP") {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

fn consumer_group_name(cfg: &RedisCoordConfig) -> String {
    format!("{}:{}", cfg.consumer_group(), cfg.node_id)
}

fn subscriber_loop(shared: Arc<SharedState>, tx: SyncSender<InvalidationEvent>) {
    let mut backoff = shared.cfg.reconnect_min_backoff;
    while !shared.stop.load(Ordering::Relaxed) {
        shared.health.set(CoordinationProviderHealth::Connecting);
        match shared
            .client
            .get_connection_with_timeout(shared.cfg.connect_timeout)
        {
            Ok(mut conn) => {
                metrics::clear_degraded();
                shared.health.set(CoordinationProviderHealth::Healthy);
                let _ = ensure_stream_group(&mut conn, &shared.cfg);
                // Claim pending then read new.
                if run_read_loop(&shared, &mut conn, &tx).is_err() {
                    metrics::note_reconnect();
                    shared.reconcile_needed.store(true, Ordering::Relaxed);
                    shared.health.set(CoordinationProviderHealth::Degraded);
                } else {
                    break; // clean stop
                }
            }
            Err(_) => {
                metrics::note_connect_fail();
                metrics::note_reconnect();
                shared.reconcile_needed.store(true, Ordering::Relaxed);
                shared.health.set(CoordinationProviderHealth::Degraded);
            }
        }
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }
        thread::sleep(backoff);
        backoff = (backoff * 2).min(shared.cfg.reconnect_max_backoff);
    }
}

fn run_read_loop(
    shared: &SharedState,
    conn: &mut Connection,
    tx: &SyncSender<InvalidationEvent>,
) -> Result<(), ()> {
    let stream = shared.cfg.stream_key();
    let group = consumer_group_name(&shared.cfg);
    let consumer = shared.cfg.node_id.clone();
    let _ = conn.set_read_timeout(Some(Duration::from_millis(1500)));
    let _ = conn.set_write_timeout(Some(shared.cfg.command_timeout));

    // Recover pending (crash after apply before ACK → harmless redelivery).
    let _ = claim_pending(shared, conn, &stream, &group, &consumer, tx);

    while !shared.stop.load(Ordering::Relaxed) {
        let opts = StreamReadOptions::default()
            .group(&group, &consumer)
            .count(16)
            .block(1000);
        let reply: RedisResult<StreamReadReply> = conn.xread_options(&[&stream], &[">"], &opts);
        match reply {
            Ok(reply) => {
                for key in reply.keys {
                    if deliver_stream_key(shared, conn, &stream, &group, &key, tx).is_err() {
                        return Err(());
                    }
                }
            }
            Err(e) => {
                if e.is_timeout() {
                    continue;
                }
                // Empty reply sometimes surfaces as nil — treat as ok.
                let msg = e.to_string().to_lowercase();
                if msg.contains("nil") || msg.contains("empty") {
                    continue;
                }
                return Err(());
            }
        }
    }
    Ok(())
}

fn claim_pending(
    shared: &SharedState,
    conn: &mut Connection,
    stream: &str,
    group: &str,
    consumer: &str,
    tx: &SyncSender<InvalidationEvent>,
) -> Result<(), ()> {
    // XAUTOCLAIM stream group consumer 60000 0-0 COUNT 32
    let result: RedisResult<redis::Value> = redis::cmd("XAUTOCLAIM")
        .arg(stream)
        .arg(group)
        .arg(consumer)
        .arg(60_000u64)
        .arg("0-0")
        .arg("COUNT")
        .arg(32)
        .query(conn);
    let Ok(value) = result else {
        return Ok(());
    };
    // Reply: [next_id, [entries...], [deleted_ids]]
    if let redis::Value::Array(items) = value {
        if items.len() >= 2 {
            if let redis::Value::Array(entries) = &items[1] {
                for entry in entries {
                    if let Ok(id) = parse_and_deliver_entry(shared, conn, stream, group, entry, tx)
                    {
                        let _ = id;
                    }
                }
            }
        }
    }
    Ok(())
}

fn deliver_stream_key(
    shared: &SharedState,
    conn: &mut Connection,
    stream: &str,
    group: &str,
    key: &StreamKey,
    tx: &SyncSender<InvalidationEvent>,
) -> Result<(), ()> {
    for id in &key.ids {
        deliver_stream_id(shared, conn, stream, group, id, tx)?;
    }
    Ok(())
}

fn value_as_str(v: &redis::Value) -> Option<&str> {
    match v {
        redis::Value::BulkString(b) => std::str::from_utf8(b).ok(),
        redis::Value::SimpleString(s) => Some(s.as_str()),
        _ => None,
    }
}

fn deliver_stream_id(
    shared: &SharedState,
    conn: &mut Connection,
    stream: &str,
    group: &str,
    id: &StreamId,
    tx: &SyncSender<InvalidationEvent>,
) -> Result<(), ()> {
    let payload = id.map.get("e").and_then(value_as_str).unwrap_or("");
    match decode_signed_event(
        payload,
        &shared.cfg.deployment_id,
        &shared.keys,
        &shared.cfg.replay,
        shared.cfg.max_event_bytes,
    ) {
        Ok(event) => match tx.try_send(event) {
            Ok(()) => {
                metrics::note_receive();
                if conn
                    .xack::<_, _, _, u64>(stream, group, &[id.id.as_str()])
                    .is_err()
                {
                    metrics::note_ack_fail();
                }
            }
            Err(_) => {
                // Overflow: do not ACK → redelivery; mark reconcile.
                shared.reconcile_needed.store(true, Ordering::Relaxed);
                metrics::note_publish_fail();
                return Err(());
            }
        },
        Err(_e) => {
            // Invalid/HMAC/replay: ACK to avoid poison loop.
            // Auth/replay counters are incremented once inside `decode_signed_event`.
            metrics::note_ack_fail();
            let _ = conn.xack::<_, _, _, u64>(stream, group, &[id.id.as_str()]);
        }
    }
    Ok(())
}

fn parse_and_deliver_entry(
    shared: &SharedState,
    conn: &mut Connection,
    stream: &str,
    group: &str,
    entry: &redis::Value,
    tx: &SyncSender<InvalidationEvent>,
) -> Result<String, ()> {
    // entry = [id, [field, value, ...]]
    let redis::Value::Array(parts) = entry else {
        return Err(());
    };
    if parts.len() < 2 {
        return Err(());
    }
    let id = match &parts[0] {
        redis::Value::BulkString(b) => String::from_utf8_lossy(b).into_owned(),
        redis::Value::SimpleString(s) => s.clone(),
        _ => return Err(()),
    };
    let mut payload = String::new();
    if let redis::Value::Array(fields) = &parts[1] {
        let mut i = 0;
        while i + 1 < fields.len() {
            let field = match &fields[i] {
                redis::Value::BulkString(b) => String::from_utf8_lossy(b).into_owned(),
                redis::Value::SimpleString(s) => s.clone(),
                _ => String::new(),
            };
            let value = match &fields[i + 1] {
                redis::Value::BulkString(b) => String::from_utf8_lossy(b).into_owned(),
                redis::Value::SimpleString(s) => s.clone(),
                _ => String::new(),
            };
            if field == "e" {
                payload = value;
            }
            i += 2;
        }
    }
    match decode_signed_event(
        &payload,
        &shared.cfg.deployment_id,
        &shared.keys,
        &shared.cfg.replay,
        shared.cfg.max_event_bytes,
    ) {
        Ok(event) => {
            let _ = tx.try_send(event);
            let _ = conn.xack::<_, _, _, u64>(stream, group, &[id.as_str()]);
        }
        Err(_) => {
            // Counters owned by `decode_signed_event` (single increment per reject).
            let _ = conn.xack::<_, _, _, u64>(stream, group, &[id.as_str()]);
        }
    }
    Ok(id)
}
