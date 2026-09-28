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
//! Linux epoll static worker (PS3A-PM3-R2): accept + keepalive FSM.
//!
//! INTERNAL WORKSPACE PLATFORM CRATE — NOT STABLE PUBLIC API
//! Single productive implementation (moved from `exyonq-core::server::epoll_worker`).
//! F1: `attach_epoll_connection` · F2: `attach_epoll_keepalive_transfer`.

use crate::linux_bind::bind_tuned_std;
use bytes::Bytes;
use exyonq_core::kernel::{
    ConnectionServeOutcome, EpollAttachDecision, EpollAttachRejectReason,
    EpollConnectionAttachment, EpollKeepaliveTransfer, PlatformConnectionEntry,
};
use exyonq_core::server::OsWorkerGuard;
use exyonq_module_api::static_epoll::{self, StaticEpollPumpResult, StaticEpollSession};
use exyonq_module_api::static_wire;
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::warn;

const EPOLLIN: u32 = 0x001;
const EPOLLOUT: u32 = 0x004;
const EPOLLRDHUP: u32 = 0x2000;
const EPOLLET: u32 = 1 << 31;
const EPOLL_CTL_ADD: i32 = 1;
const EPOLL_CTL_DEL: i32 = 2;
const EPOLL_CTL_MOD: i32 = 3;

const MAX_EVENTS: usize = 128;
const MAX_HEADER: usize = 8192;
const ACCEPT_BATCH_LIMIT: usize = 64;
const ACCEPT_PEEK_LEN: usize = 128;
/// Kernel cold-path flag — local to epoll worker (not part of module public API).
const MSG_NOSIGNAL: i32 = 0x4000;

/// Terminal 503 for a sendfile registration that fails after the fd left the Tokio side
/// (async `EPOLL_CTL_ADD` failure). Best-effort write + close — never a silent drop.
const DRAINING_RESPONSE: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 8\r\nConnection: close\r\n\r\ndraining";

const SENDFILE_TERMINAL_503: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\nunavailable";
/// HEAD begin-reject: same Content-Length, zero body (HTTP/1.1 HEAD).
const SENDFILE_TERMINAL_503_HEAD: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\n";

fn interest_reading() -> u32 {
    static_epoll::interest_reading()
}

fn interest_sending_parked() -> u32 {
    static_epoll::interest_sending_parked()
}

struct KeepAliveRegister {
    fd: RawFd,
    carry: [u8; 512],
    carry_len: usize,
    /// ADR-025 PR #2: serve this connection with the non-blocking sendfile FSM (P2/P3).
    sendfile: bool,
    peer: SocketAddr,
    /// F2: opaque already-admitted transfer (consumed once into attachment).
    transfer: EpollKeepaliveTransfer,
}

static KEEPALIVE_POOL: OnceLock<Mutex<Option<EpollKeepAlivePoolRegister>>> = OnceLock::new();
static KEEPALIVE_POOL_OWNED: Mutex<Option<EpollKeepAlivePool>> = Mutex::new(None);
/// Single-flight guard for lazy pool creation (Bugbot #12).
static KEEPALIVE_INIT: Mutex<()> = Mutex::new(());

struct KeepalivePoolLazyConfig {
    threads: usize,
    entry: Arc<PlatformConnectionEntry>,
}

static KEEPALIVE_LAZY_CONFIG: OnceLock<KeepalivePoolLazyConfig> = OnceLock::new();

fn keepalive_pool_threads(requested: usize) -> usize {
    const MIN_THREADS: usize = 1;
    const MAX_THREADS: usize = 16;

    // Composition already supplies the resolved accept-worker count. Keep this
    // mechanism-local so platform does not import core worker policy.
    let accept_workers = requested.clamp(MIN_THREADS, MAX_THREADS);
    let desired = std::env::var("EXYONQ_EPOLL_POOL_THREADS")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|threads| *threads > 0)
        .map_or(accept_workers, |threads| {
            threads.clamp(MIN_THREADS, MAX_THREADS)
        });
    desired.min(accept_workers)
}

/// Register lazy-init params when `EXYONQ_EPOLL_STATIC=1` (pool starts on first P1 handoff).
pub fn prepare_keepalive_pool(workers: usize, entry: Arc<PlatformConnectionEntry>) {
    let threads = keepalive_pool_threads(workers);
    tracing::info!(
        accept_workers = workers,
        epoll_pool_threads = threads,
        "Cap067 keepalive pool geometry"
    );
    let _ = KEEPALIVE_LAZY_CONFIG.set(KeepalivePoolLazyConfig { threads, entry });
}

pub fn stop_keepalive_pool() {
    if let Ok(mut guard) = KEEPALIVE_POOL_OWNED.lock() {
        if let Some(pool) = guard.take() {
            pool.stop();
        }
    }
}

/// Cap041: signal keepalive workers to stop accepting new registrations (drain in-map first).
pub fn signal_keepalive_pool_stop() {
    if let Ok(guard) = KEEPALIVE_POOL_OWNED.lock() {
        if let Some(pool) = guard.as_ref() {
            pool.signal_stop();
        }
    }
}

fn keepalive_register_cached() -> Option<EpollKeepAlivePoolRegister> {
    let slot = KEEPALIVE_POOL.get()?;
    let guard = slot.lock().ok()?;
    guard.clone()
}

fn ensure_keepalive_pool() -> io::Result<EpollKeepAlivePoolRegister> {
    if let Some(reg) = keepalive_register_cached() {
        return Ok(reg);
    }

    let _init = KEEPALIVE_INIT
        .lock()
        .map_err(|_| io::Error::other("keep-alive init lock poisoned"))?;

    if let Some(reg) = keepalive_register_cached() {
        return Ok(reg);
    }

    let cfg = KEEPALIVE_LAZY_CONFIG.get().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "keep-alive pool not configured",
        )
    })?;
    let pool = start_keepalive_pool(cfg.threads, Arc::clone(&cfg.entry))?;
    let reg = pool.clone_for_register();

    match KEEPALIVE_POOL.get() {
        None => {
            let _ = KEEPALIVE_POOL.set(Mutex::new(Some(reg.clone())));
        }
        Some(slot) => {
            let mut guard = slot
                .lock()
                .map_err(|_| io::Error::other("keep-alive pool lock poisoned"))?;
            *guard = Some(reg.clone());
        }
    }
    if let Ok(mut owned) = KEEPALIVE_POOL_OWNED.lock() {
        if owned.is_none() {
            *owned = Some(pool);
        } else {
            pool.stop();
        }
    } else {
        pool.stop();
    }

    keepalive_register_cached().ok_or_else(|| io::Error::other("keep-alive pool init failed"))
}

/// Tokio accept + epoll keep-alive for P1/P7/health (`EXYONQ_EPOLL_STATIC=1`, lazy start).
fn start_keepalive_pool(
    workers: usize,
    entry: Arc<PlatformConnectionEntry>,
) -> io::Result<EpollKeepAlivePool> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let abort = Arc::new(AtomicBool::new(false));
    let mut register_txs = Vec::with_capacity(workers);
    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let (tx, rx) = mpsc::sync_channel(256);
        register_txs.push(tx);
        let entry = Arc::clone(&entry);
        let shutdown = Arc::clone(&shutdown);
        let abort = Arc::clone(&abort);
        handles.push(thread::spawn(move || {
            if let Err(err) = keepalive_worker_loop(rx, entry, shutdown, abort) {
                warn!(%err, "epoll keep-alive worker stopped");
            }
        }));
    }

    let pool = EpollKeepAlivePool {
        shutdown,
        abort,
        handles,
        register_txs,
    };
    Ok(pool)
}

#[derive(Clone)]
struct EpollKeepAlivePoolRegister {
    shutdown: Arc<AtomicBool>,
    register_txs: Vec<SyncSender<KeepAliveRegister>>,
}

pub(crate) struct EpollKeepAlivePool {
    shutdown: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
    register_txs: Vec<SyncSender<KeepAliveRegister>>,
}

