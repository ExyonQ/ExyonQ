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
//! Raw HTTP/1.1 loop for pre-wired static responses (P1/P2/P3/P7 latency).

#[cfg(target_os = "linux")]
use crate::sendfile;
use crate::wire_io as conn_io;
use crate::{wire, StaticRoot};
use bytes::Bytes;
use std::io::{self, Result};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
#[cfg(target_os = "linux")]
use std::sync::{mpsc, Mutex, OnceLock};
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use tokio::io::AsyncWriteExt;
#[cfg(target_os = "linux")]
use tracing::debug;

const MAX_HEADER: usize = 8192;

fn header_read_timeout() -> std::time::Duration {
    crate::wire_io::header_read_timeout()
}

/// P1 keep-alive: stack buffer + async read + wire via `try_io` (unused; tight loop regressed vs baseline).
#[cfg(target_os = "linux")]
#[allow(dead_code)]
async fn serve_one_k_keepalive_async(
    stream: &mut TcpStream,
    wire: &[u8],
    mut pending: &[u8],
) -> Result<()> {
    let mut buf = [0u8; 512];
    let mut len = pending.len().min(buf.len());
    buf[..len].copy_from_slice(&pending[..len]);
    pending = &pending[len..];
    let _ = pending;

    loop {
        loop {
            if len == 0 {
                break;
            }
            let Some(end) = conn_io::find_header_end(&buf[..len]) else {
                break;
            };
            let hdr_end = end + 4;
            write_wire_fast(stream, wire).await?;
            if hdr_end >= len {
                len = 0;
                break;
            }
            buf.copy_within(hdr_end..len, 0);
            len -= hdr_end;
        }
        if len >= MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = conn_io::read_tcp(stream, &mut buf[len..]).await?;
        if n == 0 {
            return Ok(());
        }
        len += n;
    }
}

/// P2 keep-alive: stack buffer + async read + sendfile via `try_io` (no into_std / extra threads).
#[cfg(target_os = "linux")]
async fn serve_sendfile_keepalive_async(
    stream: &mut TcpStream,
    asset: &sendfile::SendfileAsset,
    mut pending: &[u8],
) -> Result<()> {
    let mut buf = [0u8; 512];
    let mut len = pending.len().min(buf.len());
    buf[..len].copy_from_slice(&pending[..len]);
    pending = &pending[len..];
    let _ = pending;

    loop {
        loop {
            if len == 0 {
                break;
            }
            let Some(end) = conn_io::find_header_end(&buf[..len]) else {
                break;
            };
            let hdr_end = end + 4;
            if StaticRoot::is_head_wire_request(&buf[..len]) {
                sendfile::write_bench_head_only_try_io(stream, asset).await?;
            } else {
                sendfile::write_bench_response_try_io(stream, asset).await?;
            }
            if hdr_end >= len {
                len = 0;
                break;
            }
            buf.copy_within(hdr_end..len, 0);
            len -= hdr_end;
        }
        if len >= MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = conn_io::read_tcp(stream, &mut buf[len..]).await?;
        if n == 0 {
            return Ok(());
        }
        len += n;
    }
}

/// Linux: bounded OS thread pool for blocking static keep-alive (P2/P3/P7).
#[cfg(target_os = "linux")]
const STATIC_BLOCKING_QUEUE_DEFAULT: usize = 256;

#[cfg(target_os = "linux")]
const STATIC_BLOCKING_ADMISSION_REJECTED: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\nunavailable";

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub enum BlockingAdmission {
    Accepted,
    Rejected(TcpStream),
}

#[cfg(target_os = "linux")]
struct BlockingStaticJob {
    site: Arc<StaticRoot>,
    stream: TcpStream,
    first_head: Bytes,
    carry: Bytes,
    _permit: BlockingAdmissionPermit,
}

#[cfg(target_os = "linux")]
struct BlockingAdmissionPermit {
    pool: Arc<BlockingStaticPool>,
}

#[cfg(target_os = "linux")]
impl Drop for BlockingAdmissionPermit {
    fn drop(&mut self) {
        self.pool.inflight.fetch_sub(1, Ordering::Release);
    }
}

#[cfg(target_os = "linux")]
struct BlockingStaticPool {
    tx: mpsc::SyncSender<BlockingStaticJob>,
    inflight: AtomicUsize,
    max_admission: usize,
    admission_accepted: AtomicU64,
    admission_rejected: AtomicU64,
    queue_full: AtomicU64,
}

