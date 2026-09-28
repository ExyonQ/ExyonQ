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
//! Narrow wire/event-loop hooks (KD2.5) — core never imports module wire internals.

use std::io;
use std::sync::OnceLock;

pub struct StaticWireEligibilityHooks {
    pub epoll_keep_alive_eligible: fn(head: &[u8]) -> bool,
    pub epoll_sendfile_eligible: fn(head: &[u8]) -> bool,
    pub static_wire_use_blocking_pool: fn(head: &[u8]) -> bool,
    pub static_sendfile_use_blocking_pool: fn(head: &[u8]) -> bool,
    pub might_use_static_wire: fn(head: &[u8]) -> bool,
}

static ELIGIBILITY: OnceLock<StaticWireEligibilityHooks> = OnceLock::new();

/// Install wire eligibility hooks once (composition root / module bootstrap).
pub fn install_wire_eligibility_hooks(
    hooks: StaticWireEligibilityHooks,
) -> Result<(), StaticWireEligibilityHooks> {
    ELIGIBILITY.set(hooks)
}

fn eligibility() -> &'static StaticWireEligibilityHooks {
    ELIGIBILITY
        .get()
        .expect("StaticWireEligibilityHooks not installed — register static module from CLI")
}

pub fn epoll_keep_alive_eligible(head: &[u8]) -> bool {
    (eligibility().epoll_keep_alive_eligible)(head)
}

pub fn epoll_sendfile_eligible(head: &[u8]) -> bool {
    (eligibility().epoll_sendfile_eligible)(head)
}

pub fn static_wire_use_blocking_pool(head: &[u8]) -> bool {
    (eligibility().static_wire_use_blocking_pool)(head)
}

pub fn static_sendfile_use_blocking_pool(head: &[u8]) -> bool {
    (eligibility().static_sendfile_use_blocking_pool)(head)
}

/// Combined blocking-pool admission (inline wire + sendfile paths) — single module authority.
pub fn static_use_blocking_pool(head: &[u8]) -> bool {
    static_wire_use_blocking_pool(head) || static_sendfile_use_blocking_pool(head)
}

/// TCP path blocking-pool admission (inline wire + sendfile gates).
pub fn static_tcp_blocking_pool_admission(head: &[u8]) -> bool {
    static_use_blocking_pool(head)
}

pub fn might_use_static_wire(head: &[u8]) -> bool {
    (eligibility().might_use_static_wire)(head)
}

/// Epoll/Tokio registration hooks (sendfile handoff from async path).
#[cfg(target_os = "linux")]
#[allow(clippy::type_complexity)]
pub struct StaticWireEpollHooks {
    pub register_sendfile_from_tokio_first:
        fn(tokio::net::TcpStream, &[u8], &[u8]) -> Result<(), tokio::net::TcpStream>,
    pub register_keepalive_from_tokio_first:
        fn(tokio::net::TcpStream, &[u8], &[u8]) -> Result<(), tokio::net::TcpStream>,
    pub register_keepalive_from_tokio:
        fn(tokio::net::TcpStream, &[u8]) -> Result<(), tokio::net::TcpStream>,
    pub note_sendfile_fallback: fn(),
    pub note_sendfile_fallback_rejected: fn(),
    pub keepalive_handoff_enabled: fn() -> bool,
}

#[cfg(target_os = "linux")]
static EPOLL: OnceLock<StaticWireEpollHooks> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn install_wire_epoll_hooks(hooks: StaticWireEpollHooks) -> Result<(), StaticWireEpollHooks> {
    EPOLL.set(hooks)
}

#[cfg(target_os = "linux")]
fn epoll_hooks() -> &'static StaticWireEpollHooks {
    EPOLL
        .get()
        .expect("StaticWireEpollHooks not installed — register static module from CLI")
}

#[cfg(target_os = "linux")]
pub fn register_sendfile_from_tokio_first(
    stream: tokio::net::TcpStream,
    head: &[u8],
    carry: &[u8],
) -> Result<(), tokio::net::TcpStream> {
    (epoll_hooks().register_sendfile_from_tokio_first)(stream, head, carry)
}

#[cfg(target_os = "linux")]
pub fn register_keepalive_from_tokio_first(
    stream: tokio::net::TcpStream,
    head: &[u8],
    carry: &[u8],
) -> Result<(), tokio::net::TcpStream> {
    (epoll_hooks().register_keepalive_from_tokio_first)(stream, head, carry)
}

#[cfg(target_os = "linux")]
pub fn register_keepalive_from_tokio(
    stream: tokio::net::TcpStream,
    carry: &[u8],
) -> Result<(), tokio::net::TcpStream> {
    (epoll_hooks().register_keepalive_from_tokio)(stream, carry)
}

#[cfg(target_os = "linux")]
pub fn note_sendfile_fallback() {
    (epoll_hooks().note_sendfile_fallback)();
}

#[cfg(target_os = "linux")]
pub fn note_sendfile_fallback_rejected() {
    (epoll_hooks().note_sendfile_fallback_rejected)();
}

#[cfg(target_os = "linux")]
pub fn keepalive_handoff_enabled() -> bool {
    (epoll_hooks().keepalive_handoff_enabled)()
}