impl EpollKeepAlivePool {
    fn clone_for_register(&self) -> EpollKeepAlivePoolRegister {
        EpollKeepAlivePoolRegister {
            shutdown: Arc::clone(&self.shutdown),
            register_txs: self.register_txs.clone(),
        }
    }

    /// Cap041: stop new keepalive registrations; drain in-map conns until empty or [`Self::stop`].
    pub fn signal_stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    pub fn stop(self) {
        self.abort.store(true, Ordering::SeqCst);
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(slot) = KEEPALIVE_POOL.get() {
            if let Ok(mut guard) = slot.lock() {
                *guard = None;
            }
        }
        for handle in self.handles {
            let _ = handle.join();
        }
    }
}

impl EpollKeepAlivePoolRegister {
    /// On failure, returns the job so admission ownership can be restored (V044-LIFECYCLE-C1).
    /// Large `Err` is intentional: the register job must come back by value (no Box).
    #[allow(clippy::result_large_err)]
    fn register(&self, job: KeepAliveRegister) -> Result<(), (io::Error, KeepAliveRegister)> {
        if self.shutdown.load(Ordering::Relaxed) {
            return Err((
                io::Error::new(io::ErrorKind::NotConnected, "pool stopped"),
                job,
            ));
        }
        let worker = (job.fd as usize) % self.register_txs.len();
        match self.register_txs[worker].send(job) {
            Ok(()) => Ok(()),
            Err(mpsc::SendError(job)) => Err((
                io::Error::new(io::ErrorKind::WouldBlock, "keep-alive pool full"),
                job,
            )),
        }
    }
}

/// F2 consume path: enqueue an already-admitted keepalive transfer.
///
/// On `Ok`, `fd` ownership transfers to the keep-alive pool.
/// On `Err`, `fd` remains with the caller and `EpollKeepaliveTransfer` is returned for restore.
pub fn enqueue_keepalive_transfer(
    fd: RawFd,
    peer: SocketAddr,
    carry: [u8; 512],
    carry_len: usize,
    sendfile: bool,
    transfer: EpollKeepaliveTransfer,
) -> Result<(), (io::Error, EpollKeepaliveTransfer)> {
    let register = match ensure_keepalive_pool() {
        Ok(reg) => reg,
        Err(err) => return Err((err, transfer)),
    };
    // SAFETY: `fd` is a live socket RawFd handed to keepalive registration; F_GETFL/F_SETFL
    // only toggle O_NONBLOCK on that descriptor. Negative F_GETFL is ignored (best-effort).
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            let _ = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
    match register.register(KeepAliveRegister {
        fd,
        carry,
        carry_len,
        sendfile,
        peer,
        transfer,
    }) {
        Ok(()) => Ok(()),
        Err((err, job)) => Err((err, job.transfer)),
    }
}

/// Productive epoll-listen starter registered at composition root.
pub fn start_epoll_listen_workers(
    listen: SocketAddr,
    workers: usize,
    entry: Arc<PlatformConnectionEntry>,
) -> io::Result<OsWorkerGuard> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let abort = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(workers);
    let mut listeners = Vec::with_capacity(workers);

    for _ in 0..workers {
        let listener = bind_tuned_std(listen)?;
        listener.set_nonblocking(true)?;
        listeners.push(listener.try_clone()?);

        let entry = Arc::clone(&entry);
        let shutdown = Arc::clone(&shutdown);
        let abort = Arc::clone(&abort);

        handles.push(thread::spawn(move || {
            if let Err(err) = epoll_loop(listener, entry, shutdown, abort) {
                warn!(%err, "epoll static worker stopped");
            }
        }));
    }

    Ok(OsWorkerGuard::new_with_abort(
        shutdown, abort, handles, listeners,
    ))
}

fn keepalive_worker_loop(
    register_rx: mpsc::Receiver<KeepAliveRegister>,
    entry: Arc<PlatformConnectionEntry>,
    shutdown: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
) -> io::Result<()> {
    // SAFETY: `epoll_create1(EPOLL_CLOEXEC)` allocates a fresh epoll instance; no prior fd
    // ownership. Failure is checked via `epfd < 0` before any use.
    let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if epfd < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut conns: HashMap<RawFd, ConnState> = HashMap::new();
    let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS];
    // A full epoll batch means more sockets are already readable. Do not sleep
    // before draining them; only the empty ready-list blocks.
    let mut wait_ms: i32 = 100;

    // Cap041: after shutdown, keep serving in-map conns until empty (or abort).
    loop {
        let shutting_down = shutdown.load(Ordering::Relaxed);
        if abort.load(Ordering::Relaxed) {
            break;
        }
        if shutting_down && conns.is_empty() {
            break;
        }

        if !shutting_down {
            while let Ok(reg) = register_rx.try_recv() {
                if let Err(err) = add_registered_conn(epfd, &mut conns, reg, &entry) {
                    warn!(%err, "epoll keep-alive register failed");
                }
            }
        } else {
            // Drain any late registrations without accepting ownership after stop signal.
            while register_rx.try_recv().is_ok() {}
        }

        expire_stale_header_reads(epfd, &mut conns);

        // SAFETY: `epfd` is this worker's epoll fd; `events` is a stack array of `MAX_EVENTS`
        // entries; kernel writes at most that many. `n` is bounds-checked before indexing.
        let n = unsafe { libc::epoll_wait(epfd, events.as_mut_ptr(), MAX_EVENTS as i32, wait_ms) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if shutting_down || abort.load(Ordering::Relaxed) {
                break;
            }
            return Err(err);
        }
        wait_ms = if n == MAX_EVENTS as i32 { 0 } else { 100 };
        for ev in &events[..n as usize] {
            let fd = ev.u64 as RawFd;
            if ev.events & (EPOLLIN | EPOLLRDHUP | EPOLLOUT) != 0 {
                // IU2 / connection-local: never kill the keepalive worker loop.
                let _ = drain_epoll_fd(epfd, fd, &mut conns, &entry);
            }
        }
        expire_stale_header_reads(epfd, &mut conns);
    }
    Ok(())
}

/// Platform-local process-lifetime cache for `EXYONQ_READ_TIMEOUT_MS`.
static HEADER_READ_TIMEOUT_MS: AtomicU64 = AtomicU64::new(0);
static HEADER_READ_TIMEOUT_INITIALIZED: AtomicBool = AtomicBool::new(false);

fn parse_header_read_timeout_ms(raw: Option<&str>) -> Duration {
    raw.and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(30))
}

/// `EXYONQ_READ_TIMEOUT_MS` → deadline span.
///
/// This platform path owns a layer-local process-lifetime observation. Core and
/// modules retain equivalent caches for their own paths. No `getenv` occurs per request.
fn header_read_deadline_duration() -> Duration {
    if !HEADER_READ_TIMEOUT_INITIALIZED.load(Ordering::Acquire) {
        let timeout =
            parse_header_read_timeout_ms(std::env::var("EXYONQ_READ_TIMEOUT_MS").ok().as_deref());
        HEADER_READ_TIMEOUT_MS.store(timeout.as_millis() as u64, Ordering::Relaxed);
        HEADER_READ_TIMEOUT_INITIALIZED.store(true, Ordering::Release);
    }
    Duration::from_millis(HEADER_READ_TIMEOUT_MS.load(Ordering::Relaxed))
}

