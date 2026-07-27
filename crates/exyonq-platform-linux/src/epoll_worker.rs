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
use std::sync::atomic::{AtomicBool, Ordering};
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
    std::env::var("EXYONQ_EPOLL_POOL_THREADS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(4)
        .clamp(1, 8)
        .min(requested.max(1))
}

/// Register lazy-init params when `EXYONQ_EPOLL_STATIC=1` (pool starts on first P1 handoff).
pub fn prepare_keepalive_pool(workers: usize, entry: Arc<PlatformConnectionEntry>) {
    let threads = keepalive_pool_threads(workers);
    let _ = KEEPALIVE_LAZY_CONFIG.set(KeepalivePoolLazyConfig { threads, entry });
}

pub fn stop_keepalive_pool() {
    if let Ok(mut guard) = KEEPALIVE_POOL_OWNED.lock() {
        if let Some(pool) = guard.take() {
            pool.stop();
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
    let mut register_txs = Vec::with_capacity(workers);
    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let (tx, rx) = mpsc::sync_channel(256);
        register_txs.push(tx);
        let entry = Arc::clone(&entry);
        let shutdown = Arc::clone(&shutdown);
        handles.push(thread::spawn(move || {
            if let Err(err) = keepalive_worker_loop(rx, entry, shutdown) {
                warn!(%err, "epoll keep-alive worker stopped");
            }
        }));
    }

    let pool = EpollKeepAlivePool {
        shutdown,
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

    pub fn stop(self) {
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
    fn register(&self, job: KeepAliveRegister) -> io::Result<()> {
        if self.shutdown.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "pool stopped"));
        }
        let worker = (job.fd as usize) % self.register_txs.len();
        self.register_txs[worker]
            .send(job)
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "keep-alive pool full"))
    }
}

/// F2 consume path: enqueue an already-admitted keepalive transfer (fd ownership on Ok).
pub fn enqueue_keepalive_transfer(
    fd: RawFd,
    peer: SocketAddr,
    carry: [u8; 512],
    carry_len: usize,
    sendfile: bool,
    transfer: EpollKeepaliveTransfer,
) -> io::Result<()> {
    let register = ensure_keepalive_pool()?;
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            let _ = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
    register
        .register(KeepAliveRegister {
            fd,
            carry,
            carry_len,
            sendfile,
            peer,
            transfer,
        })
        .inspect_err(|_| unsafe {
            libc::close(fd);
        })
}

/// Productive epoll-listen starter registered at composition root.
pub fn start_epoll_listen_workers(
    listen: SocketAddr,
    workers: usize,
    entry: Arc<PlatformConnectionEntry>,
) -> io::Result<OsWorkerGuard> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(workers);
    let mut listeners = Vec::with_capacity(workers);

    for _ in 0..workers {
        let listener = bind_tuned_std(listen)?;
        listener.set_nonblocking(true)?;
        listeners.push(listener.try_clone()?);

        let entry = Arc::clone(&entry);
        let shutdown = Arc::clone(&shutdown);

        handles.push(thread::spawn(move || {
            if let Err(err) = epoll_loop(listener, entry, shutdown) {
                warn!(%err, "epoll static worker stopped");
            }
        }));
    }

    Ok(OsWorkerGuard::new(shutdown, handles, listeners))
}

fn keepalive_worker_loop(
    register_rx: mpsc::Receiver<KeepAliveRegister>,
    entry: Arc<PlatformConnectionEntry>,
    shutdown: Arc<AtomicBool>,
) -> io::Result<()> {
    let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if epfd < 0 {
        return Err(io::Error::last_os_error());
    }

    let p1_wire = static_wire::p1_bench_wire_rodata();
    let mut conns: HashMap<RawFd, ConnState> = HashMap::new();
    let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS];

    while !shutdown.load(Ordering::Relaxed) {
        while let Ok(reg) = register_rx.try_recv() {
            if let Err(err) = add_registered_conn(epfd, &mut conns, reg, p1_wire, &entry) {
                warn!(%err, "epoll keep-alive register failed");
            }
        }

        expire_stale_header_reads(epfd, &mut conns);

        let n = unsafe { libc::epoll_wait(epfd, events.as_mut_ptr(), MAX_EVENTS as i32, 100) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            return Err(err);
        }
        for ev in &events[..n as usize] {
            let fd = ev.u64 as RawFd;
            if ev.events & (EPOLLIN | EPOLLRDHUP | EPOLLOUT) != 0 {
                // IU2 / connection-local: never kill the keepalive worker loop.
                let _ = drain_epoll_fd(epfd, fd, &mut conns, p1_wire, &entry);
            }
        }
        expire_stale_header_reads(epfd, &mut conns);
    }
    Ok(())
}