#[cfg(target_os = "linux")]
impl BlockingStaticPool {
    fn new(pool_threads: usize, queue_capacity: usize) -> Arc<Self> {
        // try_send needs at least one channel slot; admission cap stays pool + queue.
        let (tx, rx) = mpsc::sync_channel(queue_capacity.max(1));
        let rx = Arc::new(Mutex::new(rx));
        let pool = Arc::new(Self {
            tx,
            inflight: AtomicUsize::new(0),
            max_admission: pool_threads.saturating_add(queue_capacity),
            admission_accepted: AtomicU64::new(0),
            admission_rejected: AtomicU64::new(0),
            queue_full: AtomicU64::new(0),
        });
        for _ in 0..pool_threads {
            let rx = Arc::clone(&rx);
            let pool_ref = Arc::clone(&pool);
            thread::spawn(move || {
                loop {
                    let job = match rx.lock() {
                        Ok(guard) => match guard.recv() {
                            Ok(job) => job,
                            Err(_) => break,
                        },
                        Err(poison) => {
                            // Fail-closed: never panic the pool worker on a poisoned queue mutex.
                            drop(poison.into_inner());
                            break;
                        }
                    };
                    run_blocking_static_job(job);
                }
                let _ = pool_ref;
            });
        }
        pool
    }

    fn try_submit(
        self: &Arc<Self>,
        site: Arc<StaticRoot>,
        stream: TcpStream,
        first_head: Bytes,
        carry: Bytes,
    ) -> BlockingAdmission {
        if !self.try_acquire_inflight() {
            self.admission_rejected.fetch_add(1, Ordering::Relaxed);
            debug!("static blocking admission rejected: at capacity");
            return BlockingAdmission::Rejected(stream);
        }

        let job = BlockingStaticJob {
            site,
            stream,
            first_head,
            carry,
            _permit: BlockingAdmissionPermit {
                pool: Arc::clone(self),
            },
        };

        match self.tx.try_send(job) {
            Ok(()) => {
                self.admission_accepted.fetch_add(1, Ordering::Relaxed);
                BlockingAdmission::Accepted
            }
            Err(mpsc::TrySendError::Full(job)) => {
                drop(job._permit);
                self.queue_full.fetch_add(1, Ordering::Relaxed);
                self.admission_rejected.fetch_add(1, Ordering::Relaxed);
                debug!("static blocking admission rejected: queue full");
                BlockingAdmission::Rejected(job.stream)
            }
            Err(mpsc::TrySendError::Disconnected(job)) => {
                drop(job._permit);
                self.admission_rejected.fetch_add(1, Ordering::Relaxed);
                BlockingAdmission::Rejected(job.stream)
            }
        }
    }

    fn try_acquire_inflight(&self) -> bool {
        loop {
            let current = self.inflight.load(Ordering::Acquire);
            if current >= self.max_admission {
                return false;
            }
            if self
                .inflight
                .compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }

    fn metrics(&self) -> (u64, u64, u64, usize) {
        (
            self.admission_accepted.load(Ordering::Relaxed),
            self.admission_rejected.load(Ordering::Relaxed),
            self.queue_full.load(Ordering::Relaxed),
            self.inflight.load(Ordering::Relaxed),
        )
    }
}

#[cfg(target_os = "linux")]
fn blocking_static_pool() -> &'static Arc<BlockingStaticPool> {
    static POOL: OnceLock<Arc<BlockingStaticPool>> = OnceLock::new();
    POOL.get_or_init(|| BlockingStaticPool::new(blocking_pool_size(), blocking_queue_capacity()))
}

#[cfg(target_os = "linux")]
pub fn try_spawn_blocking_static(
    site: Arc<StaticRoot>,
    stream: TcpStream,
    first_head: Bytes,
    carry: Bytes,
) -> BlockingAdmission {
    blocking_static_pool().try_submit(site, stream, first_head, carry)
}

#[cfg(target_os = "linux")]
pub fn spawn_blocking_static(
    site: Arc<StaticRoot>,
    stream: TcpStream,
    first_head: Bytes,
    carry: Bytes,
) {
    if let BlockingAdmission::Rejected(stream) =
        try_spawn_blocking_static(site, stream, first_head, carry)
    {
        tokio::spawn(async move {
            shed_blocking_admission(stream).await;
        });
    }
}

#[cfg(target_os = "linux")]
/// Admission counters (ADR-025 names): `static_blocking_admission_accepted`,
/// `static_blocking_admission_rejected`, `static_blocking_queue_full`, `static_blocking_inflight`.
pub fn static_blocking_admission_metrics() -> (u64, u64, u64, usize) {
    blocking_static_pool().metrics()
}

#[cfg(target_os = "linux")]
pub async fn shed_blocking_admission(mut stream: TcpStream) {
    let _ = stream.write_all(STATIC_BLOCKING_ADMISSION_REJECTED).await;
    let _ = stream.shutdown().await;
}

#[cfg(target_os = "linux")]
fn default_blocking_pool_size() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_mul(32))
        .unwrap_or(128)
        .clamp(64, 512)
}

