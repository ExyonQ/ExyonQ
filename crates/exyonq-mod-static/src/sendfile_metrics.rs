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
//! ADR-025 Phase 2 PR #3: epoll sendfile wired-path counters (opt-in gate from PR #2).
//!
//! Counters increment only on the gated epoll FSM path (`EXYONQ_EPOLL_STATIC=1` +
//! `EXYONQ_EPOLL_SENDFILE=1`). Default blocking P2/P3 does not touch these series.
//!
//! Prometheus names (exported via `/metrics` when the metrics module is enabled):
//! - `exyonq_epoll_sendfile_complete_total`
//! - `exyonq_epoll_sendfile_parked_total`
//! - `exyonq_epoll_sendfile_error_total`
//! - `exyonq_epoll_sendfile_header_reject_total`
//! - `exyonq_epoll_sendfile_terminal_503_total`
//! - `exyonq_epoll_sendfile_fallback_total`
//! - `exyonq_epoll_sendfile_fallback_rejected_total`
//! - `exyonq_epoll_sendfile_unknown_fd_event_total`

use std::sync::atomic::{AtomicU64, Ordering};

static COMPLETE: AtomicU64 = AtomicU64::new(0);
static PARKED: AtomicU64 = AtomicU64::new(0);
static ERROR: AtomicU64 = AtomicU64::new(0);
static HEADER_REJECT: AtomicU64 = AtomicU64::new(0);
static FALLBACK: AtomicU64 = AtomicU64::new(0);
static FALLBACK_REJECTED: AtomicU64 = AtomicU64::new(0);
static TERMINAL_503: AtomicU64 = AtomicU64::new(0);
static UNKNOWN_FD_EVENT: AtomicU64 = AtomicU64::new(0);

/// Snapshot of epoll sendfile wired-path counters (Linux opt-in path only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EpollSendfileMetrics {
    pub complete_total: u64,
    pub parked_total: u64,
    pub error_total: u64,
    pub header_reject_total: u64,
    pub terminal_503_total: u64,
    pub fallback_total: u64,
    pub fallback_rejected_total: u64,
    pub unknown_fd_event_total: u64,
}

impl EpollSendfileMetrics {
    pub fn snapshot() -> Self {
        Self {
            complete_total: COMPLETE.load(Ordering::Relaxed),
            parked_total: PARKED.load(Ordering::Relaxed),
            error_total: ERROR.load(Ordering::Relaxed),
            header_reject_total: HEADER_REJECT.load(Ordering::Relaxed),
            terminal_503_total: TERMINAL_503.load(Ordering::Relaxed),
            fallback_total: FALLBACK.load(Ordering::Relaxed),
            fallback_rejected_total: FALLBACK_REJECTED.load(Ordering::Relaxed),
            unknown_fd_event_total: UNKNOWN_FD_EVENT.load(Ordering::Relaxed),
        }
    }
}

/// Primary getter (named fields). Prefer over the legacy tuple API.
pub fn epoll_sendfile_metrics_snapshot() -> EpollSendfileMetrics {
    EpollSendfileMetrics::snapshot()
}

/// Legacy tuple getter (PR #2). Order matches historical `epoll_sendfile_metrics()`.
pub fn epoll_sendfile_metrics() -> (u64, u64, u64, u64, u64, u64, u64, u64) {
    let m = EpollSendfileMetrics::snapshot();
    (
        m.complete_total,
        m.parked_total,
        m.error_total,
        m.header_reject_total,
        m.fallback_total,
        m.fallback_rejected_total,
        m.terminal_503_total,
        m.unknown_fd_event_total,
    )
}

pub fn note_complete() {
    COMPLETE.fetch_add(1, Ordering::Relaxed);
    tracing::trace!("epoll_sendfile complete");
}

pub fn note_parked() {
    PARKED.fetch_add(1, Ordering::Relaxed);
    tracing::trace!("epoll_sendfile parked");
}

pub fn note_error() {
    ERROR.fetch_add(1, Ordering::Relaxed);
    tracing::trace!("epoll_sendfile error");
}

