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
//! Opt-in micro-timing counters for perf iteration (`EXYONQ_BENCH_TRACE=1`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

static READ_US: AtomicU64 = AtomicU64::new(0);
static MATCH_US: AtomicU64 = AtomicU64::new(0);
static WRITE_US: AtomicU64 = AtomicU64::new(0);
static KEEPALIVE_HEADER_US: AtomicU64 = AtomicU64::new(0);
static SAMPLES: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var("EXYONQ_BENCH_TRACE").ok().as_deref() == Some("1"))
}

enum SampleKind {
    Read,
    Match,
    Write,
    KeepaliveHeader,
}

pub struct SampleGuard {
    start: Instant,
    kind: SampleKind,
}

impl SampleGuard {
    fn begin(kind: SampleKind) -> Option<Self> {
        if !enabled() {
            return None;
        }
        Some(Self {
            start: Instant::now(),
            kind,
        })
    }

    pub fn read() -> Option<Self> {
        Self::begin(SampleKind::Read)
    }

    pub fn write() -> Option<Self> {
        Self::begin(SampleKind::Write)
    }

    pub fn keepalive_header() -> Option<Self> {
        Self::begin(SampleKind::KeepaliveHeader)
    }

    pub fn route_match() -> Option<Self> {
        Self::begin(SampleKind::Match)
    }
}

impl Drop for SampleGuard {
    fn drop(&mut self) {
        let us = self.start.elapsed().as_micros().min(u64::MAX as u128) as u64;
        let counter = match self.kind {
            SampleKind::Read => &READ_US,
            SampleKind::Match => &MATCH_US,
            SampleKind::Write => &WRITE_US,
            SampleKind::KeepaliveHeader => &KEEPALIVE_HEADER_US,
        };
        counter.fetch_add(us, Ordering::Relaxed);
        SAMPLES.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn record_match_us(duration: Duration) {
    if !enabled() {
        return;
    }
    MATCH_US.fetch_add(
        duration.as_micros().min(u64::MAX as u128) as u64,
        Ordering::Relaxed,
    );
    SAMPLES.fetch_add(1, Ordering::Relaxed);
}

pub fn append_prometheus(out: &mut String) {
    if !enabled() {
        return;
    }
    out.push_str("# TYPE exyonq_bench_trace_read_us_total counter\n");
    out.push_str(&format!(
        "exyonq_bench_trace_read_us_total {}\n",
        READ_US.load(Ordering::Relaxed)
    ));
    out.push_str("# TYPE exyonq_bench_trace_match_us_total counter\n");
    out.push_str(&format!(
        "exyonq_bench_trace_match_us_total {}\n",
        MATCH_US.load(Ordering::Relaxed)
    ));
    out.push_str("# TYPE exyonq_bench_trace_write_us_total counter\n");
    out.push_str(&format!(
        "exyonq_bench_trace_write_us_total {}\n",
        WRITE_US.load(Ordering::Relaxed)
    ));
    out.push_str("# TYPE exyonq_bench_trace_keepalive_header_us_total counter\n");
    out.push_str(&format!(
        "exyonq_bench_trace_keepalive_header_us_total {}\n",
        KEEPALIVE_HEADER_US.load(Ordering::Relaxed)
    ));
    out.push_str("# TYPE exyonq_bench_trace_samples_total counter\n");
    out.push_str(&format!(
        "exyonq_bench_trace_samples_total {}\n",
        SAMPLES.load(Ordering::Relaxed)
    ));
}
