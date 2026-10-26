use crate::metrics::{note_eviction, note_expired, note_hit, note_insertion, note_miss};
use crate::namespace::{CacheNamespace, HitValidation, HitValidator, NamespaceMetrics};
use bytes::Bytes;
use exyonq_module_api::static_dispatch::StaticResourceIdentitySnapshot;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::key::CacheKey;

static ENTRY_ID_SEQ: AtomicU64 = AtomicU64::new(1);

/// Cached GET representation (clone via [`Arc`] without holding store locks).
#[derive(Debug)]
pub struct CachedEntry {
    pub(crate) entry_id: u64,
    pub(crate) expires_at: Instant,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    object_bytes: usize,
    pub(crate) static_identity: Option<StaticResourceIdentitySnapshot>,
    pub namespace: CacheNamespace,
    site_id: u64,
    route_idx: usize,
    runtime_generation: u64,
    invalidation_tags: Arc<[String]>,
}

impl CachedEntry {
    #[doc(hidden)]
    pub fn entry_id(&self) -> u64 {
        self.entry_id
    }

    pub fn namespace(&self) -> CacheNamespace {
        self.namespace
    }

    pub fn static_identity(&self) -> Option<&StaticResourceIdentitySnapshot> {
        self.static_identity.as_ref()
    }
}

struct ResponseCacheInner {
    entries: HashMap<CacheKey, Arc<CachedEntry>>,
    tag_index: HashMap<String, Vec<CacheKey>>,
    route_generation_index: HashMap<(usize, u64), Vec<CacheKey>>,
    site_bytes: HashMap<u64, usize>,
    fifo: VecDeque<CacheKey>,
    total_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStoreSnapshot {
    pub entries: usize,
    pub bytes: usize,
}

/// Bounded purge outcome (WC3) — no keys/URLs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PurgeStats {
    pub purged_entries: u64,
    pub purged_bytes: u64,
}