#[cfg(test)]
fn reset_header_read_deadline_cache_for_tests() {
    HEADER_READ_TIMEOUT_MS.store(0, Ordering::Relaxed);
    HEADER_READ_TIMEOUT_INITIALIZED.store(false, Ordering::Release);
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    // Local copy of core `server::io::find_header_end` (D1: no broad server::io import).
    let mut i = 0;
    while i + 3 < buf.len() {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn header_line_name_eq(line: &[u8], name_lower: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|&b| b == b':') else {
        return false;
    };
    let mut name = &line[..colon];
    while name.first() == Some(&b' ') || name.first() == Some(&b'\t') {
        name = &name[1..];
    }
    while name.last() == Some(&b' ') || name.last() == Some(&b'\t') {
        name = &name[..name.len() - 1];
    }
    name.len() == name_lower.len() && name.eq_ignore_ascii_case(name_lower)
}

fn header_field_name_count(head: &[u8], name_lower: &[u8]) -> usize {
    let mut count = 0usize;
    let mut start = 0usize;
    if let Some(i) = head.windows(2).position(|w| w == b"\r\n") {
        start = i + 2;
    }
    while start < head.len() {
        if start + 1 < head.len() && head[start] == b'\r' && head[start + 1] == b'\n' {
            break;
        }
        let rel = head[start..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .unwrap_or(head.len() - start);
        let line = &head[start..start + rel];
        if header_line_name_eq(line, name_lower) {
            count += 1;
        }
        start += rel + 2;
    }
    count
}

/// Mirror of core `request_headers_safe_for_wire` (D1: no broad server::io import).
fn request_headers_safe_for_wire(head: &[u8]) -> bool {
    let cl = header_field_name_count(head, b"content-length");
    let te = header_field_name_count(head, b"transfer-encoding");
    if cl > 1 {
        return false;
    }
    if te >= 1 {
        return false;
    }
    true
}

fn content_length_bytes(head: &[u8]) -> Option<usize> {
    let mut start = 0usize;
    if let Some(i) = head.windows(2).position(|w| w == b"\r\n") {
        start = i + 2;
    }
    let mut found: Option<usize> = None;
    while start < head.len() {
        if start + 1 < head.len() && head[start] == b'\r' && head[start + 1] == b'\n' {
            break;
        }
        let rel = head[start..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .unwrap_or(head.len() - start);
        let line = &head[start..start + rel];
        if header_line_name_eq(line, b"content-length") {
            let colon = line.iter().position(|&b| b == b':')?;
            let mut v = &line[colon + 1..];
            while v.first() == Some(&b' ') || v.first() == Some(&b'\t') {
                v = &v[1..];
            }
            let Ok(n) = std::str::from_utf8(v).ok()?.parse::<usize>() else {
                return None;
            };
            if found.is_some() {
                return None;
            }
            found = Some(n);
        }
        start += rel + 2;
    }
    found.or(Some(0))
}

fn write_all_fd(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        // SAFETY: `fd` is an open socket owned by the epoll worker; `buf` is a valid Rust
        // slice for the duration of the call. Partial writes advance the slice.
        let written = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0"));
        }
        buf = &buf[written as usize..];
    }
    Ok(())
}

fn write_response_fd_local(fd: RawFd, buf: &[u8]) -> io::Result<()> {
    if buf.len() <= 16 * 1024 {
        // SAFETY: same as `write_all_fd` — open worker-owned socket + valid slice.
        let written = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        if written as usize != buf.len() {
            return write_all_fd(fd, &buf[written as usize..]);
        }
        Ok(())
    } else {
        write_all_fd(fd, buf)
    }
}

fn renew_read_deadline(state: &mut ConnState) {
    state.read_deadline = Instant::now() + header_read_deadline_duration();
}

/// Wall-clock guard for slowloris: only while `Reading` with incomplete headers.
fn conn_header_read_expired(state: &ConnState, now: Instant) -> bool {
    matches!(state.phase, ConnPhase::Reading)
        && find_header_end(&state.buf[..state.len]).is_none()
        && now >= state.read_deadline
}

/// O(n) over live epoll conns; n is bounded by pool/worker load (typically small).
fn expire_stale_header_reads(epfd: RawFd, conns: &mut HashMap<RawFd, ConnState>) {
    let now = Instant::now();
    let stale: Vec<RawFd> = conns
        .iter()
        .filter_map(|(&fd, state)| conn_header_read_expired(state, now).then_some(fd))
        .collect();
    for fd in stale {
        remove_conn(epfd, fd, conns);
    }
}

fn add_registered_conn(
    epfd: RawFd,
    conns: &mut HashMap<RawFd, ConnState>,
    reg: KeepAliveRegister,
    entry: &PlatformConnectionEntry,
) -> io::Result<()> {
    let attachment = entry.attach_epoll_keepalive_transfer(reg.transfer);
    let mut state = ConnState::new(reg.carry_len, reg.sendfile, attachment, reg.peer);
    state.buf[..reg.carry_len].copy_from_slice(&reg.carry[..reg.carry_len]);

    let mut ev = libc::epoll_event {
        events: conn_epoll_events(),
        u64: reg.fd as u64,
    };
    // SAFETY: `epfd` is this worker's epoll fd; `reg.fd` is a transferred socket not yet in
    // `conns`; `ev` is stack-local and valid for the syscall duration.
    if unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_ADD, reg.fd, &mut ev) } < 0 {
        write_terminal_503(reg.fd);
        // SAFETY: ADD failed so fd is not in the epoll set; we still own `reg.fd` and must
        // close it — it will not be inserted into `conns`.
        unsafe {
            libc::close(reg.fd);
        }
        return Err(io::Error::last_os_error());
    }

    let ready = find_header_end(&state.buf[..state.len]).is_some();
    let sendfile = reg.sendfile;
    let fd = reg.fd;
    conns.insert(fd, state);

    // A2: first response on epoll when the request is already in the buffer.
    // Non-sendfile must go through `drain_epoll_fd` so `ServeResult::Handoff`
    // reaches Hyper (bare `serve_epoll_fd` dropped Handoff → idle hang).
    if ready {
        if sendfile {
            drive_sendfile_fd(epfd, fd, conns);
        } else {
            let _ = drain_epoll_fd(epfd, fd, conns, entry);
        }
    }
    Ok(())
}

struct ConnState {
    buf: [u8; MAX_HEADER],
    len: usize,
    /// Wall-clock deadline for completing the current request headers (`Reading` only).
    read_deadline: Instant,
    /// ADR-025 PR #2: `Sending` only while a gated sendfile body is in flight (lazy interest).
    /// In-memory wire conns (health) always stay `Reading` — unchanged behavior.
    phase: ConnPhase,
    /// True only for gated sendfile conns; routes events to the FSM, not `serve_epoll_fd`.
    sendfile: bool,
    peer: SocketAddr,
    /// F1/F2 opaque attachment (owns lifecycle token).
    attachment: EpollConnectionAttachment,
    /// Small-file response not yet fully written. The worker must not block in `write`.
    pending_out: Option<PendingOut>,
}

impl ConnState {
    fn new(
        carry_len: usize,
        sendfile: bool,
        attachment: EpollConnectionAttachment,
        peer: SocketAddr,
    ) -> Self {
        let now = Instant::now();
        ConnState {
            buf: [0u8; MAX_HEADER],
            len: carry_len,
            read_deadline: now + header_read_deadline_duration(),
            phase: ConnPhase::Reading,
            sendfile,
            peer,
            attachment,
            pending_out: None,
        }
    }
}

/// Send-side phase for gated sendfile conns. `SendingState` is inline (no per-request heap).
enum ConnPhase {
    Reading,
    Sending(StaticEpollSession),
}

fn conn_epoll_events() -> u32 {
    EPOLLIN | EPOLLRDHUP | EPOLLET
}

