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
    pub is_sendfile_bench_head: fn(head: &[u8]) -> bool,
    pub epoll_sendfile_eligible: fn(head: &[u8]) -> bool,
    pub static_wire_use_blocking_pool: fn(head: &[u8]) -> bool,
    pub static_sendfile_use_blocking_pool: fn(head: &[u8]) -> bool,
    pub static_one_m_sendfile_use_blocking: fn(head: &[u8]) -> bool,
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

pub fn is_sendfile_bench_head(head: &[u8]) -> bool {
    (eligibility().is_sendfile_bench_head)(head)
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

pub fn static_one_m_sendfile_use_blocking(head: &[u8]) -> bool {
    (eligibility().static_one_m_sendfile_use_blocking)(head)
}

/// Combined blocking-pool admission (inline wire + sendfile paths) — single module authority.
pub fn static_use_blocking_pool(head: &[u8]) -> bool {
    static_wire_use_blocking_pool(head) || static_sendfile_use_blocking_pool(head)
}

/// TCP path when sendfile is not eligible: inline wire, sendfile bench, and 1M bench gates.
pub fn static_tcp_blocking_pool_admission(head: &[u8]) -> bool {
    static_use_blocking_pool(head) || static_one_m_sendfile_use_blocking(head)
}

pub fn might_use_static_wire(head: &[u8]) -> bool {
    (eligibility().might_use_static_wire)(head)
}

/// P1 bench precooked wire bytes — module-owned rodata surfaced via hook.
pub struct StaticWireBenchHooks {
    pub p1_bench_wire_rodata: fn() -> &'static [u8],
}

static BENCH: OnceLock<StaticWireBenchHooks> = OnceLock::new();

pub fn install_wire_bench_hooks(hooks: StaticWireBenchHooks) -> Result<(), StaticWireBenchHooks> {
    BENCH.set(hooks)
}

fn bench() -> &'static StaticWireBenchHooks {
    BENCH
        .get()
        .expect("StaticWireBenchHooks not installed — register static module from CLI")
}

pub fn p1_bench_wire_rodata() -> &'static [u8] {
    (bench().p1_bench_wire_rodata)()
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
        fn(std::net::TcpStream) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
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

#[cfg(target_os = "linux")]
pub fn shed_blocking_admission(
    stream: std::net::TcpStream,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
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