/// Blocking wire serve hook (sync accept / io_uring path) — no `StaticRoot` in core.
#[cfg(target_os = "linux")]
pub struct StaticWireServeHooks {
    pub serve_blocking_sync: fn(
        site_slot: u32,
        stream: &mut std::net::TcpStream,
        head: bytes::Bytes,
        rest: bytes::Bytes,
    ) -> io::Result<()>,
    pub try_spawn_blocking_static: fn(
        site_slot: u32,
        stream: std::net::TcpStream,
        head: bytes::Bytes,
        rest: bytes::Bytes,
    ) -> BlockingAdmission,
    pub shed_blocking_admission:
        fn(
            std::net::TcpStream,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>>,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub enum BlockingAdmission {
    Accepted,
    Rejected(std::net::TcpStream),
}

#[cfg(target_os = "linux")]
static SERVE: OnceLock<StaticWireServeHooks> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn install_wire_serve_hooks(hooks: StaticWireServeHooks) -> Result<(), StaticWireServeHooks> {
    SERVE.set(hooks)
}

#[cfg(target_os = "linux")]
fn serve_hooks() -> &'static StaticWireServeHooks {
    SERVE
        .get()
        .expect("StaticWireServeHooks not installed — register static module from CLI")
}

#[cfg(target_os = "linux")]
pub fn serve_blocking_sync(
    site_slot: u32,
    stream: &mut std::net::TcpStream,
    head: bytes::Bytes,
    rest: bytes::Bytes,
) -> io::Result<()> {
    (serve_hooks().serve_blocking_sync)(site_slot, stream, head, rest)
}

#[cfg(target_os = "linux")]
pub fn try_spawn_blocking_static(
    site_slot: u32,
    stream: std::net::TcpStream,
    head: bytes::Bytes,
    rest: bytes::Bytes,
) -> BlockingAdmission {
    (serve_hooks().try_spawn_blocking_static)(site_slot, stream, head, rest)
}

/// Cap041: optional hook to take connection admission ownership into a blocking handoff.
#[cfg(target_os = "linux")]
#[allow(clippy::type_complexity)] // OnceLock of dyn Send hold factory — Cap041 admission handoff
static TAKE_LIFECYCLE_HOLD: OnceLock<fn() -> Option<Box<dyn Send>>> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn install_lifecycle_hold_take(take: fn() -> Option<Box<dyn Send>>) {
    let _ = TAKE_LIFECYCLE_HOLD.set(take);
}

#[cfg(target_os = "linux")]
pub fn take_lifecycle_hold() -> Option<Box<dyn Send>> {
    TAKE_LIFECYCLE_HOLD.get().and_then(|f| f())
}

#[cfg(target_os = "linux")]
pub fn shed_blocking_admission(
    stream: std::net::TcpStream,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>> {
    (serve_hooks().shed_blocking_admission)(stream)
}

/// Async wire serve hook return type (clippy: type complexity).
pub type StaticWireServeFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>>;

/// Async wire serve (Tokio path) — owned stream (no connection internals in core).
pub struct StaticWireAsyncHooks {
    pub serve_wire_site_tcp: fn(
        site_slot: u32,
        stream: tokio::net::TcpStream,
        head: bytes::Bytes,
        rest: bytes::Bytes,
    ) -> StaticWireServeFuture,
}

static ASYNC: OnceLock<StaticWireAsyncHooks> = OnceLock::new();

pub fn install_wire_async_hooks(hooks: StaticWireAsyncHooks) -> Result<(), StaticWireAsyncHooks> {
    ASYNC.set(hooks)
}

fn async_hooks() -> &'static StaticWireAsyncHooks {
    ASYNC
        .get()
        .expect("StaticWireAsyncHooks not installed — register static module from CLI")
}

pub fn serve_wire_site_tcp(
    site_slot: u32,
    stream: tokio::net::TcpStream,
    head: bytes::Bytes,
    rest: bytes::Bytes,
) -> StaticWireServeFuture {
    (async_hooks().serve_wire_site_tcp)(site_slot, stream, head, rest)
}

/// Cap061 — terminal access notice after a static-wire response status is known.
///
/// Core registers a hook that emits observability; modules must not hardcode success.
#[derive(Debug, Clone)]
pub struct StaticWireAccessNotice {
    pub head: bytes::Bytes,
    pub status: u16,
    pub outcome: &'static str,
    pub duration_ms: u64,
}

type AccessNoticeHook = Box<dyn Fn(StaticWireAccessNotice) -> Option<String> + Send + Sync>;

static ACCESS_NOTICE: OnceLock<AccessNoticeHook> = OnceLock::new();
/// Composition-root / reload flag: skip head copy + hook when access events are off.
static ACCESS_NOTICES_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);
/// When set, Cap067 must not own the request: Hyper enters the OTel request span.
static WIRE_OTEL_SPANS_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Register once from the composition root / core bootstrap.
/// `Err(())` means the hook was already installed (`OnceLock::set` collision).
#[allow(clippy::result_unit_err)]
pub fn install_access_notice_hook<F>(f: F) -> Result<(), ()>
where
    F: Fn(StaticWireAccessNotice) -> Option<String> + Send + Sync + 'static,
{
    ACCESS_NOTICE.set(Box::new(f)).map_err(|_| ())
}

/// Enable/disable static-wire access notices (IR `[logging.access]` + env override).
pub fn set_access_notices_enabled(enabled: bool) {
    ACCESS_NOTICES_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub fn access_notices_enabled() -> bool {
    ACCESS_NOTICES_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Cap061: OTel request spans are created on the Hyper path.
pub fn set_wire_otel_spans_enabled(enabled: bool) {
    WIRE_OTEL_SPANS_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub fn wire_otel_spans_enabled() -> bool {
    WIRE_OTEL_SPANS_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Emit terminal access when status is known (no-op if hook unset or notices disabled).
pub fn notify_access(notice: StaticWireAccessNotice) -> Option<String> {
    if !access_notices_enabled() {
        return None;
    }
    ACCESS_NOTICE.get().and_then(|hook| hook(notice))
}