/// `EPOLL_CTL_MOD` the connection's interest mask (lazy Reading↔Sending switch).
fn epoll_mod_interest(epfd: RawFd, fd: RawFd, interest: u32) -> io::Result<()> {
    let mut ev = libc::epoll_event {
        events: interest,
        u64: fd as u64,
    };
    if unsafe {
        // SAFETY: `fd` is in the epoll set; `ev` carries the new interest mask only.
        libc::epoll_ctl(epfd, EPOLL_CTL_MOD, fd, &mut ev)
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Best-effort terminal 503 on a sendfile fd whose epoll registration failed after the
/// Tokio side released it. **Non-blocking** single send — the keep-alive worker thread also
/// services other connections, so this cold (ENOMEM-class) path must never block it. The fd
/// is already non-blocking here; a partial/dropped 503 is acceptable since the caller closes
/// the fd immediately after. Avoids a silent drop without head-of-line blocking.
fn write_terminal_503(fd: RawFd) {
    let _ = unsafe {
        // SAFETY: `fd` is a valid open socket; `send` with MSG_NOSIGNAL is best-effort on cold path.
        libc::send(
            fd,
            SENDFILE_TERMINAL_503.as_ptr() as *const libc::c_void,
            SENDFILE_TERMINAL_503.len(),
            MSG_NOSIGNAL,
        )
    };
}

fn epoll_loop(
    listener: TcpListener,
    entry: Arc<PlatformConnectionEntry>,
    shutdown: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
) -> io::Result<()> {
    let listen_fd = listener.as_raw_fd();
    // SAFETY: fresh epoll instance for this listen worker; result checked for < 0.
    let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if epfd < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut ev = libc::epoll_event {
        events: EPOLLIN | EPOLLET,
        u64: listen_fd as u64,
    };
    // SAFETY: `listen_fd` is borrowed from `listener` (still owned by this thread); ADD
    // registers interest only — drop of `listener` later closes the fd after loop exit.
    if unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_ADD, listen_fd, &mut ev) } < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut conns: HashMap<RawFd, ConnState> = HashMap::new();
    let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS];
    let mut listen_detached = false;

    // Cap041: after shutdown, stop accepts but keep serving in-map conns until empty (or abort).
    loop {
        let shutting_down = shutdown.load(Ordering::Relaxed);
        if abort.load(Ordering::Relaxed) {
            break;
        }
        if shutting_down && conns.is_empty() {
            break;
        }

        if shutting_down && !listen_detached {
            // SAFETY: `listen_fd` was ADDed above; DEL with null event is valid. Listener
            // TcpListener still owns the fd until function return.
            let _ =
                unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_DEL, listen_fd, std::ptr::null_mut()) };
            listen_detached = true;
        }

        expire_stale_header_reads(epfd, &mut conns);

        // SAFETY: `epfd` owned by this loop; `events` stack capacity is `MAX_EVENTS`.
        let n = unsafe { libc::epoll_wait(epfd, events.as_mut_ptr(), MAX_EVENTS as i32, 500) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if shutting_down || abort.load(Ordering::Relaxed) {
                break;
            }
            return Err(err);
        }
        for ev in &events[..n as usize] {
            let fd = ev.u64 as RawFd;
            if fd == listen_fd {
                if shutting_down {
                    continue;
                }
                // Drain listen backlog under EPOLLET: retry while a full batch is taken
                // so ACCEPT_BATCH_LIMIT cannot park the backlog until the next edge.
                loop {
                    let n = accept_batch(&listener, epfd, &mut conns, &entry)?;
                    if n < ACCEPT_BATCH_LIMIT {
                        break;
                    }
                }
                continue;
            }
            if ev.events & (EPOLLIN | EPOLLRDHUP | EPOLLOUT) != 0 {
                // IU2 / connection-local: never kill the epoll worker loop.
                let _ = drain_epoll_fd(epfd, fd, &mut conns, &entry);
            }
        }
        expire_stale_header_reads(epfd, &mut conns);
    }
    Ok(())
}

/// EPOLLET: read/process until EAGAIN or handoff.
fn drain_epoll_fd(
    epfd: RawFd,
    fd: RawFd,
    conns: &mut HashMap<RawFd, ConnState>,
    entry: &PlatformConnectionEntry,
) -> io::Result<()> {
    let _ = entry; // identity reserved for future mechanism diagnostics
                   // ADR-025 PR #2: gated sendfile conns are driven by the non-blocking FSM, never Handoff.
    if conns.get(&fd).map(|c| c.sendfile).unwrap_or(false) {
        drive_sendfile_fd(epfd, fd, conns);
        return Ok(());
    }
    loop {
        match serve_epoll_fd(fd, conns.get_mut(&fd)) {
            Ok(ServeResult::Idle) => break,
            Ok(ServeResult::Continue) => continue,
            Ok(ServeResult::Handoff { head, rest }) => {
                let Some((attachment, peer)) = detach_conn_for_handoff(epfd, fd, conns) else {
                    break;
                };
                // SAFETY: `detach_conn_for_handoff` removed `fd` from epoll+`conns` without
                // closing; sole ownership transfers into `TcpStream` for Hyper handoff.
                let stream = unsafe { TcpStream::from_raw_fd(fd) };
                match attachment.handoff_to_hyper(stream, peer, Some((head, rest))) {
                    ConnectionServeOutcome::Completed
                    | ConnectionServeOutcome::ConnectionClosed(_)
                    | ConnectionServeOutcome::PolicyFailed(_) => {}
                }
                break;
            }
            Err(_) => {
                remove_conn(epfd, fd, conns);
                break;
            }
        }
    }
    Ok(())
}

enum ServeResult {
    /// Processed at least one request; more may be buffered or readable.
    Continue,
    /// No progress; socket would block (EPOLLET drain done).
    Idle,
    Handoff {
        head: Bytes,
        rest: Bytes,
    },
}

fn set_fd_nonblocking(fd: RawFd, nonblocking: bool) -> io::Result<()> {
    // SAFETY: `fd` is an open epoll-map socket; F_GETFL reads flags only.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let new_flags = if nonblocking {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
    // SAFETY: F_SETFL applies only O_NONBLOCK bit changes derived from F_GETFL on the same fd.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, new_flags) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Nonblocking epoll fds: block briefly so `write_response_fd` completes the full wire.
fn write_response_epoll_fd(fd: RawFd, buf: &[u8]) -> io::Result<()> {
    set_fd_nonblocking(fd, false)?;
    let result = write_response_fd_local(fd, buf);
    let _ = set_fd_nonblocking(fd, true);
    result
}

fn inline_wire_with_request_id(wire: bytes::Bytes, head: &[u8], started: Instant) -> bytes::Bytes {
    if !exyonq_module_api::static_wire::access_notices_enabled() {
        return wire;
    }
    let status = response_status_from_wire(&wire).unwrap_or(200);
    let Some(id) = exyonq_module_api::static_wire::notify_access(
        exyonq_module_api::static_wire::StaticWireAccessNotice {
            head: bytes::Bytes::copy_from_slice(head),
            status,
            outcome: "static",
            duration_ms: started.elapsed().as_millis() as u64,
        },
    ) else {
        return wire;
    };
    if wire
        .windows(14)
        .any(|window| window.eq_ignore_ascii_case(b"x-request-id:"))
    {
        return wire;
    }
    let Some(end) = wire.windows(4).position(|window| window == b"\r\n\r\n") else {
        return wire;
    };
    let mut out = Vec::with_capacity(wire.len() + id.len() + 16);
    out.extend_from_slice(&wire[..end]);
    out.extend_from_slice(b"\r\nx-request-id: ");
    out.extend_from_slice(id.as_bytes());
    out.extend_from_slice(&wire[end..]);
    bytes::Bytes::from(out)
}

fn response_status_from_wire(response: &[u8]) -> Option<u16> {
    let line_end = response
        .windows(2)
        .position(|pair| pair == b"\r\n")
        .unwrap_or(response.len());
    let line = std::str::from_utf8(&response[..line_end]).ok()?;
    line.split_ascii_whitespace().nth(1)?.parse().ok()
}

fn write_terminal_response_epoll_fd(fd: RawFd, response: &[u8]) -> io::Result<()> {
    write_response_epoll_fd(fd, response)?;
    if let Some(status) = response_status_from_wire(response) {
        static_epoll::record_product_response(status);
    }
    Ok(())
}

struct PeerIp {
    bytes: [u8; 64],
    len: usize,
}

impl PeerIp {
    fn from_ip(ip: std::net::IpAddr) -> Self {
        let mut this = Self {
            bytes: [0; 64],
            len: 0,
        };
        use std::fmt::Write;
        let _ = write!(this, "{}", ip.to_canonical());
        this
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("0.0.0.0")
    }
}

impl std::fmt::Write for PeerIp {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let raw = text.as_bytes();
        if self.len + raw.len() > self.bytes.len() {
            return Err(std::fmt::Error);
        }
        self.bytes[self.len..self.len + raw.len()].copy_from_slice(raw);
        self.len += raw.len();
        Ok(())
    }
}