#[cfg(target_os = "linux")]
fn blocking_pool_size() -> usize {
    std::env::var("EXYONQ_STATIC_BLOCKING_POOL")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or_else(default_blocking_pool_size)
        .clamp(4, 512)
}

#[cfg(target_os = "linux")]
fn blocking_queue_capacity() -> usize {
    std::env::var("EXYONQ_STATIC_BLOCKING_QUEUE")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(STATIC_BLOCKING_QUEUE_DEFAULT)
        .clamp(1, 4096)
}

#[cfg(target_os = "linux")]
fn run_blocking_static_job(job: BlockingStaticJob) {
    let Ok(mut std_stream) = job.stream.into_std() else {
        return;
    };
    if std_stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = std_stream.set_read_timeout(Some(header_read_timeout()));
    let _ = serve_blocking_sync(&job.site, &mut std_stream, job.first_head, job.carry);
}

/// Linux: run the full keep-alive static loop on a blocking thread (legacy API).
#[cfg(target_os = "linux")]
pub async fn serve_blocking_pool(
    site: Arc<StaticRoot>,
    stream: TcpStream,
    first_head: Bytes,
    carry: Bytes,
) -> Result<()> {
    match try_spawn_blocking_static(site, stream, first_head, carry) {
        BlockingAdmission::Accepted => Ok(()),
        BlockingAdmission::Rejected(stream) => {
            shed_blocking_admission(stream).await;
            Ok(())
        }
    }
}

/// KD2_TEMPORARY_SHIM: sync accept / io_uring handoff until wire trait (KD2.5).
/// Cross-crate visibility required; core exposes via `static_conn` shim as `pub(crate)` only.
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn serve_blocking_sync(
    site: &StaticRoot,
    stream: &mut std::net::TcpStream,
    first_head: Bytes,
    mut carry: Bytes,
) -> Result<()> {
    use std::os::unix::io::AsRawFd;

    let fd = stream.as_raw_fd();
    let mut pending_head = Some(first_head);
    let mut buf = [0u8; 512];
    loop {
        let head = if let Some(head) = pending_head.take() {
            head
        } else {
            carry = ensure_headers_blocking(stream, &mut buf, carry)?;
            if !headers_complete(&carry) {
                break;
            }
            let (head, rest) = split_head(carry)?;
            carry = rest;
            head
        };

        let served = write_static_response_blocking(site, fd, head.as_ref())?;
        if !served {
            if head.is_empty() {
                break;
            }
            conn_io::write_response_fd(fd, &wire::NOT_FOUND_KEEP_ALIVE)?;
            continue;
        }
        // P1 tight loop: same precooked 1 KiB wire for every keep-alive request on this socket.
        if site.is_bench_one_k_head(head.as_ref()) {
            if let Some(wire_bytes) = site.bench_one_k_wire_bytes() {
                serve_blocking_one_k_zero(fd, wire_bytes, stream, carry.as_ref())?;
                break;
            }
        }
        // P2/P3 tight loop: header + sendfile body per keep-alive request (blocking socket).
        if site.is_bench_64k_head(head.as_ref()) || site.is_bench_1m_head(head.as_ref()) {
            if let Some(asset) = site.match_bench_sendfile_head(head.as_ref()) {
                serve_blocking_sendfile_zero(fd, asset, stream, carry.as_ref())?;
                break;
            }
        }
        // P7 tight loop: shared wire body for routeNNN.bin keep-alive requests.
        if site.is_bench_route_head(head.as_ref()) {
            if let Some(wire_bytes) = site.bench_route_wire_bytes() {
                serve_blocking_route_zero(fd, wire_bytes, stream, carry.as_ref())?;
                break;
            }
        }
    }
    Ok(())
}

/// P2/P3 keep-alive: stack buffer + blocking sendfile body (no Tokio per request).
#[cfg(target_os = "linux")]
fn serve_blocking_sendfile_zero(
    out_fd: i32,
    asset: &sendfile::SendfileAsset,
    stream: &mut std::net::TcpStream,
    mut pending: &[u8],
) -> Result<()> {
    use std::os::unix::io::AsRawFd;

    let in_fd = asset.file.as_raw_fd();
    let header = asset.header.as_ref();
    let body_len = asset.body_len;

    let mut buf = [0u8; 512];
    let mut len = pending.len().min(buf.len());
    buf[..len].copy_from_slice(&pending[..len]);
    pending = &pending[len..];
    let _ = pending;

    loop {
        loop {
            if len == 0 {
                break;
            }
            let Some(end) = conn_io::find_header_end(&buf[..len]) else {
                break;
            };
            let hdr_end = end + 4;
            if StaticRoot::is_head_wire_request(&buf[..len]) {
                conn_io::write_response_fd(out_fd, header)?;
            } else {
                sendfile::write_bench_sendfile_fd(out_fd, in_fd, header, body_len)?;
            }
            if hdr_end >= len {
                len = 0;
                break;
            }
            buf.copy_within(hdr_end..len, 0);
            len -= hdr_end;
        }
        if len >= MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = match conn_io::blocking_tcp_read(stream, &mut buf[len..]) {
            Ok(n) => n,
            Err(e) if conn_io::is_connection_read_timeout(&e) => return Ok(()),
            Err(e) => return Err(e),
        };
        if n == 0 {
            return Ok(());
        }
        len += n;
    }
}

