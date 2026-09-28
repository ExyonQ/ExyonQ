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
//! Epoll sendfile FSM hooks (KD2.5) — FSM state owned by module, fd lifecycle in core.

#[cfg(target_os = "linux")]
use std::os::unix::io::RawFd;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;

use crate::static_dispatch::SendfileHandle;

/// Bench cache view pinned at accept (no operational root type in core).
#[derive(Debug, Clone, Copy)]
pub struct StaticBenchCacheView {
    pub site_static_slot: Option<u32>,
    pub modules_enabled: bool,
    pub generation: u64,
}

/// Result of one FSM pump (stable contract for epoll worker).
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticEpollPumpResult {
    Progress,
    Parked,
    Complete,
    Error,
}

/// Opaque sendfile session id (module-owned `SendingState`).
#[cfg(target_os = "linux")]
pub type StaticEpollSession = u64;

/// Epoll interest masks (module may customize parked vs reading).
#[cfg(target_os = "linux")]
pub struct StaticEpollInterestHooks {
    pub interest_reading: fn() -> u32,
    pub interest_sending_parked: fn() -> u32,
}

#[cfg(target_os = "linux")]
static INTEREST: OnceLock<StaticEpollInterestHooks> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn install_epoll_interest_hooks(
    hooks: StaticEpollInterestHooks,
) -> Result<(), StaticEpollInterestHooks> {
    INTEREST.set(hooks)
}

#[cfg(target_os = "linux")]
fn interest() -> &'static StaticEpollInterestHooks {
    INTEREST
        .get()
        .expect("StaticEpollInterestHooks not installed — register static module from CLI")
}

#[cfg(target_os = "linux")]
pub fn interest_reading() -> u32 {
    (interest().interest_reading)()
}

#[cfg(target_os = "linux")]
pub fn interest_sending_parked() -> u32 {
    (interest().interest_sending_parked)()
}

/// Module-owned gated sendfile FSM (core stores session id in conn phase).
#[cfg(target_os = "linux")]
pub struct StaticEpollFsmHooks {
    pub is_head_wire_request: fn(head: &[u8]) -> bool,
    pub match_sendfile_asset: fn(site_slot: u32, head: &[u8]) -> Option<SendfileHandle>,
    /// Cap067 miss terminal: Cap004-classified HTTP wire (404/403) when match returned
    /// `None` for an eligible sendfile GET/HEAD. `None` = genuine Hyper fallback still
    /// required (ineligible / mechanism off). Never re-enter epoll sendfile via NeedHyper.
    pub sendfile_miss_http_wire: fn(site_slot: u32, head: &[u8]) -> Option<bytes::Bytes>,
    /// `head` is the full HTTP/1 request head (Cap019 may select a byte range).
    pub begin_sendfile_session:
        fn(fd: RawFd, head: &[u8], asset: SendfileHandle) -> Result<StaticEpollSession, ()>,
    pub pump_sendfile_session: fn(fd: RawFd, session: StaticEpollSession) -> StaticEpollPumpResult,
    pub clear_sendfile_session: fn(fd: RawFd, session: StaticEpollSession),
    /// Cap067: cancel unused issued handle (idempotent if already taken).
    pub release_sendfile_handle: fn(asset: SendfileHandle),
}

#[cfg(target_os = "linux")]
static FSM: OnceLock<StaticEpollFsmHooks> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn install_epoll_fsm_hooks(hooks: StaticEpollFsmHooks) -> Result<(), StaticEpollFsmHooks> {
    FSM.set(hooks)
}

#[cfg(target_os = "linux")]
fn fsm() -> &'static StaticEpollFsmHooks {
    FSM.get()
        .expect("StaticEpollFsmHooks not installed — register static module from CLI")
}

#[cfg(target_os = "linux")]
pub fn is_head_wire_request(head: &[u8]) -> bool {
    (fsm().is_head_wire_request)(head)
}

#[cfg(target_os = "linux")]
pub fn match_sendfile_asset(site_slot: u32, head: &[u8]) -> Option<SendfileHandle> {
    (fsm().match_sendfile_asset)(site_slot, head)
}

#[cfg(target_os = "linux")]
pub fn sendfile_miss_http_wire(site_slot: u32, head: &[u8]) -> Option<bytes::Bytes> {
    (fsm().sendfile_miss_http_wire)(site_slot, head)
}

#[cfg(target_os = "linux")]
#[allow(clippy::result_unit_err)]
pub fn begin_sendfile_session(
    fd: RawFd,
    head: &[u8],
    asset: SendfileHandle,
) -> Result<StaticEpollSession, ()> {
    (fsm().begin_sendfile_session)(fd, head, asset)
}

#[cfg(target_os = "linux")]
pub fn pump_sendfile_session(fd: RawFd, session: StaticEpollSession) -> StaticEpollPumpResult {
    (fsm().pump_sendfile_session)(fd, session)
}

#[cfg(target_os = "linux")]
pub fn clear_sendfile_session(fd: RawFd, session: StaticEpollSession) {
    (fsm().clear_sendfile_session)(fd, session);
}

#[cfg(target_os = "linux")]
pub fn release_sendfile_handle(asset: SendfileHandle) {
    (fsm().release_sendfile_handle)(asset);
}

/// Cap067: false when wire admit cannot reject, so the caller skips IP formatting.
#[cfg(target_os = "linux")]
#[inline]
pub fn product_admit_active() -> bool {
    crate::wire_admit_active()
}