pub struct ResponseCache {
    max_entries: usize,
    max_total_bytes: usize,
    /// When set, each `site_id` may hold at most this many stored object bytes.
    max_bytes_per_site: Option<usize>,
    /// NG-16: how long an expired entry is kept for `stale-if-error`.
    /// Zero (default) removes expired entries on lookup, as before.
    stale_retention: Duration,
    inner: RwLock<ResponseCacheInner>,
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseCache {
    pub fn new() -> Self {
        Self::with_limits(
            exyonq_module_api::CACHE_MAX_ENTRIES,
            exyonq_module_api::CACHE_MAX_TOTAL_BYTES,
        )
    }

    pub fn with_limits(max_entries: usize, max_total_bytes: usize) -> Self {
        Self::with_limits_and_site_cap(max_entries, max_total_bytes, None)
    }

    pub fn with_limits_and_site_cap(
        max_entries: usize,
        max_total_bytes: usize,
        max_bytes_per_site: Option<usize>,
    ) -> Self {
        Self {
            max_entries,
            max_total_bytes,
            max_bytes_per_site,
            stale_retention: Duration::ZERO,
            inner: RwLock::new(ResponseCacheInner {
                entries: HashMap::new(),
                tag_index: HashMap::new(),
                route_generation_index: HashMap::new(),
                site_bytes: HashMap::new(),
                fifo: VecDeque::new(),
                total_bytes: 0,
            }),
        }
    }

    /// NG-16: keep expired entries this long so `stale-if-error` can serve them.
    pub fn with_stale_retention(mut self, retention: Duration) -> Self {
        self.stale_retention = retention;
        self
    }

    /// NG-16: an expired entry still inside the retention window, or `None`.
    /// Does not count a hit/miss (the caller decides what the request was).
    pub fn lookup_stale(&self, key: &CacheKey) -> Option<Arc<CachedEntry>> {
        if self.stale_retention.is_zero() {
            return None;
        }
        let entry = {
            let inner = self.inner.read().expect("cache read");
            inner.entries.get(key).map(Arc::clone)
        }?;
        let now = Instant::now();
        if entry.expires_at <= now && entry.expires_at + self.stale_retention > now {
            Some(entry)
        } else {
            None
        }
    }

    pub fn lookup(&self, key: &CacheKey) -> Option<Arc<CachedEntry>> {
        self.lookup_with_hooks(key, true, NamespaceMetrics::NONE, None)
    }

    pub(crate) fn lookup_with_hooks(
        &self,
        key: &CacheKey,
        count_miss_on_failure: bool,
        metrics: NamespaceMetrics,
        validate_hit: Option<HitValidator>,
    ) -> Option<Arc<CachedEntry>> {
        match self.lookup_raw(key) {
            LookupOutcome::Miss => {
                if count_miss_on_failure {
                    note_miss();
                    (metrics.on_miss)();
                }
                None
            }
            LookupOutcome::Stale => {
                // NG-16: still inside the retention window — a miss for this
                // request, but the entry stays for `stale-if-error`.
                if count_miss_on_failure {
                    note_miss();
                    (metrics.on_miss)();
                }
                note_expired();
                None
            }
            LookupOutcome::Expired(entry_id) => {
                if count_miss_on_failure {
                    note_miss();
                    (metrics.on_miss)();
                }
                note_expired();
                self.try_remove_if_same(key, entry_id);
                None
            }
            LookupOutcome::Hit(entry) => {
                self.validate_hit(key, entry, count_miss_on_failure, metrics, validate_hit)
            }
        }
    }

    fn validate_hit(
        &self,
        key: &CacheKey,
        entry: Arc<CachedEntry>,
        count_miss_on_failure: bool,
        metrics: NamespaceMetrics,
        validate_hit: Option<HitValidator>,
    ) -> Option<Arc<CachedEntry>> {
        if let Some(validate) = validate_hit {
            match validate(&entry) {
                HitValidation::Accept => {}
                HitValidation::Reject { entry_id } => {
                    if count_miss_on_failure {
                        note_miss();
                        (metrics.on_miss)();
                    }
                    self.try_remove_if_same(key, entry_id);
                    return None;
                }
            }
        }
        note_hit();
        (metrics.on_hit)();
        Some(entry)
    }

    pub(crate) fn get_if_alive(
        &self,
        key: &CacheKey,
        metrics: NamespaceMetrics,
        validate_hit: Option<HitValidator>,
    ) -> Option<Arc<CachedEntry>> {
        self.lookup_with_hooks(key, false, metrics, validate_hit)
    }

    fn lookup_raw(&self, key: &CacheKey) -> LookupOutcome {
        let entry = {
            let inner = self.inner.read().expect("cache read");
            inner.entries.get(key).map(Arc::clone)
        };
        let Some(entry) = entry else {
            return LookupOutcome::Miss;
        };
        if entry.expires_at <= Instant::now() {
            // NG-16: retain inside the stale window; drop otherwise.
            if !self.stale_retention.is_zero()
                && entry.expires_at + self.stale_retention > Instant::now()
            {
                return LookupOutcome::Stale;
            }
            return LookupOutcome::Expired(entry.entry_id);
        }
        LookupOutcome::Hit(entry)
    }

    pub(crate) fn lookup_after_singleflight(
        &self,
        key: &CacheKey,
        metrics: NamespaceMetrics,
        validate_hit: Option<HitValidator>,
    ) -> Option<Arc<CachedEntry>> {
        self.lookup_with_hooks(key, false, metrics, validate_hit)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_entry(
        &self,
        key: CacheKey,
        site_id: u64,
        route_idx: usize,
        runtime_generation: u64,
        ttl: Duration,
        status: u16,
        headers: Vec<(String, String)>,
        body: Bytes,
        static_identity: Option<StaticResourceIdentitySnapshot>,
        namespace: CacheNamespace,
        invalidation_tags: Arc<[String]>,
        metrics: NamespaceMetrics,
    ) {
        let object_bytes = body.len()
            + headers
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>();

        let entry = Arc::new(CachedEntry {
            entry_id: ENTRY_ID_SEQ.fetch_add(1, Ordering::Relaxed),
            expires_at: Instant::now() + ttl,
            status,
            headers,
            body,
            object_bytes,
            static_identity,
            namespace,
            site_id,
            route_idx,
            runtime_generation,
            invalidation_tags,
        });

        let mut inner = self.inner.write().expect("cache write");
        if inner.entries.contains_key(&key) {
            detach_entry(&mut inner, &key);
        }

        evict_for_insert(
            &mut inner,
            object_bytes,
            site_id,
            self.max_entries,
            self.max_total_bytes,
            self.max_bytes_per_site,
            self.stale_retention,
        );

        index_entry(&mut inner, &key, &entry);
        inner.entries.insert(key.clone(), Arc::clone(&entry));
        inner.fifo.push_back(key);
        inner.total_bytes += object_bytes;
        *inner.site_bytes.entry(site_id).or_insert(0) += object_bytes;
        sync_store_metrics(&inner);
        note_insertion();
        (metrics.on_insert)();
    }

    pub fn invalidate_key_if_same(&self, key: &CacheKey, entry_id: u64) {
        self.try_remove_if_same(key, entry_id);
    }

    pub fn try_remove_if_same(&self, key: &CacheKey, entry_id: u64) {
        let mut inner = self.inner.write().expect("cache write");
        let Some(current) = inner.entries.get(key) else {
            return;
        };
        if current.entry_id != entry_id {
            return;
        }
        detach_entry(&mut inner, key);
        sync_store_metrics(&inner);
        note_eviction();
    }

    /// Hard-delete one key (WC3 purge URL). Idempotent.
    pub fn invalidate_key(&self, key: &CacheKey) -> PurgeStats {
        let mut stats = PurgeStats::default();
        let mut inner = self.inner.write().expect("cache write");
        if let Some(entry) = detach_entry(&mut inner, key) {
            stats.purged_entries = 1;
            stats.purged_bytes = entry.object_bytes as u64;
            sync_store_metrics(&inner);
            note_eviction();
        }
        stats
    }

    /// Hard-delete all entries for one site/tenant (WC3). O(n) scan; off hot path.
    pub fn invalidate_site(&self, site_id: u64) -> PurgeStats {
        let keys: Vec<CacheKey> = {
            let inner = self.inner.read().expect("cache read");
            inner
                .entries
                .iter()
                .filter(|(_, entry)| entry.site_id == site_id)
                .map(|(key, _)| key.clone())
                .collect()
        };
        self.purge_keys(&keys)
    }

    /// Hard-delete entries for `site_id` with matching `runtime_generation` (WC3).
    pub fn invalidate_site_runtime_generation(&self, site_id: u64, generation: u64) -> PurgeStats {
        let keys: Vec<CacheKey> = {
            let inner = self.inner.read().expect("cache read");
            inner
                .entries
                .iter()
                .filter(|(_, entry)| {
                    entry.site_id == site_id && entry.runtime_generation == generation
                })
                .map(|(key, _)| key.clone())
                .collect()
        };
        self.purge_keys(&keys)
    }

    /// Purge every entry carrying `tag`, across sites. Used by internal
    /// invalidation (a changed static file invalidates all its variants).
    pub fn invalidate_tag(&self, tag: &str) -> PurgeStats {
        let keys: Vec<CacheKey> = {
            let inner = self.inner.read().expect("cache read");
            inner.tag_index.get(tag).cloned().unwrap_or_default()
        };
        self.purge_keys(&keys)
    }

    /// NG-16: purge the entries of `site_id` tagged `tag`. Tags are shared
    /// strings across sites and the tag index is global, so this filter is
    /// what keeps an operator purge tenant-scoped.
    pub fn invalidate_tag_for_site(&self, site_id: u64, tag: &str) -> PurgeStats {
        let keys: Vec<CacheKey> = {
            let inner = self.inner.read().expect("cache read");
            inner
                .tag_index
                .get(tag)
                .map(|keys| {
                    keys.iter()
                        .filter(|key| {
                            inner
                                .entries
                                .get(*key)
                                .is_some_and(|entry| entry.site_id == site_id)
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        self.purge_keys(&keys)
    }

    pub fn invalidate_route_generation(&self, route_idx: usize, generation: u64) {
        let keys: Vec<CacheKey> = {
            let inner = self.inner.read().expect("cache read");
            inner
                .route_generation_index
                .get(&(route_idx, generation))
                .cloned()
                .unwrap_or_default()
        };
        for key in keys {
            self.force_remove_key(&key);
        }
    }

    pub fn invalidate_runtime_generation(&self, generation: u64) {
        let keys: Vec<CacheKey> = {
            let inner = self.inner.read().expect("cache read");
            inner
                .entries
                .iter()
                .filter(|(_, entry)| entry.runtime_generation == generation)
                .map(|(key, _)| key.clone())
                .collect()
        };
        for key in keys {
            self.force_remove_key(&key);
        }
    }

    fn purge_keys(&self, keys: &[CacheKey]) -> PurgeStats {
        let mut stats = PurgeStats::default();
        for key in keys {
            let mut inner = self.inner.write().expect("cache write");
            if let Some(entry) = detach_entry(&mut inner, key) {
                stats.purged_entries += 1;
                stats.purged_bytes += entry.object_bytes as u64;
                sync_store_metrics(&inner);
                note_eviction();
            }
        }
        stats
    }

    fn force_remove_key(&self, key: &CacheKey) {
        let mut inner = self.inner.write().expect("cache write");
        if detach_entry(&mut inner, key).is_some() {
            sync_store_metrics(&inner);
            note_eviction();
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.write().expect("cache write");
        inner.entries.clear();
        inner.tag_index.clear();
        inner.route_generation_index.clear();
        inner.site_bytes.clear();
        inner.fifo.clear();
        inner.total_bytes = 0;
        sync_store_metrics(&inner);
    }

    pub fn snapshot(&self) -> CacheStoreSnapshot {
        let inner = self.inner.read().expect("cache read");
        CacheStoreSnapshot {
            entries: inner.entries.len(),
            bytes: inner.total_bytes,
        }
    }

    pub fn metrics_match_store(&self) -> bool {
        let snap = self.snapshot();
        snap.entries == crate::metrics::cache_entries_current()
            && snap.bytes == crate::metrics::cache_bytes_current()
    }

    #[doc(hidden)]
    pub fn store_read_lock_held(&self) -> bool {
        self.inner.try_read().is_err()
    }

    #[doc(hidden)]
    pub fn store_write_lock_held(&self) -> bool {
        self.inner.try_write().is_err()
    }
}

enum LookupOutcome {
    Hit(Arc<CachedEntry>),
    Miss,
    /// Expired but still inside the stale retention window (NG-16).
    Stale,
    Expired(u64),
}

fn index_entry(inner: &mut ResponseCacheInner, key: &CacheKey, entry: &CachedEntry) {
    inner
        .route_generation_index
        .entry((entry.route_idx, entry.runtime_generation))
        .or_default()
        .push(key.clone());
    for tag in entry.invalidation_tags.iter() {
        inner
            .tag_index
            .entry(tag.clone())
            .or_default()
            .push(key.clone());
    }
}

fn detach_entry(inner: &mut ResponseCacheInner, key: &CacheKey) -> Option<Arc<CachedEntry>> {
    let entry = inner.entries.remove(key)?;
    inner.total_bytes = inner.total_bytes.saturating_sub(entry.object_bytes);
    if let Some(site_total) = inner.site_bytes.get_mut(&entry.site_id) {
        *site_total = site_total.saturating_sub(entry.object_bytes);
        if *site_total == 0 {
            inner.site_bytes.remove(&entry.site_id);
        }
    }
    if let Some(keys) = inner
        .route_generation_index
        .get_mut(&(entry.route_idx, entry.runtime_generation))
    {
        keys.retain(|k| k != key);
        if keys.is_empty() {
            inner
                .route_generation_index
                .remove(&(entry.route_idx, entry.runtime_generation));
        }
    }
    for tag in entry.invalidation_tags.iter() {
        if let Some(keys) = inner.tag_index.get_mut(tag) {
            keys.retain(|k| k != key);
            if keys.is_empty() {
                inner.tag_index.remove(tag);
            }
        }
    }
    Some(entry)
}

fn evict_for_insert(
    inner: &mut ResponseCacheInner,
    incoming_bytes: usize,
    site_id: u64,
    max_entries: usize,
    max_total_bytes: usize,
    max_bytes_per_site: Option<usize>,
    stale_retention: Duration,
) {
    purge_expired(inner, stale_retention);
    let mut guard = 0usize;
    while needs_eviction(
        inner,
        incoming_bytes,
        site_id,
        max_entries,
        max_total_bytes,
        max_bytes_per_site,
    ) && guard < max_entries.saturating_add(8)
    {
        guard += 1;
        let prefer_site = max_bytes_per_site.is_some_and(|cap| {
            inner
                .site_bytes
                .get(&site_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(incoming_bytes)
                > cap
        });
        let Some(victim) = pop_fifo_victim(inner, prefer_site.then_some(site_id)) else {
            break;
        };
        if detach_entry(inner, &victim).is_some() {
            note_eviction();
        }
    }
}

fn needs_eviction(
    inner: &ResponseCacheInner,
    incoming_bytes: usize,
    site_id: u64,
    max_entries: usize,
    max_total_bytes: usize,
    max_bytes_per_site: Option<usize>,
) -> bool {
    if inner.entries.len() >= max_entries {
        return true;
    }
    if inner.total_bytes.saturating_add(incoming_bytes) > max_total_bytes {
        return true;
    }
    if let Some(cap) = max_bytes_per_site {
        let site = inner.site_bytes.get(&site_id).copied().unwrap_or(0);
        if site.saturating_add(incoming_bytes) > cap {
            return true;
        }
    }
    false
}

fn purge_expired(inner: &mut ResponseCacheInner, stale_retention: Duration) {
    let now = Instant::now();
    let expired: Vec<CacheKey> = inner
        .entries
        .iter()
        .filter_map(|(key, entry)| {
            // NG-16: an entry inside the stale window is not garbage yet —
            // wiping it on every insert would make stale-if-error unreachable
            // under normal traffic.
            let keep_for_stale =
                !stale_retention.is_zero() && entry.expires_at + stale_retention > now;
            (entry.expires_at <= now && !keep_for_stale).then_some(key.clone())
        })
        .collect();
    for key in expired {
        if detach_entry(inner, &key).is_some() {
            note_eviction();
        }
    }
}

fn sync_store_metrics(inner: &ResponseCacheInner) {
    crate::metrics::sync_size_metrics(inner.entries.len(), inner.total_bytes);
}

fn pop_fifo_victim(inner: &mut ResponseCacheInner, prefer_site: Option<u64>) -> Option<CacheKey> {
    if let Some(site_id) = prefer_site {
        let mut skipped = VecDeque::new();
        let mut found = None;
        while let Some(key) = inner.fifo.pop_front() {
            if !inner.entries.contains_key(&key) {
                continue;
            }
            if inner
                .entries
                .get(&key)
                .is_some_and(|e| e.site_id == site_id)
            {
                found = Some(key);
                break;
            }
            skipped.push_back(key);
        }
        while let Some(k) = skipped.pop_back() {
            inner.fifo.push_front(k);
        }
        if found.is_some() {
            return found;
        }
    }
    while let Some(key) = inner.fifo.pop_front() {
        if inner.entries.contains_key(&key) {
            return Some(key);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{build_cache_key, CacheKeyParts};
    use crate::metrics::lock_size_metrics_for_tests;

    fn sample_key(path: &str) -> CacheKey {
        sample_key_for_site(0, path)
    }

    fn sample_key_for_site(site_id: u64, path: &str) -> CacheKey {
        build_cache_key(CacheKeyParts {
            site_id,
            namespace: 1,
            backend_id: 0,
            runtime_generation: 1,
            policy_generation: 1,
            route_idx: 0,
            method: "GET".into(),
            scheme: "http".into(),
            host: "h".into(),
            path: path.into(),
            query: "".into(),
            content_encoding: "identity".into(),
            vary: String::new(),
        })
    }

    fn insert_bytes(cache: &ResponseCache, site_id: u64, path: &str, body: Bytes, ttl: Duration) {
        let key = sample_key_for_site(site_id, path);
        cache.insert_entry(
            key,
            site_id,
            0,
            1,
            ttl,
            200,
            vec![],
            body,
            None,
            CacheNamespace::new(1),
            Arc::from([]),
            NamespaceMetrics::NONE,
        );
    }

    fn insert_plain(
        cache: &ResponseCache,
        site_id: u64,
        path: &str,
        body: &'static [u8],
        ttl: Duration,
    ) {
        insert_bytes(cache, site_id, path, Bytes::from_static(body), ttl);
    }

    #[test]
    fn ttl_expires_entries() {
        // KF-P16-008: size gauges are process-singleton; serialize metric asserts.
        let _metrics = lock_size_metrics_for_tests();
        let cache = ResponseCache::with_limits(10, 1024 * 1024);
        let key = sample_key("/x");
        insert_plain(&cache, 0, "/x", b"hello", Duration::from_millis(50));
        assert!(cache.lookup(&key).is_some());
        std::thread::sleep(Duration::from_millis(80));
        assert!(cache.lookup(&key).is_none());
        assert!(cache.metrics_match_store());
    }

    #[test]
    fn max_entries_evicts_oldest() {
        let _metrics = lock_size_metrics_for_tests();
        let cache = ResponseCache::with_limits(3, 1024 * 1024);
        for i in 0..5 {
            insert_plain(&cache, 0, &format!("/f{i}"), b"x", Duration::from_secs(30));
        }
        assert!(cache.snapshot().entries <= 3);
        assert!(cache.metrics_match_store());
    }

    #[test]
    fn per_site_cap_evicts_only_that_site() {
        let _metrics = lock_size_metrics_for_tests();
        // Exact body bytes only (no stored headers) so caps are predictable.
        let cache = ResponseCache::with_limits_and_site_cap(100, 1024 * 1024, Some(20));
        insert_bytes(
            &cache,
            1,
            "/a",
            Bytes::from(vec![1u8; 10]),
            Duration::from_secs(60),
        );
        insert_bytes(
            &cache,
            1,
            "/b",
            Bytes::from(vec![2u8; 10]),
            Duration::from_secs(60),
        );
        insert_bytes(
            &cache,
            2,
            "/c",
            Bytes::from(vec![3u8; 10]),
            Duration::from_secs(60),
        );
        // Site 1 is at 20; another 10-byte insert must prefer site-1 victims.
        insert_bytes(
            &cache,
            1,
            "/d",
            Bytes::from(vec![4u8; 10]),
            Duration::from_secs(60),
        );
        assert!(cache.lookup(&sample_key_for_site(2, "/c")).is_some());
        assert!(cache.lookup(&sample_key_for_site(1, "/d")).is_some());
        assert!(cache.metrics_match_store());
    }

    /// NG-16: inside the retention window an expired entry is a miss for the
    /// request but stays available to `stale-if-error`.
    #[test]
    fn stale_retention_serves_expired_entry_to_stale_lookup() {
        let cache = ResponseCache::with_limits(10, 1024 * 1024)
            .with_stale_retention(Duration::from_secs(30));
        insert_plain(&cache, 1, "/s", b"stale-body", Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(10));
        let key = sample_key_for_site(1, "/s");
        assert!(cache.lookup(&key).is_none(), "expired is a miss");
        let stale = cache.lookup_stale(&key).expect("retained stale entry");
        assert_eq!(stale.body.as_ref(), b"stale-body");
    }

    #[test]
    fn stale_lookup_is_off_without_retention() {
        let cache = ResponseCache::with_limits(10, 1024 * 1024);
        insert_plain(&cache, 1, "/s", b"x", Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(10));
        let key = sample_key_for_site(1, "/s");
        assert!(cache.lookup(&key).is_none());
        assert!(
            cache.lookup_stale(&key).is_none(),
            "no retention configured"
        );
    }

    /// NG-16: tag purge removes exactly the tagged entries and reports counts.
    #[test]
    fn invalidate_tag_purges_only_tagged_entries() {
        let cache = ResponseCache::with_limits(10, 1024 * 1024);
        let tagged = sample_key_for_site(1, "/tagged");
        let other = sample_key_for_site(1, "/other");
        cache.insert_entry(
            tagged.clone(),
            1,
            0,
            1,
            Duration::from_secs(60),
            200,
            vec![],
            Bytes::from_static(b"tagged"),
            None,
            CacheNamespace::new(4),
            std::sync::Arc::from(vec!["blog".to_string()]),
            NamespaceMetrics::NONE,
        );
        insert_plain(&cache, 1, "/other", b"other", Duration::from_secs(60));
        let stats = cache.invalidate_tag("blog");
        assert_eq!(stats.purged_entries, 1);
        assert!(cache.lookup(&tagged).is_none());
        assert!(cache.lookup(&other).is_some());
    }

    /// F1 (audit): the tag index is global; the operator purge must stay
    /// inside the requested site.
    #[test]
    fn invalidate_tag_for_site_is_tenant_scoped() {
        let cache = ResponseCache::with_limits(10, 1024 * 1024);
        for site in [1u64, 2u64] {
            cache.insert_entry(
                sample_key_for_site(site, "/blog"),
                site,
                0,
                1,
                Duration::from_secs(60),
                200,
                vec![],
                Bytes::from_static(b"post"),
                None,
                CacheNamespace::new(4),
                std::sync::Arc::from(vec!["blog".to_string()]),
                NamespaceMetrics::NONE,
            );
        }
        let stats = cache.invalidate_tag_for_site(1, "blog");
        assert_eq!(stats.purged_entries, 1);
        assert!(cache.lookup(&sample_key_for_site(1, "/blog")).is_none());
        assert!(
            cache.lookup(&sample_key_for_site(2, "/blog")).is_some(),
            "site 2 must survive site 1's purge"
        );
        // The internal, cross-site variant still purges everything.
        assert_eq!(cache.invalidate_tag("blog").purged_entries, 1);
    }

    /// F2 (audit): an insert must not wipe entries still inside the stale window.
    #[test]
    fn insert_keeps_entries_inside_stale_window() {
        let cache = ResponseCache::with_limits(10, 1024 * 1024)
            .with_stale_retention(Duration::from_secs(30));
        insert_plain(&cache, 1, "/keep", b"keep", Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(10));
        insert_plain(&cache, 1, "/new", b"new", Duration::from_secs(60));
        let stale = cache
            .lookup_stale(&sample_key_for_site(1, "/keep"))
            .expect("stale entry must survive another insert");
        assert_eq!(stale.body.as_ref(), b"keep");
    }

    /// F3 (audit): stale is bounded by the retention window, not just by
    /// "expired".
    #[test]
    fn stale_lookup_expires_after_the_window() {
        let cache = ResponseCache::with_limits(10, 1024 * 1024)
            .with_stale_retention(Duration::from_millis(20));
        insert_plain(&cache, 1, "/w", b"w", Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(10));
        assert!(cache.lookup_stale(&sample_key_for_site(1, "/w")).is_some());
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            cache.lookup_stale(&sample_key_for_site(1, "/w")).is_none(),
            "past the retention window the entry is not stale-servable"
        );
    }

    #[test]
    fn distinct_site_keys_do_not_share_entries() {
        let cache = ResponseCache::with_limits(10, 1024 * 1024);
        insert_plain(&cache, 1, "/same", b"one", Duration::from_secs(60));
        insert_plain(&cache, 2, "/same", b"two", Duration::from_secs(60));
        let a = cache
            .lookup(&sample_key_for_site(1, "/same"))
            .expect("site1");
        let b = cache
            .lookup(&sample_key_for_site(2, "/same"))
            .expect("site2");
        assert_eq!(a.body.as_ref(), b"one");
        assert_eq!(b.body.as_ref(), b"two");
    }
}