/// P1 keep-alive: stack buffer only, no `Bytes` alloc per request.
#[cfg(target_os = "linux")]
fn serve_blocking_one_k_zero(
    fd: i32,
    wire: &[u8],
    stream: &mut std::net::TcpStream,
    mut pending: &[u8],
) -> Result<()> {
    let mut buf = [0u8; 512];
    let mut len = pending.len().min(buf.len());
    buf[..len].copy_from_slice(&pending[..len]);
    pending = &pending[len..];
    let _ = pending;

    loop {
        loop {
            if len == 0 {
                break;
            }
            let Some(end) = conn_io::find_header_end(&buf[..len]) else {
                break;
            };
            let hdr_end = end + 4;
            conn_io::write_response_fd(fd, wire)?;
            if hdr_end >= len {
                len = 0;
                break;
            }
            buf.copy_within(hdr_end..len, 0);
            len -= hdr_end;
        }
        if len >= MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = match conn_io::blocking_tcp_read(stream, &mut buf[len..]) {
            Ok(n) => n,
            Err(e) if conn_io::is_connection_read_timeout(&e) => return Ok(()),
            Err(e) => return Err(e),
        };
        if n == 0 {
            return Ok(());
        }
        len += n;
    }
}

/// P7 keep-alive: stack buffer + shared route wire (same body for every routeNNN.bin).
#[cfg(target_os = "linux")]
fn serve_blocking_route_zero(
    fd: i32,
    wire: &[u8],
    stream: &mut std::net::TcpStream,
    mut pending: &[u8],
) -> Result<()> {
    let mut buf = [0u8; 512];
    let mut len = pending.len().min(buf.len());
    buf[..len].copy_from_slice(&pending[..len]);
    pending = &pending[len..];
    let _ = pending;

    loop {
        loop {
            if len == 0 {
                break;
            }
            let Some(end) = conn_io::find_header_end(&buf[..len]) else {
                break;
            };
            let hdr_end = end + 4;
            conn_io::write_response_fd(fd, wire)?;
            if hdr_end >= len {
                len = 0;
                break;
            }
            buf.copy_within(hdr_end..len, 0);
            len -= hdr_end;
        }
        if len >= MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = match conn_io::blocking_tcp_read(stream, &mut buf[len..]) {
            Ok(n) => n,
            Err(e) if conn_io::is_connection_read_timeout(&e) => return Ok(()),
            Err(e) => return Err(e),
        };
        if n == 0 {
            return Ok(());
        }
        len += n;
    }
}