fn header_read_deadline_duration() -> Duration {
    std::env::var("EXYONQ_READ_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(30))
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

fn request_headers_safe_for_wire(head: &[u8]) -> bool {
    fn count_field(head: &[u8], name: &[u8]) -> usize {
        head.windows(name.len())
            .filter(|window| *window == name)
            .count()
    }
    if count_field(head, b"Content-Length:") > 1 {
        return false;
    }
    if count_field(head, b"Transfer-Encoding:") >= 1 && count_field(head, b"Content-Length:") >= 1 {
        return false;
    }
    true
}

fn write_all_fd(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
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
    p1_wire: &[u8],
    entry: &PlatformConnectionEntry,
) -> io::Result<()> {
    let attachment = entry.attach_epoll_keepalive_transfer(reg.transfer);
    let mut state = ConnState::new(reg.carry_len, reg.sendfile, attachment, reg.peer);
    state.buf[..reg.carry_len].copy_from_slice(&reg.carry[..reg.carry_len]);

    let mut ev = libc::epoll_event {
        events: conn_epoll_events(),
        u64: reg.fd as u64,
    };
    if unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_ADD, reg.fd, &mut ev) } < 0 {
        write_terminal_503(reg.fd);
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
    if ready {
        if sendfile {
            drive_sendfile_fd(epfd, fd, conns);
        } else if let Err(err) = serve_epoll_fd(fd, conns.get_mut(&fd), p1_wire) {
            warn!(%err, fd, "epoll A2 serve failed");
            remove_conn(epfd, fd, conns);
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
    /// In-memory wire conns (P1/P7/health) always stay `Reading` — unchanged behavior.
    phase: ConnPhase,
    /// True only for gated P2/P3 sendfile conns; routes events to the FSM, not `serve_epoll_fd`.
    sendfile: bool,
    peer: SocketAddr,
    /// F1/F2 opaque attachment (owns lifecycle token).
    attachment: EpollConnectionAttachment,
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
) -> io::Result<()> {
    let listen_fd = listener.as_raw_fd();
    let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if epfd < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut ev = libc::epoll_event {
        events: EPOLLIN | EPOLLET,
        u64: listen_fd as u64,
    };
    if unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_ADD, listen_fd, &mut ev) } < 0 {
        return Err(io::Error::last_os_error());
    }

    let p1_wire = static_wire::p1_bench_wire_rodata();
    let mut conns: HashMap<RawFd, ConnState> = HashMap::new();
    let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS];

    while !shutdown.load(Ordering::Relaxed) {
        expire_stale_header_reads(epfd, &mut conns);

        let n = unsafe { libc::epoll_wait(epfd, events.as_mut_ptr(), MAX_EVENTS as i32, 500) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            return Err(err);
        }
        for ev in &events[..n as usize] {
            let fd = ev.u64 as RawFd;
            if fd == listen_fd {
                // accept_batch failures that are connection-local stay inside the helper;
                // only listener/OS errors are worker-fatal.
                accept_batch(&listener, epfd, &mut conns, &entry)?;
                continue;
            }
            if ev.events & (EPOLLIN | EPOLLRDHUP | EPOLLOUT) != 0 {
                // IU2 / connection-local: never kill the epoll worker loop.
                let _ = drain_epoll_fd(epfd, fd, &mut conns, p1_wire, &entry);
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
    p1_wire: &[u8],
    entry: &PlatformConnectionEntry,
) -> io::Result<()> {
    let _ = entry; // identity reserved for future mechanism diagnostics
                   // ADR-025 PR #2: gated sendfile conns are driven by the non-blocking FSM, never Handoff.
    if conns.get(&fd).map(|c| c.sendfile).unwrap_or(false) {
        drive_sendfile_fd(epfd, fd, conns);
        return Ok(());
    }
    loop {
        match serve_epoll_fd(fd, conns.get_mut(&fd), p1_wire) {
            Ok(ServeResult::Idle) => break,
            Ok(ServeResult::Continue) => continue,
            Ok(ServeResult::Handoff { head, rest }) => {
                let Some((attachment, peer)) = detach_conn_for_handoff(epfd, fd, conns) else {
                    break;
                };
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
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let new_flags = if nonblocking {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
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

fn serve_epoll_fd(
    fd: RawFd,
    state: Option<&mut ConnState>,
    p1_wire: &[u8],
) -> Result<ServeResult, io::Error> {
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

        if !request_headers_safe_for_wire(head) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ambiguous request headers",
            ));
        }

        if let Some(site_slot) = state.attachment.static_site_slot() {
            match static_epoll::try_write_bench_response(
                site_slot,
                fd,
                head,
                p1_wire,
                write_response_epoll_fd,
            ) {
                static_epoll::StaticEpollBenchWriteResult::Written => {}
                static_epoll::StaticEpollBenchWriteResult::Handoff => {
                    let head_b = Bytes::copy_from_slice(head);
                    let rest_b = Bytes::copy_from_slice(&state.buf[hdr_end..state.len]);
                    return Ok(ServeResult::Handoff {
                        head: head_b,
                        rest: rest_b,
                    });
                }
                static_epoll::StaticEpollBenchWriteResult::NoMatch => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "bench write failed",
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
        if hdr_end >= state.len {
            state.len = 0;
            renew_read_deadline(state);
            return Ok(ServeResult::Continue);
        }
        state.buf.copy_within(hdr_end..state.len, 0);
        state.len -= hdr_end;
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
}

/// Drive a gated P2/P3 sendfile connection with the non-blocking FSM.
///
/// Lazy interest: stays armed `EPOLLIN` while reading and only switches to `EPOLLOUT` when a
/// body parks (`PumpResult::Parked`), restoring `EPOLLIN` on resume-`Complete`. The common
/// case (healthy socket) completes in the first drain with **0** `EPOLL_CTL_MOD` per request.
/// `Parked` stops the loop immediately and is never mapped to a retry (PR #1 contract).
fn drive_sendfile_fd(epfd: RawFd, fd: RawFd, conns: &mut HashMap<RawFd, ConnState>) {
    loop {
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
        }
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
        let mut tmp = [0u8; 256];
        let n = unsafe {
            // SAFETY: `tmp` is a valid stack buffer; `read` on a non-blocking epoll-owned fd.
            libc::read(fd, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len())
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                return ReadServe::Idle;
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
    }

    let (hdr_end, head_only, asset_handle) = {
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
        if !request_headers_safe_for_wire(head) {
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        }
        let head_only = static_epoll::is_head_wire_request(head);
        let asset_handle = c
            .attachment
            .static_site_slot()
            .and_then(|slot| static_epoll::match_bench_sendfile(slot, head));
        (hdr_end, head_only, asset_handle)
    };

    let Some(asset_handle) = asset_handle else {
        remove_conn(epfd, fd, conns);
        return ReadServe::Closed;
    };

    {
        let Some(c) = conns.get_mut(&fd) else {
            return ReadServe::Closed;
        };
        if hdr_end >= c.len {
            c.len = 0;
        } else {
            c.buf.copy_within(hdr_end..c.len, 0);
            c.len -= hdr_end;
        }
    }

    let session = match static_epoll::begin_sendfile_session(fd, head_only, asset_handle) {
        Ok(session) => session,
        Err(()) => {
            static_epoll::note_header_reject();
            remove_conn(epfd, fd, conns);
            return ReadServe::Closed;
        }
    };

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
            return Ok(false);
        }
        return Err(err);
    }
    if n == 0 {
        return Ok(true);
    }
    let peek = &buf[..n as usize];
    Ok(tokio_accept_required(peek) || !static_wire::epoll_keep_alive_eligible(peek))
}

fn tokio_accept_required(head: &[u8]) -> bool {
    head.starts_with(b"GET /api/")
        || head.starts_with(b"GET /site/64k.bin ")
        || head.starts_with(b"HEAD /site/64k.bin ")
        || head.starts_with(b"GET /site/1m.bin ")
        || head.starts_with(b"HEAD /site/1m.bin ")
        || !static_wire::might_use_static_wire(head)
}

fn accept_batch(
    listener: &TcpListener,
    epfd: RawFd,
    conns: &mut HashMap<RawFd, ConnState>,
    entry: &PlatformConnectionEntry,
) -> io::Result<()> {
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
                if unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_ADD, fd, &mut ev) } < 0 {
                    drop(attachment);
                    continue;
                }
                std::mem::forget(stream);
                conns.insert(fd, ConnState::new(0, false, attachment, peer));
            }
        }
    }
    Ok(())
}

fn remove_conn(epfd: RawFd, fd: RawFd, conns: &mut HashMap<RawFd, ConnState>) {
    conns.remove(&fd);
    let _ = unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_DEL, fd, std::ptr::null_mut()) };
    unsafe { libc::close(fd) };
}

/// Remove from epoll map without closing — fd ownership moves to `TcpStream::from_raw_fd`.
fn detach_conn_for_handoff(
    epfd: RawFd,
    fd: RawFd,
    conns: &mut HashMap<RawFd, ConnState>,
) -> Option<(EpollConnectionAttachment, SocketAddr)> {
    let state = conns.remove(&fd)?;
    let _ = unsafe { libc::epoll_ctl(epfd, EPOLL_CTL_DEL, fd, std::ptr::null_mut()) };
    Some((state.attachment, state.peer))
}
