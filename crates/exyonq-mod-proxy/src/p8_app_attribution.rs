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
//! P8O3 diagnostic counters for SSE proxy wire attribution.
//! Compiled only with feature `p8-app-attribution`. Not a product surface.

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant};

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static COMPLETED_STREAMS: AtomicU64 = AtomicU64::new(0);
static UPSTREAM_BODY_POLLS: AtomicU64 = AtomicU64::new(0);
static DOWNSTREAM_WRITES: AtomicU64 = AtomicU64::new(0);
static CHUNKS_IN: AtomicU64 = AtomicU64::new(0);
static CHUNKS_OUT: AtomicU64 = AtomicU64::new(0);
static BYTES_IN: AtomicU64 = AtomicU64::new(0);
static BYTES_OUT: AtomicU64 = AtomicU64::new(0);
static WRITE_CHUNK_CALLS: AtomicU64 = AtomicU64::new(0);
static WRITE_ALL_CALLS: AtomicU64 = AtomicU64::new(0);
static HEADER_WRITE_BYTES: AtomicU64 = AtomicU64::new(0);
static BODY_WRITE_BYTES: AtomicU64 = AtomicU64::new(0);
static FLUSH_CALLS: AtomicU64 = AtomicU64::new(0);
static BACKPRESSURE_HITS: AtomicU64 = AtomicU64::new(0);
static CLIENT_DISCONNECTS: AtomicU64 = AtomicU64::new(0);
static HANDLER_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static PROXY_FORWARD_NS: AtomicU64 = AtomicU64::new(0);
static BODY_POLL_NS: AtomicU64 = AtomicU64::new(0);
static WIRE_WRITE_NS: AtomicU64 = AtomicU64::new(0);
static HEADER_WRITE_NS: AtomicU64 = AtomicU64::new(0);
static CHUNK_FRAME_NS: AtomicU64 = AtomicU64::new(0);

static DUMPER_STARTED: AtomicBool = AtomicBool::new(false);
static INIT: Once = Once::new();

#[inline]
fn add(counter: &AtomicU64, value: u64) {
    counter.fetch_add(value, Ordering::Relaxed);
}

pub fn note_request() {
    ensure_dumper();
    add(&REQUESTS, 1);
}

pub fn note_completed_stream(total_ns: u64, forward_ns: u64) {
    add(&COMPLETED_STREAMS, 1);
    add(&HANDLER_TOTAL_NS, total_ns);
    add(&PROXY_FORWARD_NS, forward_ns);
}

pub fn note_body_poll(ns: u64) {
    add(&UPSTREAM_BODY_POLLS, 1);
    add(&BODY_POLL_NS, ns);
}

pub fn note_chunk_in(bytes: usize) {
    add(&CHUNKS_IN, 1);
    add(&BYTES_IN, bytes as u64);
}

pub fn note_header_write(bytes: usize, ns: u64) {
    add(&HEADER_WRITE_BYTES, bytes as u64);
    add(&HEADER_WRITE_NS, ns);
    add(&DOWNSTREAM_WRITES, 1);
    add(&WRITE_ALL_CALLS, 1);
    add(&WIRE_WRITE_NS, ns);
}

pub fn note_write_chunk(bytes: usize, frame_ns: u64, write_ns: u64, flushed: bool) {
    add(&WRITE_CHUNK_CALLS, 1);
    add(&CHUNKS_OUT, 1);
    add(&BYTES_OUT, bytes as u64);
    add(&BODY_WRITE_BYTES, bytes as u64);
    add(&CHUNK_FRAME_NS, frame_ns);
    add(&WIRE_WRITE_NS, write_ns);
    add(&DOWNSTREAM_WRITES, 1);
    add(&WRITE_ALL_CALLS, 1);
    if flushed {
        add(&FLUSH_CALLS, 1);
        add(&BACKPRESSURE_HITS, 1);
    }
}

pub fn note_write_all(bytes: usize, ns: u64) {
    add(&DOWNSTREAM_WRITES, 1);
    add(&WRITE_ALL_CALLS, 1);
    add(&BODY_WRITE_BYTES, bytes as u64);
    add(&BYTES_OUT, bytes as u64);
    add(&WIRE_WRITE_NS, ns);
}

pub fn note_flush(ns: u64) {
    add(&FLUSH_CALLS, 1);
    add(&WIRE_WRITE_NS, ns);
}

pub fn note_client_disconnect() {
    add(&CLIENT_DISCONNECTS, 1);
}