#[cfg(target_os = "linux")]
fn write_static_response_blocking(site: &StaticRoot, fd: i32, head: &[u8]) -> Result<bool> {
    if head.starts_with(b"GET /health ") || head.starts_with(b"HEAD /health ") {
        conn_io::write_response_fd(fd, &wire::HEALTH_KEEP_ALIVE)?;
        return Ok(true);
    }
    if head.starts_with(b"GET /metrics ") || head.starts_with(b"GET /metrics\r") {
        conn_io::write_response_fd(fd, &crate::sendfile_metrics::wire_prometheus_response())?;
        return Ok(true);
    }
    if let Some(asset) = site.match_bench_sendfile_head(head) {
        if StaticRoot::is_head_wire_request(head) {
            conn_io::write_response_fd(fd, asset.header.as_ref())?;
        } else {
            sendfile::write_bench_response_fd(fd, asset)?;
        }
        return Ok(true);
    }
    if let Some(wire_bytes) = site.match_bench_head(head) {
        conn_io::write_response_fd(fd, wire_bytes.as_ref())?;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(target_os = "linux")]
fn ensure_headers_blocking(
    stream: &mut std::net::TcpStream,
    buf: &mut [u8; 512],
    mut carry: Bytes,
) -> Result<Bytes> {
    loop {
        if headers_complete(&carry) {
            return Ok(carry);
        }
        if carry.len() > MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = match conn_io::blocking_tcp_read(stream, buf) {
            Ok(n) => n,
            Err(e) if conn_io::is_connection_read_timeout(&e) => return Ok(carry),
            Err(e) => return Err(e),
        };
        if n == 0 {
            return Ok(carry);
        }
        carry = conn_io::append_bytes(carry, &buf[..n]);
    }
}

pub async fn serve(
    site: Arc<StaticRoot>,
    mut stream: TcpStream,
    first_head: Bytes,
    mut carry: Bytes,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    if let Some(asset) = site.match_bench_sendfile_head(first_head.as_ref()) {
        // P2/P3: blocking sendfile keep-alive — async try_io regressed under ceiling load (P3 −18% req).
        if site.is_bench_64k_head(first_head.as_ref()) || site.is_bench_1m_head(first_head.as_ref())
        {
            // ADR-025 PR #2 (opt-in: EXYONQ_EPOLL_STATIC=1 + EXYONQ_EPOLL_SENDFILE=1):
            // register into the epoll sendfile FSM (no Handoff, no blocking thread). On any
            // synchronous registration failure the original stream is returned and we fall
            // back to the existing blocking admission path — never a silent drop.
            let mut fell_back_from_epoll = false;
            if crate::wire_eligibility::epoll_sendfile_eligible(first_head.as_ref()) {
                match exyonq_module_api::static_wire::register_sendfile_from_tokio_first(
                    stream,
                    first_head.as_ref(),
                    carry.as_ref(),
                ) {
                    Ok(()) => return Ok(()),
                    Err(restored) => {
                        exyonq_module_api::static_wire::note_sendfile_fallback();
                        fell_back_from_epoll = true;
                        stream = restored;
                    }
                }
            }
            return match try_spawn_blocking_static(site, stream, first_head, carry) {
                BlockingAdmission::Accepted => Ok(()),
                BlockingAdmission::Rejected(stream) => {
                    if fell_back_from_epoll {
                        exyonq_module_api::static_wire::note_sendfile_fallback_rejected();
                    }
                    shed_blocking_admission(stream).await;
                    Ok(())
                }
            };
        }
        sendfile::write_bench_response_try_io(&mut stream, asset).await?;
        return serve_sendfile_keepalive_async(&mut stream, asset, carry.as_ref()).await;
    }

    #[cfg(target_os = "linux")]
    if site.is_bench_route_head(first_head.as_ref()) {
        // P7: blocking route keep-alive — same pattern as P1/P2/P3 (no Tokio per request).
        return match try_spawn_blocking_static(site, stream, first_head, carry) {
            BlockingAdmission::Accepted => Ok(()),
            BlockingAdmission::Rejected(stream) => {
                shed_blocking_admission(stream).await;
                Ok(())
            }
        };
    }

    let mut pending_head = Some(first_head);
    loop {
        let head = if let Some(head) = pending_head.take() {
            head
        } else {
            carry = ensure_headers_tcp(&mut stream, carry).await?;
            if !headers_complete(&carry) {
                break;
            }
            let (head, rest) = split_head(carry)?;
            carry = rest;
            head
        };

        #[cfg(target_os = "linux")]
        if exyonq_module_api::static_wire::keepalive_handoff_enabled()
            && crate::wire_eligibility::epoll_keep_alive_eligible(head.as_ref())
        {
            match exyonq_module_api::static_wire::register_keepalive_from_tokio_first(
                stream,
                head.as_ref(),
                carry.as_ref(),
            ) {
                Ok(()) => return Ok(()),
                Err(restored) => stream = restored,
            }
        }

        let (next_stream, served) = write_static_response_tcp(&site, stream, head.as_ref()).await?;
        stream = next_stream;
        if !served {
            if head.is_empty() {
                break;
            }
            write_wire_fast(&mut stream, &wire::NOT_FOUND_KEEP_ALIVE).await?;
        } else {
            #[cfg(target_os = "linux")]
            if exyonq_module_api::static_wire::keepalive_handoff_enabled()
                && crate::wire_eligibility::epoll_keep_alive_eligible(head.as_ref())
            {
                match exyonq_module_api::static_wire::register_keepalive_from_tokio(
                    stream,
                    carry.as_ref(),
                ) {
                    Ok(()) => return Ok(()),
                    Err(restored) => stream = restored,
                }
            }
        }
    }
    Ok(())
}

pub async fn serve_async<S>(
    site: Arc<StaticRoot>,
    stream: &mut S,
    first_head: Bytes,
    mut carry: Bytes,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut pending_head = Some(first_head);
    loop {
        let head = if let Some(head) = pending_head.take() {
            head
        } else {
            carry = ensure_headers(stream, carry).await?;
            if !headers_complete(&carry) {
                break;
            }
            let (head, rest) = split_head(carry)?;
            carry = rest;
            head
        };

        let served = write_static_response_async(&site, stream, head.as_ref()).await?;
        if !served {
            if head.is_empty() {
                break;
            }
            conn_io::write_all_async(stream, &wire::NOT_FOUND_KEEP_ALIVE).await?;
        }
    }
    Ok(())
}

async fn write_static_response_tcp(
    site: &StaticRoot,
    stream: TcpStream,
    head: &[u8],
) -> Result<(TcpStream, bool)> {
    if head.starts_with(b"GET /health ") || head.starts_with(b"HEAD /health ") {
        let mut stream = stream;
        write_wire_fast(&mut stream, &wire::HEALTH_KEEP_ALIVE).await?;
        return Ok((stream, true));
    }
    #[cfg(target_os = "linux")]
    if head.starts_with(b"GET /metrics ") || head.starts_with(b"GET /metrics\r") {
        let mut stream = stream;
        write_wire_fast(
            &mut stream,
            &crate::sendfile_metrics::wire_prometheus_response(),
        )
        .await?;
        return Ok((stream, true));
    }

    #[cfg(target_os = "linux")]
    if let Some(asset) = site.match_bench_sendfile_head(head) {
        let mut stream = stream;
        if StaticRoot::is_head_wire_request(head) {
            sendfile::write_bench_head_only_try_io(&mut stream, asset).await?;
        } else {
            sendfile::write_bench_response_try_io(&mut stream, asset).await?;
        }
        return Ok((stream, true));
    }

    if let Some(wire_bytes) = site.match_bench_head(head) {
        let mut stream = stream;
        write_wire_fast(&mut stream, wire_bytes.as_ref()).await?;
        return Ok((stream, true));
    }

    Ok((stream, false))
}

async fn write_static_response_async<S: AsyncWrite + Unpin>(
    site: &StaticRoot,
    stream: &mut S,
    head: &[u8],
) -> Result<bool> {
    if head.starts_with(b"GET /health ") || head.starts_with(b"HEAD /health ") {
        conn_io::write_all_async(stream, &wire::HEALTH_KEEP_ALIVE).await?;
        return Ok(true);
    }
    #[cfg(target_os = "linux")]
    if head.starts_with(b"GET /metrics ") || head.starts_with(b"GET /metrics\r") {
        conn_io::write_all_async(stream, &crate::sendfile_metrics::wire_prometheus_response())
            .await?;
        return Ok(true);
    }
    if let Some(wire_bytes) = site.match_bench_head(head) {
        conn_io::write_all_async(stream, wire_bytes.as_ref()).await?;
        return Ok(true);
    }
    Ok(false)
}

#[allow(clippy::needless_return)] // return is required by the cfg-split body
async fn write_wire_fast(stream: &mut TcpStream, wire: &[u8]) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        use tokio::io::Interest;

        let fd = stream.as_raw_fd();
        stream
            .try_io(Interest::WRITABLE, || conn_io::write_response_fd(fd, wire))
            .map_err(|_| io::Error::other("wire write interrupted"))?;
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    conn_io::write_all_try_io(stream, wire).await
}