/// Admit only after Cap067 has committed to owning this request.
///
/// `Ok(false)` means a delivered 429 terminal; callers must close/remove the fd.
fn admit_owned_product_request(fd: RawFd, peer: SocketAddr) -> io::Result<bool> {
    if !static_epoll::product_admit_active() {
        return Ok(true);
    }
    let ip = PeerIp::from_ip(peer.ip());
    match static_epoll::admit_product_request(ip.as_str()) {
        Ok(()) => Ok(true),
        Err(retry_after_secs) => {
            let response = static_epoll::rate_limit_reject_response(retry_after_secs);
            write_response_epoll_fd(fd, &response)?;
            static_epoll::record_product_response(429);
            Ok(false)
        }
    }
}

fn serve_epoll_fd(fd: RawFd, state: Option<&mut ConnState>) -> Result<ServeResult, io::Error> {
    let Some(state) = state else {
        return Err(io::Error::from(io::ErrorKind::NotFound));
    };

    // A2 tokio handoff may have already buffered complete headers — read only if needed.
    if find_header_end(&state.buf[..state.len]).is_none() {
        let mut tmp = [0u8; 256];
        let n = unsafe {
            // SAFETY: `tmp` is a valid stack buffer; `read` on a non-blocking epoll-owned fd.
            libc::read(fd, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len())
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                return Ok(ServeResult::Idle);
            }
            return Err(err);
        }
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        let n = n as usize;
        if state.len + n > state.buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        state.buf[state.len..state.len + n].copy_from_slice(&tmp[..n]);
        state.len += n;
    }

    let mut served = false;
    loop {
        let Some(end) = find_header_end(&state.buf[..state.len]) else {
            if state.len >= MAX_HEADER {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "request headers too large",
                ));
            }
            return if served {
                Ok(ServeResult::Continue)
            } else {
                Ok(ServeResult::Idle)
            };
        };
        let hdr_end = end + 4;
        let head = &state.buf[..hdr_end];

        if state.attachment.is_generation_stale() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reload generation mismatch",
            ));
        }

        // Cap040 / SECINT-003: already-admitted epoll keepalive must reject new product
        // work after drain (probe-aware boundary).
        if state.attachment.is_draining() {
            let resp = state.attachment.drain_boundary_response(head);
            let _ = write_response_epoll_fd(fd, resp);
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "draining"));
        }

        if !request_headers_safe_for_wire(head) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ambiguous request headers",
            ));
        }

        // WAF-KEEPALIVE-001 / Cap015: every request on the keepalive map must re-enter WAF.
        if let Some(reject) = state.attachment.evaluate_wire_waf_reject(head, state.peer) {
            if let Err(err) = write_response_epoll_fd(fd, &reject) {
                warn!(%err, fd, peer = %state.peer, "waf reject response write failed");
            }
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "waf reject",
            ));
        }

        if let Some(site_slot) = state.attachment.static_site_slot() {
            match static_epoll::try_write_inline_wire_response(
                site_slot,
                fd,
                head,
                write_response_epoll_fd,
            ) {
                static_epoll::StaticEpollInlineWireResult::Written => {}
                static_epoll::StaticEpollInlineWireResult::Handoff => {
                    let head_b = Bytes::copy_from_slice(head);
                    let rest_b = Bytes::copy_from_slice(&state.buf[hdr_end..state.len]);
                    return Ok(ServeResult::Handoff {
                        head: head_b,
                        rest: rest_b,
                    });
                }
                static_epoll::StaticEpollInlineWireResult::NoMatch => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "inline wire write failed",
                    ));
                }
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no static site",
            ));
        }

        served = true;
        let body_len = match content_length_bytes(head) {
            Some(n) => n,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid content-length",
                ));
            }
        };
        let consume_end = hdr_end.saturating_add(body_len);
        if consume_end > state.len {
            // Body not fully buffered — fail closed rather than desync on next request.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete request body on wire keepalive",
            ));
        }
        if consume_end >= state.len {
            state.len = 0;
            renew_read_deadline(state);
            return Ok(ServeResult::Continue);
        }
        state.buf.copy_within(consume_end..state.len, 0);
        state.len -= consume_end;
        renew_read_deadline(state);
    }
}

/// Result of one read/serve step for a gated sendfile connection.
enum ReadServe {
    /// Socket would block / header incomplete — stay `Reading`, wait for next `EPOLLIN`.
    Idle,
    /// A body parked on `EAGAIN`; interest moved to `EPOLLOUT` — stop the drive loop.
    StartedSending,
    /// Served a complete response (head-only or 0-body); try the next buffered request.
    Continue,
    /// Connection closed/removed.
    Closed,
    /// Cap067: sendfile match miss before bytes committed — Hyper fallback (NOT_STARTED).
    NeedHyper {
        head: bytes::Bytes,
        rest: bytes::Bytes,
    },
}

/// In-flight small-file response. The worker stays non-blocking and serves other fds.
struct PendingOut {
    bytes: bytes::Bytes,
    offset: usize,
    close_after: bool,
}

enum InlineFlush {
    /// Nothing was queued.
    Idle,
    /// The response bytes are on the socket.
    Done,
    /// `EPOLLOUT` is armed; try again on the next writable event.
    Parked,
    /// The connection was removed.
    Closed,
}

fn send_nonblocking(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    loop {
        let n = unsafe {
            // SAFETY: `fd` is a worker-owned non-blocking socket; `buf` is a valid slice.
            libc::send(
                fd,
                buf.as_ptr() as *const libc::c_void,
                buf.len(),
                MSG_NOSIGNAL,
            )
        };
        if n >= 0 {
            return Ok(n as usize);
        }
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(err);
    }
}

fn queue_inline_wire(
    epfd: RawFd,
    fd: RawFd,
    conns: &mut HashMap<RawFd, ConnState>,
    wire: bytes::Bytes,
    close_after: bool,
    head: &[u8],
    started: Instant,
) -> InlineFlush {
    let wire = inline_wire_with_request_id(wire, head, started);
    let mut offset = 0usize;
    loop {
        if offset >= wire.len() {
            if let Some(status) = response_status_from_wire(&wire) {
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                static_epoll::record_product_exchange(head, status, elapsed_ms);
            }
            if close_after {
                remove_conn(epfd, fd, conns);
                return InlineFlush::Closed;
            }
            return InlineFlush::Done;
        }
        match send_nonblocking(fd, &wire[offset..]) {
            Ok(0) => {
                remove_conn(epfd, fd, conns);
                return InlineFlush::Closed;
            }
            Ok(written) => offset += written,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                let Some(conn) = conns.get_mut(&fd) else {
                    return InlineFlush::Closed;
                };
                conn.pending_out = Some(PendingOut {
                    bytes: wire,
                    offset,
                    close_after,
                });
                if epoll_mod_interest(epfd, fd, conn_epoll_events() | EPOLLOUT).is_err() {
                    remove_conn(epfd, fd, conns);
                    return InlineFlush::Closed;
                }
                return InlineFlush::Parked;
            }
            Err(err) => {
                warn!(%err, fd, "inline response write failed");
                remove_conn(epfd, fd, conns);
                return InlineFlush::Closed;
            }
        }
    }
}

