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
//! Plan 12 proxy microcache counters (module-owned KD3.5).

use std::sync::atomic::{AtomicU64, Ordering};

static CACHE_PROXY_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_PROXY_MISSES: AtomicU64 = AtomicU64::new(0);
static CACHE_PROXY_INSERTIONS: AtomicU64 = AtomicU64::new(0);
static CACHE_PROXY_REJECTIONS: AtomicU64 = AtomicU64::new(0);

pub fn note_proxy_hit() {
    CACHE_PROXY_HITS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_proxy_miss() {
    CACHE_PROXY_MISSES.fetch_add(1, Ordering::Relaxed);
}

pub fn note_proxy_insertion() {
    CACHE_PROXY_INSERTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_proxy_rejection() {
    CACHE_PROXY_REJECTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn cache_proxy_hits_total() -> u64 {
    CACHE_PROXY_HITS.load(Ordering::Relaxed)
}

pub fn cache_proxy_misses_total() -> u64 {
    CACHE_PROXY_MISSES.load(Ordering::Relaxed)
}

pub fn cache_proxy_insertions_total() -> u64 {
    CACHE_PROXY_INSERTIONS.load(Ordering::Relaxed)
}

pub fn cache_proxy_rejections_total() -> u64 {
    CACHE_PROXY_REJECTIONS.load(Ordering::Relaxed)
}

#[doc(hidden)]
pub fn reset_proxy_cache_metrics_for_tests() {
    CACHE_PROXY_HITS.store(0, Ordering::Relaxed);
    CACHE_PROXY_MISSES.store(0, Ordering::Relaxed);
    CACHE_PROXY_INSERTIONS.store(0, Ordering::Relaxed);
    CACHE_PROXY_REJECTIONS.store(0, Ordering::Relaxed);
}