pub fn note_header_reject() {
    HEADER_REJECT.fetch_add(1, Ordering::Relaxed);
    tracing::trace!("epoll_sendfile header_reject");
}

pub fn note_terminal_503() {
    TERMINAL_503.fetch_add(1, Ordering::Relaxed);
    tracing::trace!("epoll_sendfile terminal_503");
}

/// Synchronous epoll-register fallback to blocking admission (original stream preserved).
pub fn note_fallback() {
    FALLBACK.fetch_add(1, Ordering::Relaxed);
    tracing::trace!("epoll_sendfile fallback");
}

/// Fallback to blocking admission then rejected → 503 shed.
pub fn note_fallback_rejected() {
    FALLBACK_REJECTED.fetch_add(1, Ordering::Relaxed);
    tracing::trace!("epoll_sendfile fallback_rejected");
}

pub fn note_unknown_fd_event() {
    UNKNOWN_FD_EVENT.fetch_add(1, Ordering::Relaxed);
    tracing::trace!("epoll_sendfile unknown_fd_event");
}

/// PR #2 alias — synchronous epoll-register fallback (used from `static_conn`).
pub fn note_sendfile_fallback() {
    note_fallback();
}

/// PR #2 alias — blocking admission rejected after fallback.
pub fn note_sendfile_fallback_rejected() {
    note_fallback_rejected();
}

/// Append OpenMetrics/Prometheus counter lines for the wired path (cold path — `/metrics` scrape).
pub fn append_prometheus(out: &mut String) {
    let m = EpollSendfileMetrics::snapshot();
    out.push_str(
        "# HELP exyonq_epoll_sendfile_complete_total Epoll sendfile FSM responses completed.\n",
    );
    out.push_str("# TYPE exyonq_epoll_sendfile_complete_total counter\n");
    out.push_str(&format!(
        "exyonq_epoll_sendfile_complete_total {}\n",
        m.complete_total
    ));
    out.push_str("# HELP exyonq_epoll_sendfile_parked_total Epoll sendfile FSM body/header parked on EAGAIN.\n");
    out.push_str("# TYPE exyonq_epoll_sendfile_parked_total counter\n");
    out.push_str(&format!(
        "exyonq_epoll_sendfile_parked_total {}\n",
        m.parked_total
    ));
    out.push_str(
        "# HELP exyonq_epoll_sendfile_error_total Epoll sendfile FSM terminal send errors.\n",
    );
    out.push_str("# TYPE exyonq_epoll_sendfile_error_total counter\n");
    out.push_str(&format!(
        "exyonq_epoll_sendfile_error_total {}\n",
        m.error_total
    ));
    out.push_str(
        "# HELP exyonq_epoll_sendfile_header_reject_total Epoll sendfile invalid response header.\n",
    );
    out.push_str("# TYPE exyonq_epoll_sendfile_header_reject_total counter\n");
    out.push_str(&format!(
        "exyonq_epoll_sendfile_header_reject_total {}\n",
        m.header_reject_total
    ));
    out.push_str(
        "# HELP exyonq_epoll_sendfile_terminal_503_total Controlled 503 after epoll register failure.\n",
    );
    out.push_str("# TYPE exyonq_epoll_sendfile_terminal_503_total counter\n");
    out.push_str(&format!(
        "exyonq_epoll_sendfile_terminal_503_total {}\n",
        m.terminal_503_total
    ));
    out.push_str(
        "# HELP exyonq_epoll_sendfile_fallback_total Sync register fail → blocking admission.\n",
    );
    out.push_str("# TYPE exyonq_epoll_sendfile_fallback_total counter\n");
    out.push_str(&format!(
        "exyonq_epoll_sendfile_fallback_total {}\n",
        m.fallback_total
    ));
    out.push_str(
        "# HELP exyonq_epoll_sendfile_fallback_rejected_total Blocking admission rejected after fallback.\n",
    );
    out.push_str("# TYPE exyonq_epoll_sendfile_fallback_rejected_total counter\n");
    out.push_str(&format!(
        "exyonq_epoll_sendfile_fallback_rejected_total {}\n",
        m.fallback_rejected_total
    ));
    out.push_str(
        "# HELP exyonq_epoll_sendfile_unknown_fd_event_total Non-fatal epoll event for unknown fd.\n",
    );
    out.push_str("# TYPE exyonq_epoll_sendfile_unknown_fd_event_total counter\n");
    out.push_str(&format!(
        "exyonq_epoll_sendfile_unknown_fd_event_total {}\n",
        m.unknown_fd_event_total
    ));
}