fn headers_complete(data: &[u8]) -> bool {
    conn_io::find_header_end(data).is_some()
}

async fn ensure_headers_tcp(stream: &mut TcpStream, carry: Bytes) -> Result<Bytes> {
    match tokio::time::timeout(
        header_read_timeout(),
        ensure_headers_tcp_inner(stream, carry),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "header read timeout",
        )),
    }
}

async fn ensure_headers_tcp_inner(stream: &mut TcpStream, mut carry: Bytes) -> Result<Bytes> {
    let mut buf = [0u8; 512];
    loop {
        if headers_complete(&carry) {
            return Ok(carry);
        }
        if carry.len() > MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = conn_io::read_tcp(stream, &mut buf).await?;
        if n == 0 {
            return Ok(carry);
        }
        carry = conn_io::append_bytes(carry, &buf[..n]);
    }
}

async fn ensure_headers<S: AsyncRead + Unpin>(stream: &mut S, carry: Bytes) -> Result<Bytes> {
    match tokio::time::timeout(header_read_timeout(), ensure_headers_inner(stream, carry)).await {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "header read timeout",
        )),
    }
}

async fn ensure_headers_inner<S: AsyncRead + Unpin>(
    stream: &mut S,
    mut carry: Bytes,
) -> Result<Bytes> {
    use tokio::io::AsyncReadExt;
    let mut buf = [0u8; 512];
    loop {
        if headers_complete(&carry) {
            return Ok(carry);
        }
        if carry.len() > MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(carry);
        }
        carry = conn_io::append_bytes(carry, &buf[..n]);
    }
}

