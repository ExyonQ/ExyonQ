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
//! KD3.4 — Proxy wire transport hooks (core never imports module wire internals).

use std::io::{self, IoSlice};
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

#[cfg(unix)]
use std::os::fd::RawFd;

/// Transport-agnostic bidirectional wire byte stream (implementation in modules).
pub trait WireStream: Send + Unpin {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>>;

    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>>;

    /// Vectored write. Default: first non-empty slice via [`Self::poll_write`].
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        for buf in bufs {
            if !buf.is_empty() {
                return self.as_mut().poll_write(cx, buf);
            }
        }
        Poll::Ready(Ok(0))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>>;

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>>;

    /// Peer TCP fd when captured by the adapter (Unix). Default: none.
    /// Needed for TCP_CORK / MSG_MORE; requires fd capture at `box_wire_stream`.
    #[cfg(unix)]
    fn peer_tcp_fd(&self) -> Option<RawFd> {
        None
    }
}

pub type BoxedWireStream = Box<dyn WireStream>;

/// Optional Linux send knobs (cork). Installed by CLI from platform-linux.
/// `exyonq-mod-proxy` must not depend on platform-linux (D1).
#[cfg(unix)]
pub struct ProxyTcpSendHooks {
    pub set_cork: fn(RawFd, bool) -> io::Result<()>,
}

#[cfg(unix)]
static TCP_SEND: OnceLock<ProxyTcpSendHooks> = OnceLock::new();

#[cfg(unix)]
pub fn install_proxy_tcp_send_hooks(hooks: ProxyTcpSendHooks) -> Result<(), ProxyTcpSendHooks> {
    TCP_SEND.set(hooks)
}

/// Apply TCP_CORK when hooks are installed; `None` if not composed.
#[cfg(unix)]
pub fn try_set_tcp_cork(fd: RawFd, on: bool) -> Option<io::Result<()>> {
    TCP_SEND.get().map(|h| (h.set_cork)(fd, on))
}

pub struct ProxyWireEligibilityHooks {
    pub might_use_proxy_wire: fn(head: &[u8]) -> bool,
}

static ELIGIBILITY: OnceLock<ProxyWireEligibilityHooks> = OnceLock::new();

pub fn install_wire_eligibility_hooks(
    hooks: ProxyWireEligibilityHooks,
) -> Result<(), ProxyWireEligibilityHooks> {
    ELIGIBILITY.set(hooks)
}

fn eligibility() -> &'static ProxyWireEligibilityHooks {
    ELIGIBILITY
        .get()
        .expect("ProxyWireEligibilityHooks not installed — register proxy module from CLI")
}

pub fn might_use_proxy_wire(head: &[u8]) -> bool {
    (eligibility().might_use_proxy_wire)(head)
}

pub type ProxyWireServeFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>>;

/// Async proxy wire serve — module-owned FSM; core passes compiled cluster id only.
pub struct ProxyWireAsyncHooks {
    pub serve_proxy_wire: fn(
        cluster_id: u32,
        generation: u64,
        x_forwarded_for: String,
        stream: BoxedWireStream,
        head: bytes::Bytes,
        rest: bytes::Bytes,
    ) -> ProxyWireServeFuture,
}

static ASYNC: OnceLock<ProxyWireAsyncHooks> = OnceLock::new();

pub fn install_wire_async_hooks(hooks: ProxyWireAsyncHooks) -> Result<(), ProxyWireAsyncHooks> {
    ASYNC.set(hooks)
}

fn async_hooks() -> &'static ProxyWireAsyncHooks {
    ASYNC
        .get()
        .expect("ProxyWireAsyncHooks not installed — register proxy module from CLI")
}

pub fn serve_proxy_wire(
    cluster_id: u32,
    generation: u64,
    x_forwarded_for: String,
    stream: BoxedWireStream,
    head: bytes::Bytes,
    rest: bytes::Bytes,
) -> ProxyWireServeFuture {
    (async_hooks().serve_proxy_wire)(cluster_id, generation, x_forwarded_for, stream, head, rest)
}