fn flush_pending_inline(
    epfd: RawFd,
    fd: RawFd,
    conns: &mut HashMap<RawFd, ConnState>,
) -> InlineFlush {
    loop {
        enum Step {
            Wrote,
            Block,
            Fail,
            Finished,
        }
        let step = {
            let Some(conn) = conns.get_mut(&fd) else {
                return InlineFlush::Closed;
            };
            let Some(pending) = conn.pending_out.as_mut() else {
                return InlineFlush::Idle;
            };
            if pending.offset >= pending.bytes.len() {
                Step::Finished
            } else {
                match send_nonblocking(fd, &pending.bytes[pending.offset..]) {
                    Ok(0) => Step::Fail,
                    Ok(written) => {
                        pending.offset += written;
                        Step::Wrote
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => Step::Block,
                    Err(err) => {
                        warn!(%err, fd, "inline response resume failed");
                        Step::Fail
                    }
                }
            }
        };
        match step {
            Step::Wrote => continue,
            Step::Finished => break,
            Step::Block => {
                if epoll_mod_interest(epfd, fd, conn_epoll_events() | EPOLLOUT).is_err() {
                    remove_conn(epfd, fd, conns);
                    return InlineFlush::Closed;
                }
                return InlineFlush::Parked;
            }
            Step::Fail => {
                remove_conn(epfd, fd, conns);
                return InlineFlush::Closed;
            }
        }
    }
    let (status, close_after) = {
        let Some(conn) = conns.get_mut(&fd) else {
            return InlineFlush::Closed;
        };
        let pending = conn.pending_out.take();
        renew_read_deadline(conn);
        match pending {
            Some(pending) => (
                response_status_from_wire(&pending.bytes),
                pending.close_after,
            ),
            None => return InlineFlush::Idle,
        }
    };
    if let Some(status) = status {
        static_epoll::record_product_response(status);
    }
    if close_after {
        remove_conn(epfd, fd, conns);
        return InlineFlush::Closed;
    }
    if epoll_mod_interest(epfd, fd, conn_epoll_events()).is_err() {
        remove_conn(epfd, fd, conns);
        return InlineFlush::Closed;
    }
    InlineFlush::Done
}

/// Drive a gated sendfile connection with the non-blocking FSM.
///
/// Lazy interest: stays armed `EPOLLIN` while reading and only switches to `EPOLLOUT` when a
/// body parks (`PumpResult::Parked`), restoring `EPOLLIN` on resume-`Complete`. The common
/// case (healthy socket) completes in the first drain with **0** `EPOLL_CTL_MOD` per request.
/// `Parked` stops the loop immediately and is never mapped to a retry (PR #1 contract).
fn drive_sendfile_fd(epfd: RawFd, fd: RawFd, conns: &mut HashMap<RawFd, ConnState>) {
    loop {
        match flush_pending_inline(epfd, fd, conns) {
            InlineFlush::Idle => {}
            InlineFlush::Done => {}
            InlineFlush::Parked | InlineFlush::Closed => return,
        }
        let sending_phase = match conns.get(&fd) {
            None => {
                // Event raced with removal — non-fatal, never close here (C1).
                static_epoll::note_unknown_fd_event();
                return;
            }
            Some(c) => matches!(c.phase, ConnPhase::Sending(_)),
        };

        if sending_phase {
            let session = {
                let Some(c) = conns.get_mut(&fd) else {
                    return;
                };
                let ConnPhase::Sending(session) = c.phase else {
                    return;
                };
                session
            };
            match static_epoll::pump_sendfile_session(fd, session) {
                StaticEpollPumpResult::Parked => {
                    static_epoll::note_parked();
                    return;
                }
                StaticEpollPumpResult::Complete => {
                    static_epoll::note_complete();
                    if let Some(c) = conns.get_mut(&fd) {
                        static_epoll::clear_sendfile_session(fd, session);
                        c.phase = ConnPhase::Reading;
                        renew_read_deadline(c);
                    }
                    if epoll_mod_interest(epfd, fd, interest_reading()).is_err() {
                        remove_conn(epfd, fd, conns);
                        return;
                    }
                    continue;
                }
                StaticEpollPumpResult::Error => {
                    static_epoll::note_error();
                    static_epoll::clear_sendfile_session(fd, session);
                    remove_conn(epfd, fd, conns);
                    return;
                }
                StaticEpollPumpResult::Progress => return,
            }
        }

        match sendfile_read_and_serve(epfd, fd, conns) {
            ReadServe::Idle | ReadServe::StartedSending | ReadServe::Closed => return,
            ReadServe::Continue => continue,
            ReadServe::NeedHyper { head, rest } => {
                let Some((attachment, peer)) = detach_conn_for_handoff(epfd, fd, conns) else {
                    return;
                };
                // SAFETY: after `detach_conn_for_handoff`, `fd` is not in epoll/`conns` and
                // was not closed; `TcpStream` becomes the unique owner for Hyper handoff.
                let stream = unsafe { TcpStream::from_raw_fd(fd) };
                match attachment.handoff_to_hyper(stream, peer, Some((head, rest))) {
                    ConnectionServeOutcome::Completed
                    | ConnectionServeOutcome::ConnectionClosed(_)
                    | ConnectionServeOutcome::PolicyFailed(_) => {}
                }
                return;
            }
        }
    }
}

/// Reused request-head buffer for the sendfile keepalive loop.
///
/// `mem::take` pulls the capacitated `Vec` out of thread-local storage so the
/// hot path copies the header without a fresh allocation after the first request.
struct HeaderBuf(Vec<u8>);

thread_local! {
    static HEADER_BUF: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl HeaderBuf {
    fn copy_from(head: &[u8]) -> Self {
        HEADER_BUF.with(|slot| {
            let mut guard = slot.borrow_mut();
            let mut buf = std::mem::take(&mut *guard);
            buf.clear();
            buf.extend_from_slice(head);
            HeaderBuf(buf)
        })
    }
}

impl Drop for HeaderBuf {
    fn drop(&mut self) {
        HEADER_BUF.with(|slot| {
            let mut guard = slot.borrow_mut();
            if guard.capacity() < self.0.capacity() {
                *guard = std::mem::take(&mut self.0);
            }
        });
    }
}

impl std::ops::Deref for HeaderBuf {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

/// Read one request (if needed) and start sending its sendfile response via the FSM.
fn sendfile_read_and_serve(
    epfd: RawFd,
    fd: RawFd,
    conns: &mut HashMap<RawFd, ConnState>,
) -> ReadServe {
    let need_read = match conns.get(&fd) {
        None => return ReadServe::Closed,
        Some(c) => find_header_end(&c.buf[..c.len]).is_none(),
    };
    if need_read {
        // EPOLLET: pull every byte already queued. One short read leaves the rest
        // buffered with no new edge, and that connection waits out the window.
        let mut tmp = [0u8; 256];
        loop {
            let n = unsafe {
                // SAFETY: `tmp` is a valid stack buffer; `read` on a non-blocking epoll-owned fd.
                libc::read(fd, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len())
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                remove_conn(epfd, fd, conns);
                return ReadServe::Closed;
            }
            if n == 0 {
                remove_conn(epfd, fd, conns);
                return ReadServe::Closed;
            }
            let n = n as usize;
            let Some(c) = conns.get_mut(&fd) else {
                return ReadServe::Closed;
            };
            if c.len + n > c.buf.len() {
                remove_conn(epfd, fd, conns);
                return ReadServe::Closed;
            }
            c.buf[c.len..c.len + n].copy_from_slice(&tmp[..n]);
            c.len += n;
            if find_header_end(&c.buf[..c.len]).is_some() {
                break;
            }
        }
    }

    let (hdr_end, body_len, head_owned) = {
        let Some(c) = conns.get(&fd) else {
            return ReadServe::Closed;
        };
        let Some(end) = find_header_end(&c.buf[..c.len]) else {
            if c.len >= MAX_HEADER {
                remove_conn(epfd, fd, conns);
                return ReadServe::Closed;
            }
            return ReadServe::Idle;
        };
        let hdr_end = end + 4;
        let head = &c.buf[..hdr_end];
        if c.attachment.is_generation_stale() {
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        }
        if c.attachment.is_draining() {
            let resp = c.attachment.drain_boundary_response(head);
            let _ = write_response_epoll_fd(fd, resp);
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        }
        if !request_headers_safe_for_wire(head) {
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        }
        let Some(body_len) = content_length_bytes(head) else {
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        };
        // Cap067 LA-CAP067-001: wait for full request body before issuing a sendfile handle.
        let consume_end = hdr_end.saturating_add(body_len);
        if consume_end > c.len {
            return ReadServe::Idle;
        }
        let head_owned = HeaderBuf::copy_from(head);
        (hdr_end, body_len, head_owned)
    };

    let asset_handle = {
        let Some(c) = conns.get(&fd) else {
            return ReadServe::Closed;
        };
        c.attachment
            .static_site_slot()
            .and_then(|slot| static_epoll::match_sendfile_asset(slot, &head_owned))
    };

    let Some(asset_handle) = asset_handle else {
        // Cap067 CAP067_MISS_NEEDHYPER_HANG: eligible miss must terminate in epoll.
        // NeedHyper → WirePlan::Static → register_sendfile_from_tokio_first re-enters
        // Cap067 and hangs (0 response bytes). Cap004-classified wire instead.
        let site_slot = conns.get(&fd).and_then(|c| c.attachment.static_site_slot());
        if let Some(slot) = site_slot {
            if let Some(wire) = static_epoll::sendfile_miss_http_wire(slot, &head_owned) {
                static_epoll::note_sendfile_fallback();
                let Some(peer) = conns.get(&fd).map(|c| c.peer) else {
                    return ReadServe::Closed;
                };
                match admit_owned_product_request(fd, peer) {
                    Ok(true) => {}
                    Ok(false) => {
                        remove_conn(epfd, fd, conns);
                        return ReadServe::Closed;
                    }
                    Err(err) => {
                        warn!(%err, fd, peer = %peer, "wire rate-limit reject write failed");
                        remove_conn(epfd, fd, conns);
                        return ReadServe::Closed;
                    }
                }
                let waf_reject = conns
                    .get(&fd)
                    .and_then(|c| c.attachment.evaluate_wire_waf_reject(&head_owned, c.peer));
                if let Some(reject) = waf_reject {
                    if let Err(err) = write_terminal_response_epoll_fd(fd, &reject) {
                        warn!(%err, fd, peer = %peer, "waf reject response write failed");
                    }
                    remove_conn(epfd, fd, conns);
                    return ReadServe::Closed;
                }
                let client_close = wire.windows(17).any(|w| w == b"Connection: close");
                {
                    let Some(c) = conns.get_mut(&fd) else {
                        return ReadServe::Closed;
                    };
                    let consume_end = hdr_end.saturating_add(body_len);
                    if consume_end >= c.len {
                        c.len = 0;
                    } else {
                        c.buf.copy_within(consume_end..c.len, 0);
                        c.len -= consume_end;
                    }
                    renew_read_deadline(c);
                }
                let started = Instant::now();
                match queue_inline_wire(epfd, fd, conns, wire, client_close, &head_owned.0, started) {
                    InlineFlush::Done => {
                        if client_close {
                            remove_conn(epfd, fd, conns);
                            return ReadServe::Closed;
                        }
                        return ReadServe::Continue;
                    }
                    InlineFlush::Parked => return ReadServe::StartedSending,
                    InlineFlush::Closed => return ReadServe::Closed,
                    InlineFlush::Idle => return ReadServe::Closed,
                }
            }
        }
        // Genuine Hyper fallback only when Cap067 declines ownership (ineligible / off).
        static_epoll::note_sendfile_fallback();
        let (head, rest) = {
            let Some(c) = conns.get(&fd) else {
                return ReadServe::Closed;
            };
            let head = bytes::Bytes::copy_from_slice(&c.buf[..hdr_end.min(c.len)]);
            let rest = if c.len > hdr_end {
                bytes::Bytes::copy_from_slice(&c.buf[hdr_end..c.len])
            } else {
                bytes::Bytes::new()
            };
            (head, rest)
        };
        return ReadServe::NeedHyper { head, rest };
    };

    let Some(peer) = conns.get(&fd).map(|c| c.peer) else {
        static_epoll::release_sendfile_handle(asset_handle);
        return ReadServe::Closed;
    };
    match admit_owned_product_request(fd, peer) {
        Ok(true) => {}
        Ok(false) => {
            static_epoll::release_sendfile_handle(asset_handle);
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        }
        Err(err) => {
            static_epoll::release_sendfile_handle(asset_handle);
            warn!(%err, fd, peer = %peer, "wire rate-limit reject write failed");
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        }
    }
    let waf_reject = conns
        .get(&fd)
        .and_then(|c| c.attachment.evaluate_wire_waf_reject(&head_owned, c.peer));
    if let Some(reject) = waf_reject {
        static_epoll::release_sendfile_handle(asset_handle);
        if let Err(err) = write_terminal_response_epoll_fd(fd, &reject) {
            warn!(%err, fd, peer = %peer, "waf reject response write failed");
        }
        remove_conn(epfd, fd, conns);
        return ReadServe::Closed;
    }

    // Cap067 LA-CAP067-003: begin before buffer consume so NOT_STARTED can Hyper-fallback.
    let session = match static_epoll::begin_sendfile_session(fd, &head_owned, asset_handle) {
        Ok(session) => session,
        Err(()) => {
            static_epoll::release_sendfile_handle(asset_handle);
            static_epoll::note_header_reject();
            // Same hang class as match-miss NeedHyper: do not re-enter Cap067 via Hyper.
            static_epoll::note_sendfile_fallback();
            let terminal = if static_epoll::is_head_wire_request(&head_owned) {
                SENDFILE_TERMINAL_503_HEAD
            } else {
                SENDFILE_TERMINAL_503
            };
            if let Err(err) = write_terminal_response_epoll_fd(fd, terminal) {
                warn!(%err, fd, "sendfile begin-reject response write failed");
            }
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        }
    };

    {
        let Some(c) = conns.get_mut(&fd) else {
            static_epoll::clear_sendfile_session(fd, session);
            return ReadServe::Closed;
        };
        let consume_end = hdr_end.saturating_add(body_len);
        if consume_end > c.len {
            // Should be unreachable: body completeness gated before match/begin.
            static_epoll::clear_sendfile_session(fd, session);
            static_epoll::note_error();
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        }
        if consume_end >= c.len {
            c.len = 0;
        } else {
            c.buf.copy_within(consume_end..c.len, 0);
            c.len -= consume_end;
        }
    }

    match static_epoll::pump_sendfile_session(fd, session) {
        StaticEpollPumpResult::Complete => {
            static_epoll::note_complete();
            static_epoll::clear_sendfile_session(fd, session);
            if let Some(c) = conns.get_mut(&fd) {
                renew_read_deadline(c);
            }
            ReadServe::Continue
        }
        StaticEpollPumpResult::Parked => {
            static_epoll::note_parked();
            let Some(c) = conns.get_mut(&fd) else {
                return ReadServe::Closed;
            };
            c.phase = ConnPhase::Sending(session);
            if epoll_mod_interest(epfd, fd, interest_sending_parked()).is_err() {
                static_epoll::clear_sendfile_session(fd, session);
                remove_conn(epfd, fd, conns);
                return ReadServe::Closed;
            }
            ReadServe::StartedSending
        }
        StaticEpollPumpResult::Error => {
            static_epoll::note_error();
            static_epoll::clear_sendfile_session(fd, session);
            remove_conn(epfd, fd, conns);
            ReadServe::Closed
        }
        StaticEpollPumpResult::Progress => ReadServe::Idle,
    }
}

/// Peek request line; route proxy/sendfile/unknown to Tokio before epoll registration.
fn should_handoff_on_accept(fd: RawFd) -> io::Result<bool> {
    let mut buf = [0u8; ACCEPT_PEEK_LEN];
    let n = unsafe {
        // SAFETY: peek buffer is stack-allocated; MSG_PEEK|MSG_DONTWAIT never consumes bytes.
        libc::recv(
            fd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    if n < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::WouldBlock {
            // Accept raced ahead of request bytes (P5 Connection:close under wrk -c100).
            // Cap067 StayAttached for unread sockets fills the map and collapses RPS
            // (~2k vs ~30k). Hand to Tokio immediately. Static still wins when the
            // request line is already buffered at accept (peek classify below).
            return Ok(true);
        }
        return Err(err);
    }
    if n == 0 {
        return Ok(true);
    }
    let peek = &buf[..n as usize];
    // Partial bytes without a request line: stay on Cap067 (static may still be arriving).
    if !peek.windows(2).any(|w| w == b"\r\n") {
        return Ok(false);
    }
    Ok(tokio_accept_required(peek) || !static_wire::epoll_keep_alive_eligible(peek))
}

fn tokio_accept_required(head: &[u8]) -> bool {
    // Cap061 LA-008: no filename force for former bench assets — same as ordinary
    // traffic via `!might_use_static_wire` (`/health` stays on wire; `/metrics` → Hyper).
    head.starts_with(b"GET /api/") || !static_wire::might_use_static_wire(head)
}

fn accept_batch(
    listener: &TcpListener,
    epfd: RawFd,
    conns: &mut HashMap<RawFd, ConnState>,
    entry: &PlatformConnectionEntry,
) -> io::Result<usize> {
    let mut accepted = 0usize;
    loop {
        if accepted >= ACCEPT_BATCH_LIMIT {
            break;
        }
        let (mut stream, peer) = match listener.accept() {
            Ok(conn) => conn,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(err) => return Err(err),
        };
        accepted += 1;
        stream.set_nonblocking(true)?;
        let _ = stream.set_nodelay(true);
        let fd = stream.as_raw_fd();
        let force_hyper = match should_handoff_on_accept(fd) {
            Ok(v) => v,
            Err(_) => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                continue;
            }
        };
        match entry.attach_epoll_connection(force_hyper) {
            EpollAttachDecision::Rejected(EpollAttachRejectReason::Drain) => {
                let _ = stream.write_all(DRAINING_RESPONSE);
                continue;
            }
            EpollAttachDecision::HyperReady(cap) => {
                // Do not prefetch on the Cap067 accept thread: header drain here
                // serializes accept workers (P5 Connection:close tax). Tokio reads
                // after handoff; MSG_PEEK already classified the route.
                match cap.handoff_to_hyper(stream, peer, None) {
                    ConnectionServeOutcome::Completed
                    | ConnectionServeOutcome::ConnectionClosed(_)
                    | ConnectionServeOutcome::PolicyFailed(_) => {}
                }
            }
            EpollAttachDecision::StayAttached(attachment) => {
                let mut ev = libc::epoll_event {
                    events: conn_epoll_events(),
                    u64: fd as u64,
                };
                // SAFETY: `fd` from `stream.as_raw_fd()`; ADD before `forget` so failure
                // drops `stream` (closes fd). On success, `forget` transfers ownership to
                // the epoll map until `remove_conn` or handoff.
                if unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_ADD, fd, &mut ev) } < 0 {
                    drop(attachment);
                    continue;
                }
                std::mem::forget(stream);
                // Inline wire (sendfile=false): EPOLLET will not re-edge if the
                // request is already buffered — drain now and honor Handoff.
                conns.insert(fd, ConnState::new(0, false, attachment, peer));
                let _ = drain_epoll_fd(epfd, fd, conns, entry);
            }
        }
    }
    Ok(accepted)
}

fn remove_conn(epfd: RawFd, fd: RawFd, conns: &mut HashMap<RawFd, ConnState>) {
    conns.remove(&fd);
    // SAFETY: `fd` was in this worker's epoll set (or DEL is harmless if already gone);
    // null event pointer is valid for EPOLL_CTL_DEL.
    let _ = unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_DEL, fd, std::ptr::null_mut()) };
    // SAFETY: after map+epoll removal, this worker uniquely owns `fd`; close releases it.
    // Must not be called after `detach_conn_for_handoff` (ownership moved to TcpStream).
    unsafe { libc::close(fd) };
}

