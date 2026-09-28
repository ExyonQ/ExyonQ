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

/// Cap067: epoll static/keepalive mechanism — auto ON for cleartext Linux listens.
///
/// Kill-switch: `EXYONQ_EPOLL_STATIC=0` (or `off`/`false`). Explicit `=1` remains force-on.
/// TLS listens never enable this path (sendfile/epoll wire is cleartext H1 only).
///
/// Env resolved once at first observation (process-lifetime). Emergency disable
/// requires process restart.
pub fn epoll_static_env_enabled(tls_on_listen: bool) -> bool {
    if tls_on_listen {
        return false;
    }
    static ENABLED: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);
    use std::sync::atomic::Ordering;
    let cached = ENABLED.load(Ordering::Relaxed);
    if cached >= 0 {
        return cached != 0;
    }
    let v = !matches!(
        std::env::var("EXYONQ_EPOLL_STATIC"),
        Ok(raw) if env_flag_disabled(&raw)
    );
    let encoded: i8 = if v { 1 } else { 0 };
    let _ = ENABLED.compare_exchange(-1, encoded, Ordering::Relaxed, Ordering::Relaxed);
    ENABLED.load(Ordering::Relaxed) != 0
}

fn env_flag_disabled(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "0" | "off" | "false" | "no"
    )
}

