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
//! KD4.1 — FastCGI cache metrics (module-owned).

use std::sync::atomic::{AtomicU64, Ordering};

static FCGI_HITS: AtomicU64 = AtomicU64::new(0);
static FCGI_MISSES: AtomicU64 = AtomicU64::new(0);
static FCGI_INSERTIONS: AtomicU64 = AtomicU64::new(0);
static FCGI_REJECTIONS: AtomicU64 = AtomicU64::new(0);

pub fn note_fcgi_hit() {
    FCGI_HITS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fcgi_miss() {
    FCGI_MISSES.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fcgi_insertion() {
    FCGI_INSERTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fcgi_rejection() {
    FCGI_REJECTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn cache_fcgi_hits_total() -> u64 {
    FCGI_HITS.load(Ordering::Relaxed)
}

pub fn cache_fcgi_misses_total() -> u64 {
    FCGI_MISSES.load(Ordering::Relaxed)
}

pub fn cache_fcgi_insertions_total() -> u64 {
    FCGI_INSERTIONS.load(Ordering::Relaxed)
}

pub fn cache_fcgi_rejections_total() -> u64 {
    FCGI_REJECTIONS.load(Ordering::Relaxed)
}

pub fn reset_fcgi_cache_metrics_for_tests() {
    FCGI_HITS.store(0, Ordering::Relaxed);
    FCGI_MISSES.store(0, Ordering::Relaxed);
    FCGI_INSERTIONS.store(0, Ordering::Relaxed);
    FCGI_REJECTIONS.store(0, Ordering::Relaxed);
}