/// Remove from epoll map without closing — fd ownership moves to `TcpStream::from_raw_fd`.
fn detach_conn_for_handoff(
    epfd: RawFd,
    fd: RawFd,
    conns: &mut HashMap<RawFd, ConnState>,
) -> Option<(EpollConnectionAttachment, SocketAddr)> {
    let state = conns.remove(&fd)?;
    // SAFETY: remove interest only; deliberately do not close — caller takes ownership via
    // `TcpStream::from_raw_fd(fd)`.
    let _ = unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_DEL, fd, std::ptr::null_mut()) };
    Some((state.attachment, state.peer))
}

#[cfg(test)]
mod tests {
    use super::{
        header_read_deadline_duration, keepalive_pool_threads, parse_header_read_timeout_ms,
        reset_header_read_deadline_cache_for_tests, response_status_from_wire, HeaderBuf, PeerIp,
    };
    use std::sync::Mutex;
    use std::time::Duration;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env(key: &str, value: Option<&str>, test: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let saved = std::env::var_os(key);
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
        test();
        match saved {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn header_scratch_reuses_capacity_across_requests() {
        let first = HeaderBuf::copy_from(b"GET /site/1k.bin HTTP/1.1\r\nHost: x\r\n\r\n");
        let capacity = first.0.capacity();
        assert!(capacity >= first.len());
        drop(first);
        let second = HeaderBuf::copy_from(b"GET /site/1k.bin HTTP/1.1\r\nHost: y\r\n\r\n");
        assert!(second.0.capacity() >= capacity);
        assert_eq!(&second[..], b"GET /site/1k.bin HTTP/1.1\r\nHost: y\r\n\r\n");
    }

    #[test]
    fn peer_ip_formats_without_heap_string() {
        let ip = PeerIp::from_ip("203.0.113.9".parse().expect("ip"));
        assert_eq!(ip.as_str(), "203.0.113.9");
    }

    #[test]
    fn epoll_pool_threads_match_worker_geometry_contract() {
        for invalid in [None, Some(""), Some("0"), Some("-1"), Some("invalid")] {
            with_env("EXYONQ_EPOLL_POOL_THREADS", invalid, || {
                assert_eq!(keepalive_pool_threads(8), 8);
            });
        }
        with_env("EXYONQ_EPOLL_POOL_THREADS", Some("4"), || {
            assert_eq!(keepalive_pool_threads(8), 4);
        });
        with_env("EXYONQ_EPOLL_POOL_THREADS", Some("99"), || {
            assert_eq!(keepalive_pool_threads(32), 16);
            assert_eq!(keepalive_pool_threads(2), 2);
        });
    }

    #[test]
    fn header_read_timeout_parse_matches_shared_env_contract() {
        assert_eq!(parse_header_read_timeout_ms(None), Duration::from_secs(30));
        assert_eq!(
            parse_header_read_timeout_ms(Some("")),
            Duration::from_secs(30)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("invalid")),
            Duration::from_secs(30)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("-1")),
            Duration::from_secs(30)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("0")),
            Duration::from_millis(0)
        );
        assert_eq!(
            parse_header_read_timeout_ms(Some("125")),
            Duration::from_millis(125)
        );
    }

    #[test]
    fn header_read_timeout_is_cached_until_test_reset() {
        with_env("EXYONQ_READ_TIMEOUT_MS", Some("73"), || {
            reset_header_read_deadline_cache_for_tests();
            assert_eq!(header_read_deadline_duration(), Duration::from_millis(73));

            std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "91");
            assert_eq!(header_read_deadline_duration(), Duration::from_millis(73));

            reset_header_read_deadline_cache_for_tests();
            assert_eq!(header_read_deadline_duration(), Duration::from_millis(91));
            reset_header_read_deadline_cache_for_tests();
        });
    }

    #[test]
    fn terminal_wire_status_parser_covers_product_statuses() {
        for status in [200, 206, 304, 403, 404, 416, 429, 500] {
            let wire = format!("HTTP/1.1 {status} status\r\nContent-Length: 0\r\n\r\n");
            assert_eq!(response_status_from_wire(wire.as_bytes()), Some(status));
        }
        assert_eq!(response_status_from_wire(b"transport aborted"), None);
    }
}