pub fn snapshot_json() -> String {
    let requests = REQUESTS.load(Ordering::Relaxed);
    let completed = COMPLETED_STREAMS.load(Ordering::Relaxed);
    let per = |total: u64| -> f64 {
        if completed == 0 {
            0.0
        } else {
            total as f64 / completed as f64
        }
    };
    format!(
        concat!(
            "{{\n",
            "  \"schema\": \"p8_app_attribution.v1\",\n",
            "  \"diagnostic_only\": true,\n",
            "  \"P8_APP_REQUESTS\": {requests},\n",
            "  \"P8_APP_COMPLETED_STREAMS\": {completed},\n",
            "  \"P8_APP_UPSTREAM_BODY_POLLS\": {polls},\n",
            "  \"P8_APP_DOWNSTREAM_WRITES\": {writes},\n",
            "  \"P8_APP_CHUNKS_IN\": {cin},\n",
            "  \"P8_APP_CHUNKS_OUT\": {cout},\n",
            "  \"P8_APP_BYTES_IN\": {bin},\n",
            "  \"P8_APP_BYTES_OUT\": {bout},\n",
            "  \"P8_APP_WRITE_CHUNK_CALLS\": {wcc},\n",
            "  \"P8_APP_WRITE_ALL_CALLS\": {wac},\n",
            "  \"P8_APP_HEADER_WRITE_BYTES\": {hwb},\n",
            "  \"P8_APP_BODY_WRITE_BYTES\": {bwb},\n",
            "  \"P8_APP_FLUSH_CALLS\": {flush},\n",
            "  \"P8_APP_BACKPRESSURE_HITS\": {bp},\n",
            "  \"P8_APP_CLIENT_DISCONNECTS\": {disc},\n",
            "  \"P8_APP_ALLOC_COUNTER_IF_AVAILABLE\": \"NOT_AVAILABLE\",\n",
            "  \"P8_APP_HANDLER_TOTAL_NS\": {ht},\n",
            "  \"P8_APP_PROXY_FORWARD_NS\": {pf},\n",
            "  \"P8_APP_BODY_POLL_NS\": {bpn},\n",
            "  \"P8_APP_WIRE_WRITE_NS\": {wn},\n",
            "  \"P8_APP_HEADER_WRITE_NS\": {hwn},\n",
            "  \"P8_APP_CHUNK_FRAME_NS\": {cfn},\n",
            "  \"per_completed_stream\": {{\n",
            "    \"handler_ns\": {pht},\n",
            "    \"body_poll_ns\": {pbp},\n",
            "    \"wire_write_ns\": {pww},\n",
            "    \"header_write_ns\": {phw},\n",
            "    \"chunk_frame_ns\": {pcf},\n",
            "    \"chunks_in\": {pci},\n",
            "    \"chunks_out\": {pco},\n",
            "    \"write_chunk_calls\": {pwcc}\n",
            "  }}\n",
            "}}\n"
        ),
        requests = requests,
        completed = completed,
        polls = UPSTREAM_BODY_POLLS.load(Ordering::Relaxed),
        writes = DOWNSTREAM_WRITES.load(Ordering::Relaxed),
        cin = CHUNKS_IN.load(Ordering::Relaxed),
        cout = CHUNKS_OUT.load(Ordering::Relaxed),
        bin = BYTES_IN.load(Ordering::Relaxed),
        bout = BYTES_OUT.load(Ordering::Relaxed),
        wcc = WRITE_CHUNK_CALLS.load(Ordering::Relaxed),
        wac = WRITE_ALL_CALLS.load(Ordering::Relaxed),
        hwb = HEADER_WRITE_BYTES.load(Ordering::Relaxed),
        bwb = BODY_WRITE_BYTES.load(Ordering::Relaxed),
        flush = FLUSH_CALLS.load(Ordering::Relaxed),
        bp = BACKPRESSURE_HITS.load(Ordering::Relaxed),
        disc = CLIENT_DISCONNECTS.load(Ordering::Relaxed),
        ht = HANDLER_TOTAL_NS.load(Ordering::Relaxed),
        pf = PROXY_FORWARD_NS.load(Ordering::Relaxed),
        bpn = BODY_POLL_NS.load(Ordering::Relaxed),
        wn = WIRE_WRITE_NS.load(Ordering::Relaxed),
        hwn = HEADER_WRITE_NS.load(Ordering::Relaxed),
        cfn = CHUNK_FRAME_NS.load(Ordering::Relaxed),
        pht = per(HANDLER_TOTAL_NS.load(Ordering::Relaxed)),
        pbp = per(BODY_POLL_NS.load(Ordering::Relaxed)),
        pww = per(WIRE_WRITE_NS.load(Ordering::Relaxed)),
        phw = per(HEADER_WRITE_NS.load(Ordering::Relaxed)),
        pcf = per(CHUNK_FRAME_NS.load(Ordering::Relaxed)),
        pci = per(CHUNKS_IN.load(Ordering::Relaxed)),
        pco = per(CHUNKS_OUT.load(Ordering::Relaxed)),
        pwcc = per(WRITE_CHUNK_CALLS.load(Ordering::Relaxed)),
    )
}

fn ensure_dumper() {
    INIT.call_once(|| {
        if DUMPER_STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        let path = std::env::var_os("EXYONQ_P8_APP_ATTRIBUTION_OUT")
            .unwrap_or_else(|| std::ffi::OsString::from("/tmp/p8_app_attribution.json"));
        std::thread::Builder::new()
            .name("p8-app-attr-dump".into())
            .spawn(move || loop {
                if let Err(err) = fs::write(&path, snapshot_json()) {
                    eprintln!("p8-app-attr-dump: write {}: {err}", path.display());
                }
                std::thread::sleep(Duration::from_secs(2));
            })
            .ok();
    });
}

pub struct ScopeTimer {
    start: Instant,
}

impl ScopeTimer {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed_ns(&self) -> u64 {
        self.start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }
}