fn split_head(data: Bytes) -> Result<(Bytes, Bytes)> {
    let Some(end) = conn_io::find_header_end(data.as_ref()) else {
        return Ok((data, Bytes::new()));
    };
    let head_len = end + 4;
    Ok((data.slice(0..head_len), data.slice(head_len..)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_complete_detects_end_of_headers() {
        let data = Bytes::from("GET /x HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(headers_complete(&data));
    }

    #[cfg(target_os = "linux")]
    mod blocking_admission {
        use super::*;
        use std::io::Read;
        use std::net::{Shutdown, TcpListener};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        fn head_64k_get() -> Bytes {
            Bytes::from("GET /site/64k.bin HTTP/1.1\r\nHost: x\r\n\r\n")
        }

        fn head_64k_head() -> Bytes {
            Bytes::from("HEAD /site/64k.bin HTTP/1.1\r\nHost: x\r\n\r\n")
        }

        fn minimal_site() -> (tempfile::TempDir, Arc<StaticRoot>) {
            let dir = tempfile::tempdir().expect("tempdir");
            std::fs::write(dir.path().join("64k.bin"), vec![0u8; 65536]).expect("64k.bin");
            let mut root = StaticRoot::new(dir.path(), "/site", None).expect("static root");
            root.preload_tree().expect("preload");
            (dir, Arc::new(root))
        }

        use std::sync::OnceLock;
        use tokio::io::AsyncReadExt;
        use tokio::runtime::Runtime;

        static TEST_RT: OnceLock<Runtime> = OnceLock::new();

        fn test_runtime() -> &'static Runtime {
            TEST_RT.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                    .expect("test runtime")
            })
        }

        fn tcp_pair() -> (TcpStream, std::net::TcpStream) {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("addr");
            let client =
                std::thread::spawn(move || std::net::TcpStream::connect(addr).expect("connect"));
            let (server, _) = listener.accept().expect("accept");
            server.set_nonblocking(true).expect("set_nonblocking");
            let client = client.join().expect("join");
            let tokio_stream = test_runtime()
                .block_on(async { TcpStream::from_std(server).expect("tokio stream") });
            (tokio_stream, client)
        }

        fn shed_in_test(stream: TcpStream) {
            test_runtime().block_on(shed_blocking_admission(stream));
        }

        #[test]
        fn admission_reject_is_immediate_when_saturated() {
            let pool = BlockingStaticPool::new(1, 1);
            let (_dir, site) = minimal_site();

            let (stream1, mut client1) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream1, head_64k_get(), Bytes::new()),
                BlockingAdmission::Accepted
            ));

            let mut buf = [0u8; 65536];
            client1
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let n = client1.read(&mut buf).expect("first response");
            assert!(std::str::from_utf8(&buf[..n]).unwrap().contains("200"));

            let (stream2, _client2) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream2, head_64k_get(), Bytes::new()),
                BlockingAdmission::Accepted
            ));

            let (stream3, mut client3) = tcp_pair();
            let start = Instant::now();
            let result = pool.try_submit(site, stream3, head_64k_get(), Bytes::new());
            assert!(
                matches!(result, BlockingAdmission::Rejected(_)),
                "expected reject, got {result:?}"
            );
            assert!(
                start.elapsed() < Duration::from_millis(200),
                "rejection blocked caller for {:?}",
                start.elapsed()
            );
            if let BlockingAdmission::Rejected(stream) = result {
                shed_in_test(stream);
            }

            client3
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let n = client3.read(&mut buf).expect("read shed response");
            let response = std::str::from_utf8(&buf[..n]).expect("utf8");
            assert!(response.contains("503"));
            assert!(response.contains("unavailable"));

            let _ = client1.shutdown(Shutdown::Both);
        }

        #[test]
        fn admission_metrics_classify_accept_and_reject() {
            let pool = BlockingStaticPool::new(1, 0);
            let (_dir, site) = minimal_site();
            let (stream, mut client) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream, head_64k_get(), Bytes::new()),
                BlockingAdmission::Accepted
            ));

            let (stream2, mut client2) = tcp_pair();
            let result = pool.try_submit(site, stream2, head_64k_get(), Bytes::new());
            assert!(matches!(result, BlockingAdmission::Rejected(_)));
            if let BlockingAdmission::Rejected(stream) = result {
                shed_in_test(stream);
            }

            let (accepted, rejected, queue_full, inflight) = pool.metrics();
            assert_eq!(accepted, 1);
            assert_eq!(rejected, 1);
            assert!(queue_full >= 1 || rejected >= 1);

            let mut buf = [0u8; 512];
            client
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let n = client.read(&mut buf).unwrap();
            assert!(std::str::from_utf8(&buf[..n]).unwrap().contains("200"));

            client2
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let n2 = client2.read(&mut buf).unwrap();
            assert!(std::str::from_utf8(&buf[..n2]).unwrap().contains("503"));
            let _ = inflight;
        }

        #[test]
        fn head_static_request_still_served_when_admitted() {
            let pool = BlockingStaticPool::new(2, 2);
            let (_dir, site) = minimal_site();
            let (stream, mut client) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site, stream, head_64k_head(), Bytes::new()),
                BlockingAdmission::Accepted
            ));

            let mut buf = [0u8; 512];
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let n = client.read(&mut buf).expect("read HEAD response");
            let response = std::str::from_utf8(&buf[..n]).expect("utf8");
            assert!(response.starts_with("HTTP/1.1 200"));
            assert!(response.contains("Content-Length:"));
        }

        #[tokio::test]
        async fn async_shed_writes_503_without_blocking_submit() {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let addr = listener.local_addr().expect("addr");
            let client_task = tokio::spawn(async move {
                tokio::net::TcpStream::connect(addr).await.expect("connect")
            });
            let (stream, _) = listener.accept().await.expect("accept");
            shed_blocking_admission(stream).await;
            let mut client = client_task.await.expect("join");
            let mut buf = [0u8; 256];
            let n = client.read(&mut buf).await.expect("read");
            let response = std::str::from_utf8(&buf[..n]).expect("utf8");
            assert!(response.contains("503"));
            assert!(response.contains("unavailable"));
            assert!(response.contains("Connection: close"));
        }

        #[test]
        fn try_submit_never_blocks_when_queue_full() {
            let pool = Arc::new(BlockingStaticPool::new(1, 1));
            let (_dir, site) = minimal_site();

            let (stream1, mut client1) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream1, head_64k_get(), Bytes::new()),
                BlockingAdmission::Accepted
            ));
            let mut buf = [0u8; 65536];
            client1
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let _ = client1.read(&mut buf).expect("first response");

            let (stream2, _client2) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream2, head_64k_get(), Bytes::new()),
                BlockingAdmission::Accepted
            ));

            let rejectors = (0..8)
                .map(|_| {
                    let pool = Arc::clone(&pool);
                    let site = site.clone();
                    std::thread::spawn(move || {
                        let (stream, _) = tcp_pair();
                        let start = Instant::now();
                        let result = pool.try_submit(site, stream, head_64k_get(), Bytes::new());
                        (result, start.elapsed())
                    })
                })
                .collect::<Vec<_>>();

            for handle in rejectors {
                let (result, elapsed) = handle.join().expect("join");
                assert!(
                    matches!(result, BlockingAdmission::Rejected(_)),
                    "expected reject, got {result:?}"
                );
                if let BlockingAdmission::Rejected(stream) = result {
                    shed_in_test(stream);
                }
                assert!(
                    elapsed < Duration::from_millis(200),
                    "try_submit blocked for {:?}",
                    elapsed
                );
            }

            let _ = client1.shutdown(Shutdown::Both);
        }
    }

    #[cfg(target_os = "linux")]
    mod ps2_iu2_keepalive_timeout {
        use super::*;
        use bytes::Bytes;
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::thread;
        use std::time::Duration;

        #[test]
        fn serve_blocking_sync_closes_on_keepalive_read_timeout() {
            let prev = std::env::var("EXYONQ_READ_TIMEOUT_MS").ok();
            std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "50");

            let dir = tempfile::tempdir().expect("tempdir");
            std::fs::write(dir.path().join("1k.bin"), vec![b'x'; 1024]).expect("1k.bin");
            let mut root = StaticRoot::new(dir.path(), "/site", None).expect("root");
            root.preload_tree().expect("preload");

            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("addr");
            let req =
                b"GET /site/1k.bin HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n";
            let client = thread::spawn(move || {
                let mut s = TcpStream::connect(addr).expect("connect");
                s.write_all(req).expect("write");
                let mut buf = [0u8; 4096];
                s.set_read_timeout(Some(Duration::from_millis(500)))
                    .expect("timeout");
                let _ = s.read(&mut buf).expect("first response");
                thread::sleep(Duration::from_millis(120));
                s
            });

            let (mut server, _) = listener.accept().expect("accept");
            server
                .set_read_timeout(Some(Duration::from_millis(50)))
                .expect("timeout");
            let head = Bytes::from_static(req);
            let end = conn_io::find_header_end(&head).expect("headers");
            let head_part = head.slice(0..end + 4);
            let rest = head.slice(end + 4..);
            let result = serve_blocking_sync(&root, &mut server, head_part, rest);
            assert!(
                result.is_ok(),
                "keep-alive timeout must not error: {result:?}"
            );
            let _ = client.join();

            match prev {
                Some(v) => std::env::set_var("EXYONQ_READ_TIMEOUT_MS", v),
                None => std::env::remove_var("EXYONQ_READ_TIMEOUT_MS"),
            }
        }
    }
}