/// Cap067 terminal-owner bridge for process wire admission.
#[cfg(target_os = "linux")]
pub fn admit_product_request(client_ip: &str) -> Result<(), u64> {
    match crate::wire_admit(client_ip) {
        crate::WireAdmit::Allow => Ok(()),
        crate::WireAdmit::Reject429 { retry_after_secs } => Err(retry_after_secs),
    }
}

/// Cap067 terminal-owner bridge for the canonical 429 wire response.
#[cfg(target_os = "linux")]
pub fn rate_limit_reject_response(retry_after_secs: u64) -> Vec<u8> {
    crate::rate_limit_reject_wire(retry_after_secs)
}

/// Cap067 terminal-owner bridge for one delivered response status.
#[cfg(target_os = "linux")]
pub fn record_product_response(status: u16) {
    crate::wire_record_response(status);
}

/// Delivered inline response: counter, route label, and latency.
#[cfg(target_os = "linux")]
pub fn record_product_exchange(head: &[u8], status: u16, elapsed_ms: f64) {
    let method = if head.starts_with(b"HEAD ") {
        "HEAD"
    } else {
        "GET"
    };
    let path = request_target(head);
    crate::wire_record_exchange(method, path, status, elapsed_ms);
}

#[cfg(target_os = "linux")]
fn request_target(head: &[u8]) -> &str {
    let line_end = head
        .iter()
        .position(|&b| b == b'\r' || b == b'\n')
        .unwrap_or(head.len());
    let Ok(line) = std::str::from_utf8(&head[..line_end]) else {
        return "/";
    };
    line.split_whitespace().nth(1).unwrap_or("/")
}

/// Epoll sendfile metrics (module-owned counters).
#[cfg(target_os = "linux")]
#[allow(clippy::type_complexity)]
pub struct StaticEpollMetricsHooks {
    pub note_complete: fn(),
    pub note_parked: fn(),
    pub note_error: fn(),
    pub note_header_reject: fn(),
    pub note_terminal_503: fn(),
    pub note_unknown_fd_event: fn(),
    pub append_prometheus: fn(out: &mut String),
    pub metrics_tuple: fn() -> (u64, u64, u64, u64, u64, u64, u64, u64),
    pub note_sendfile_fallback: fn(),
    pub note_sendfile_fallback_rejected: fn(),
    #[cfg(any(test, feature = "test-utils"))]
    pub reset_for_test: fn(),
}

#[cfg(target_os = "linux")]
static METRICS: OnceLock<StaticEpollMetricsHooks> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn install_epoll_metrics_hooks(
    hooks: StaticEpollMetricsHooks,
) -> Result<(), StaticEpollMetricsHooks> {
    METRICS.set(hooks)
}

#[cfg(target_os = "linux")]
fn metrics() -> &'static StaticEpollMetricsHooks {
    METRICS
        .get()
        .expect("StaticEpollMetricsHooks not installed — register static module from CLI")
}

#[cfg(target_os = "linux")]
pub fn note_complete() {
    (metrics().note_complete)();
}

#[cfg(target_os = "linux")]
pub fn note_parked() {
    (metrics().note_parked)();
}

#[cfg(target_os = "linux")]
pub fn note_error() {
    (metrics().note_error)();
}

#[cfg(target_os = "linux")]
pub fn note_header_reject() {
    (metrics().note_header_reject)();
}

#[cfg(target_os = "linux")]
pub fn note_terminal_503() {
    (metrics().note_terminal_503)();
}

#[cfg(target_os = "linux")]
pub fn note_unknown_fd_event() {
    (metrics().note_unknown_fd_event)();
}

#[cfg(target_os = "linux")]
pub fn epoll_sendfile_metrics() -> (u64, u64, u64, u64, u64, u64, u64, u64) {
    (metrics().metrics_tuple)()
}

#[cfg(target_os = "linux")]
pub fn append_prometheus(out: &mut String) {
    if let Some(hooks) = METRICS.get() {
        (hooks.append_prometheus)(out);
    }
}

#[cfg(target_os = "linux")]
pub fn note_sendfile_fallback() {
    (metrics().note_sendfile_fallback)();
}

#[cfg(target_os = "linux")]
pub fn note_sendfile_fallback_rejected() {
    (metrics().note_sendfile_fallback_rejected)();
}

/// Epoll inline wire write hook (health probe + handoff for site static).
#[cfg(target_os = "linux")]
#[allow(clippy::type_complexity)]
pub struct StaticEpollInlineWireHooks {
    pub try_write_inline_wire_response: fn(
        site_slot: u32,
        fd: RawFd,
        head: &[u8],
        write_fn: fn(RawFd, &[u8]) -> std::io::Result<()>,
    ) -> StaticEpollInlineWireResult,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticEpollInlineWireResult {
    Written,
    Handoff,
    NoMatch,
}

#[cfg(target_os = "linux")]
static INLINE_WIRE: OnceLock<StaticEpollInlineWireHooks> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn install_epoll_inline_wire_hooks(
    hooks: StaticEpollInlineWireHooks,
) -> Result<(), StaticEpollInlineWireHooks> {
    INLINE_WIRE.set(hooks)
}

#[cfg(target_os = "linux")]
fn inline_wire_hooks() -> &'static StaticEpollInlineWireHooks {
    INLINE_WIRE
        .get()
        .expect("StaticEpollInlineWireHooks not installed — register static module from CLI")
}

#[cfg(target_os = "linux")]
pub fn try_write_inline_wire_response(
    site_slot: u32,
    fd: RawFd,
    head: &[u8],
    write_fn: fn(RawFd, &[u8]) -> std::io::Result<()>,
) -> StaticEpollInlineWireResult {
    (inline_wire_hooks().try_write_inline_wire_response)(site_slot, fd, head, write_fn)
}
