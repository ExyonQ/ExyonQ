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
//! Composition seam + core hook shim for epoll (PS3A-PM3-R2).
//!
//! **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**
//!
//! Productive FSM lives in `exyonq-platform-linux`. This module owns:
//! - env gates
//! - OnceLock registration of platform starters
//! - static_wire hook installation (needs `create_epoll_keepalive_transfer`, `pub(crate)`)

/// Env gate for `EXYONQ_EPOLL_STATIC` (mechanism flag only).
pub fn epoll_static_env_enabled(tls_on_listen: bool) -> bool {
    !tls_on_listen && std::env::var("EXYONQ_EPOLL_STATIC").ok().as_deref() == Some("1")
}

/// Env gate for `EXYONQ_EPOLL_LISTEN` (epoll owns accept). Default off.
pub fn epoll_listen_env_enabled() -> bool {
    std::env::var("EXYONQ_EPOLL_LISTEN").ok().as_deref() == Some("1")
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::kernel::{EpollKeepaliveTransfer, PlatformConnectionEntry};
    use crate::lifecycle;
    use crate::server::os_worker_guard::OsWorkerGuard;
    use exyonq_module_api::static_epoll;
    use exyonq_module_api::static_wire;
    use std::io;
    use std::net::SocketAddr;
    use std::os::unix::io::{AsRawFd, RawFd};
    use std::sync::{Arc, OnceLock};
    use tokio::net::TcpStream as TokioTcpStream;

    pub type EpollListenStartFn =
        fn(SocketAddr, usize, Arc<PlatformConnectionEntry>) -> io::Result<OsWorkerGuard>;
    pub type EpollKeepalivePrepareFn = fn(usize, Arc<PlatformConnectionEntry>);
    pub type EpollKeepaliveStopFn = fn();
    /// Platform enqueue: ownership of `fd` transfers on `Ok`.
    pub type EpollKeepaliveEnqueueFn =
        fn(RawFd, SocketAddr, [u8; 512], usize, bool, EpollKeepaliveTransfer) -> io::Result<()>;

    static EPOLL_LISTEN_START: OnceLock<EpollListenStartFn> = OnceLock::new();
    static EPOLL_KEEPALIVE_PREPARE: OnceLock<EpollKeepalivePrepareFn> = OnceLock::new();
    static EPOLL_KEEPALIVE_STOP: OnceLock<EpollKeepaliveStopFn> = OnceLock::new();
    static EPOLL_KEEPALIVE_ENQUEUE: OnceLock<EpollKeepaliveEnqueueFn> = OnceLock::new();
    static HOOK_ENTRY: OnceLock<Arc<PlatformConnectionEntry>> = OnceLock::new();

    pub fn register_epoll_listen_start(start: EpollListenStartFn) {
        let _ = EPOLL_LISTEN_START.set(start);
    }

    pub fn register_epoll_keepalive_prepare(prepare: EpollKeepalivePrepareFn) {
        let _ = EPOLL_KEEPALIVE_PREPARE.set(prepare);
    }

    pub fn register_epoll_keepalive_stop(stop: EpollKeepaliveStopFn) {
        let _ = EPOLL_KEEPALIVE_STOP.set(stop);
    }

    pub fn register_epoll_keepalive_enqueue(enqueue: EpollKeepaliveEnqueueFn) {
        let _ = EPOLL_KEEPALIVE_ENQUEUE.set(enqueue);
    }

    pub(crate) fn start_registered_epoll_listen(
        listen: SocketAddr,
        workers: usize,
        entry: Arc<PlatformConnectionEntry>,
    ) -> io::Result<OsWorkerGuard> {
        let start = EPOLL_LISTEN_START.get().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "epoll listen starter not registered (link exyonq-platform-linux)",
            )
        })?;
        start(listen, workers, entry)
    }

    pub(crate) fn prepare_registered_keepalive_pool(
        workers: usize,
        entry: Arc<PlatformConnectionEntry>,
    ) {
        let _ = HOOK_ENTRY.set(Arc::clone(&entry));
        if let Some(prepare) = EPOLL_KEEPALIVE_PREPARE.get() {
            prepare(workers, entry);
        }
    }

    pub(crate) fn stop_registered_keepalive_pool() {
        if let Some(stop) = EPOLL_KEEPALIVE_STOP.get() {
            stop();
        }
    }

    pub(crate) fn keepalive_handoff_enabled() -> bool {
        HOOK_ENTRY.get().is_some() && EPOLL_KEEPALIVE_ENQUEUE.get().is_some()
    }

    /// KD2.5: install core wire→epoll hooks into module-api (idempotent).
    pub(crate) fn install_static_wire_hooks() {
        static INSTALLED: OnceLock<()> = OnceLock::new();
        let _ = INSTALLED.get_or_init(|| {
            let _ = static_wire::install_wire_epoll_hooks(static_wire::StaticWireEpollHooks {
                register_sendfile_from_tokio_first,
                register_keepalive_from_tokio_first,
                register_keepalive_from_tokio,
                note_sendfile_fallback: static_epoll::note_sendfile_fallback,
                note_sendfile_fallback_rejected: static_epoll::note_sendfile_fallback_rejected,
                keepalive_handoff_enabled,
            });
        });
    }

    pub(crate) fn register_keepalive_from_tokio(
        stream: TokioTcpStream,
        carry: &[u8],
    ) -> Result<(), TokioTcpStream> {
        register_keepalive_from_tokio_buffered(stream, &[], carry, false)
    }

    pub(crate) fn register_keepalive_from_tokio_first(
        stream: TokioTcpStream,
        head: &[u8],
        carry: &[u8],
    ) -> Result<(), TokioTcpStream> {
        register_keepalive_from_tokio_buffered(stream, head, carry, false)
    }

    pub(crate) fn register_sendfile_from_tokio_first(
        stream: TokioTcpStream,
        head: &[u8],
        carry: &[u8],
    ) -> Result<(), TokioTcpStream> {
        register_keepalive_from_tokio_buffered(stream, head, carry, true)
    }

    fn register_keepalive_from_tokio_buffered(
        stream: TokioTcpStream,
        head: &[u8],
        carry: &[u8],
        sendfile: bool,
    ) -> Result<(), TokioTcpStream> {
        let Some(entry) = HOOK_ENTRY.get() else {
            return Err(stream);
        };
        let Some(enqueue) = EPOLL_KEEPALIVE_ENQUEUE.get() else {
            return Err(stream);
        };

        // Prefer transfer of existing task token — never double admit.
        let transfer = match lifecycle::take_task_connection_token() {
            Some(token) => {
                let pin = entry.executor().pin_bench_cache();
                entry.create_epoll_keepalive_transfer(
                    token,
                    pin.generation,
                    pin.site_static_slot,
                    pin.modules_enabled,
                )
            }
            None => match entry.executor().admit_for_epoll_interest() {
                Ok(attach) => entry.create_epoll_keepalive_transfer(
                    attach.token,
                    attach.bench_cache.generation,
                    attach.bench_cache.site_static_slot,
                    attach.bench_cache.modules_enabled,
                ),
                Err(_) => return Err(stream),
            },
        };

        let total = head.len() + carry.len();
        if total > 512 {
            return Err(stream);
        }
        let mut buf = [0u8; 512];
        buf[..head.len()].copy_from_slice(head);
        buf[head.len()..total].copy_from_slice(carry);

        let fd = stream.as_raw_fd();
        let peer = match stream.peer_addr() {
            Ok(addr) => addr,
            Err(_) => return Err(stream),
        };
        let dup_fd = unsafe {
            // SAFETY: `fd` is an open Tokio socket; `dup` yields an independent fd for epoll ownership.
            libc::dup(fd)
        };
        if dup_fd < 0 {
            return Err(stream);
        }

        match enqueue(dup_fd, peer, buf, total, sendfile, transfer) {
            Ok(()) => {
                drop(stream);
                Ok(())
            }
            Err(_) => {
                unsafe {
                    libc::close(dup_fd);
                }
                Err(stream)
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    install_static_wire_hooks, prepare_registered_keepalive_pool, start_registered_epoll_listen,
    stop_registered_keepalive_pool,
};
#[cfg(target_os = "linux")]
pub use linux::{
    register_epoll_keepalive_enqueue, register_epoll_keepalive_prepare,
    register_epoll_keepalive_stop, register_epoll_listen_start,
};