/// Minimal OpenMetrics scrape for the wire/epoll path (`GET /metrics` on bench.toml).
pub fn wire_prometheus_response() -> Vec<u8> {
    let mut body = String::new();
    append_prometheus(&mut body);
    body.push_str("# EOF\n");
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/openmetrics-text; version=1.0.0; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

#[cfg(any(test, feature = "test-utils"))]
#[doc(hidden)]
pub fn reset_for_test() {
    COMPLETE.store(0, Ordering::Relaxed);
    PARKED.store(0, Ordering::Relaxed);
    ERROR.store(0, Ordering::Relaxed);
    HEADER_REJECT.store(0, Ordering::Relaxed);
    FALLBACK.store(0, Ordering::Relaxed);
    FALLBACK_REJECTED.store(0, Ordering::Relaxed);
    TERMINAL_503.store(0, Ordering::Relaxed);
    UNKNOWN_FD_EVENT.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    fn serial_metrics_test() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    #[test]
    fn snapshot_and_tuple_api_match() {
        let _guard = serial_metrics_test();
        reset_for_test();
        note_complete();
        note_parked();
        note_error();
        note_header_reject();
        note_fallback();
        note_fallback_rejected();
        note_terminal_503();
        note_unknown_fd_event();
        let m = EpollSendfileMetrics::snapshot();
        assert_eq!(m.complete_total, 1);
        assert_eq!(m.parked_total, 1);
        assert_eq!(m.error_total, 1);
        assert_eq!(m.header_reject_total, 1);
        assert_eq!(m.fallback_total, 1);
        assert_eq!(m.fallback_rejected_total, 1);
        assert_eq!(m.terminal_503_total, 1);
        assert_eq!(m.unknown_fd_event_total, 1);
        let tuple = epoll_sendfile_metrics();
        assert_eq!(tuple, (1, 1, 1, 1, 1, 1, 1, 1));
    }

    #[test]
    fn prometheus_exports_all_series() {
        let _guard = serial_metrics_test();
        reset_for_test();
        note_complete();
        note_complete();
        let mut out = String::new();
        append_prometheus(&mut out);
        for name in [
            "exyonq_epoll_sendfile_complete_total 2",
            "exyonq_epoll_sendfile_parked_total 0",
            "exyonq_epoll_sendfile_error_total 0",
            "exyonq_epoll_sendfile_header_reject_total 0",
            "exyonq_epoll_sendfile_terminal_503_total 0",
            "exyonq_epoll_sendfile_fallback_total 0",
            "exyonq_epoll_sendfile_fallback_rejected_total 0",
            "exyonq_epoll_sendfile_unknown_fd_event_total 0",
        ] {
            assert!(out.contains(name), "missing {name} in:\n{out}");
        }
        assert!(out.contains("# TYPE exyonq_epoll_sendfile_complete_total counter"));
    }

    #[test]
    fn counters_start_at_zero_after_reset() {
        let _guard = serial_metrics_test();
        reset_for_test();
        let m = EpollSendfileMetrics::snapshot();
        assert_eq!(m, EpollSendfileMetrics::default());
    }

    #[test]
    fn error_and_header_reject_independent() {
        let _guard = serial_metrics_test();
        reset_for_test();
        note_header_reject();
        note_error();
        let m = EpollSendfileMetrics::snapshot();
        assert_eq!(m.header_reject_total, 1);
        assert_eq!(m.error_total, 1);
        assert_eq!(m.complete_total, 0);
    }
}