/// Env gate for `EXYONQ_EPOLL_LISTEN` (epoll owns accept). Default off.
///
/// Process-lifetime cache (listen path; not per-request, but same S1 class).
pub fn epoll_listen_env_enabled() -> bool {
    static ENABLED: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);
    use std::sync::atomic::Ordering;
    let cached = ENABLED.load(Ordering::Relaxed);
    if cached >= 0 {
        return cached != 0;
    }
    let v = std::env::var("EXYONQ_EPOLL_LISTEN").ok().as_deref() == Some("1");
    let encoded: i8 = if v { 1 } else { 0 };
    let _ = ENABLED.compare_exchange(-1, encoded, Ordering::Relaxed, Ordering::Relaxed);
    ENABLED.load(Ordering::Relaxed) != 0
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
    pub type EpollKeepaliveSignalStopFn = fn();
    /// Platform enqueue: `fd` ownership transfers on `Ok`; transfer returned on `Err` for restore.
    pub type EpollKeepaliveEnqueueFn = fn(
        RawFd,
        SocketAddr,
        [u8; 512],
        usize,
        bool,
        EpollKeepaliveTransfer,
    ) -> Result<(), (io::Error, EpollKeepaliveTransfer)>;

    static EPOLL_LISTEN_START: OnceLock<EpollListenStartFn> = OnceLock::new();
    static EPOLL_KEEPALIVE_PREPARE: OnceLock<EpollKeepalivePrepareFn> = OnceLock::new();
    static EPOLL_KEEPALIVE_STOP: OnceLock<EpollKeepaliveStopFn> = OnceLock::new();
    static EPOLL_KEEPALIVE_SIGNAL_STOP: OnceLock<EpollKeepaliveSignalStopFn> = OnceLock::new();
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

    pub fn register_epoll_keepalive_signal_stop(signal: EpollKeepaliveSignalStopFn) {
        let _ = EPOLL_KEEPALIVE_SIGNAL_STOP.set(signal);
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

    /// Cap041: signal keepalive drain without joining (call before finalize wait).
    pub(crate) fn signal_registered_keepalive_pool_stop() {
        if let Some(signal) = EPOLL_KEEPALIVE_SIGNAL_STOP.get() {
            signal();
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
            // Cap041: blocking-static handoff takes task-local admission ownership.
            static_wire::install_lifecycle_hold_take(|| {
                crate::lifecycle::take_task_connection_token()
                    .map(|token| Box::new(token) as Box<dyn Send>)
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

        // V044-LIFECYCLE-C1: fallible prep must complete before moving admission ownership.
        // Token stays task-local while SOCKET_OWNER remains the Tokio stream.
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
        let owned = match exyonq_linux_ffi::dup_raw_fd(fd) {
            Ok(o) => o,
            Err(_) => return Err(stream),
        };
        let dup_fd = {
            use std::os::fd::IntoRawFd;
            owned.into_raw_fd()
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
                Err(_) => {
                    exyonq_linux_ffi::close_raw_fd(dup_fd);
                    return Err(stream);
                }
            },
        };

        match enqueue(dup_fd, peer, buf, total, sendfile, transfer) {
            Ok(()) => {
                drop(stream);
                Ok(())
            }
            Err((_err, transfer)) => {
                // Single close owner on failure: this function (enqueue no longer closes).
                exyonq_linux_ffi::close_raw_fd(dup_fd);
                let token = transfer.into_admission_token();
                match lifecycle::restore_task_connection_token(token) {
                    Ok(()) => Err(stream),
                    Err(token) => {
                        // LA-CAP067-OWN-001: never Ok(()) after a failed handoff.
                        // Admission accounting is already lost; return the stream so the
                        // caller can Hyper-fallback / close instead of silent EOF success.
                        drop(token);
                        Err(stream)
                    }
                }
            }
        }
    }

    #[cfg(test)]
    mod c1_ownership_tests {
        use super::*;
        use crate::kernel::PlatformConnectionEntry;
        use crate::lifecycle::{
            run_with_connection_token, take_task_connection_token, LifecycleState,
        };
        use crate::reload;
        use crate::server::connection_executor::CoreConnectionExecutor;
        use exyonq_mod_proxy::build_incoming_client;
        use std::sync::{Arc, Once};
        use tokio::net::TcpListener as TokioTcpListener;

        fn ensure_hooks_and_sentinel_enqueue() {
            static ONCE: Once = Once::new();
            ONCE.call_once(|| {
                exyonq_mod_static::install_kernel_hooks(Arc::new(
                    exyonq_mod_static::StaticRuntime::new(),
                ));
                exyonq_mod_proxy::install_kernel_hooks(Arc::new(
                    exyonq_mod_proxy::ProxyRuntime::new(),
                ));
                // Sentinel only: oversized path must not reach enqueue (real size check).
                register_epoll_keepalive_enqueue(|_fd, _peer, _buf, _len, _sf, _transfer| {
                    panic!("V044-LIFECYCLE-C1: enqueue must not run when head+carry > 512");
                });
                install_static_wire_hooks();
            });
        }

        async fn test_entry(ops: Arc<LifecycleState>) -> PlatformConnectionEntry {
            let raw = include_str!("../../../tests/fixtures/minimal.toml");
            let config: crate::config::AppConfig = raw.parse().expect("config");
            let proxy = build_incoming_client();
            let shared = reload::wrap_state(
                crate::server::state::ServerState::new(config, proxy.clone())
                    .await
                    .expect("state"),
            );
            PlatformConnectionEntry::from_executor(Arc::new(CoreConnectionExecutor::new(
                shared,
                proxy,
                ops,
                tokio::runtime::Handle::current(),
            )))
        }

        /// Real TCP sockets + real size gate: eligible oversized head must not release admission.
        #[tokio::test]
        async fn oversized_eligible_handoff_keeps_task_local_admission() {
            ensure_hooks_and_sentinel_enqueue();
            let ops = LifecycleState::new();
            let entry = test_entry(Arc::clone(&ops)).await;
            prepare_registered_keepalive_pool(1, Arc::new(entry));

            let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let client = tokio::net::TcpStream::connect(addr).await.unwrap();
            let (server, _) = listener.accept().await.unwrap();
            drop(client);

            // Eligible prefix + Cookie so total head exceeds the 512-byte handoff buffer.
            let mut head = b"GET /health HTTP/1.1\r\nHost: x\r\nCookie: ".to_vec();
            head.extend(std::iter::repeat_n(b'a', 480));
            head.extend_from_slice(b"\r\n\r\n");
            assert!(head.len() > 512);
            assert!(exyonq_module_api::static_wire::epoll_keep_alive_eligible(
                &head
            ));

            let token = ops.try_enter().unwrap();
            assert_eq!(ops.active_connections(), 1);
            run_with_connection_token(token, async {
                let result = register_keepalive_from_tokio_first(server, &head, &[]);
                assert!(result.is_err(), "oversized handoff must return the stream");
                assert_eq!(
                    ops.active_connections(),
                    1,
                    "admission must remain while Tokio still owns the connection"
                );
                assert!(
                    take_task_connection_token().is_some(),
                    "token must remain task-local after rejected handoff prep"
                );
                assert_eq!(ops.active_connections(), 0);
            })
            .await;
            assert_eq!(ops.active_connections(), 0);
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    install_static_wire_hooks, prepare_registered_keepalive_pool,
    signal_registered_keepalive_pool_stop, start_registered_epoll_listen,
    stop_registered_keepalive_pool,
};
#[cfg(target_os = "linux")]
pub use linux::{
    register_epoll_keepalive_enqueue, register_epoll_keepalive_prepare,
    register_epoll_keepalive_signal_stop, register_epoll_keepalive_stop,
    register_epoll_listen_start,
};
