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

use crate::wire_io as conn_io;
use crate::{wire, StaticRoot};
use bytes::Bytes;
use std::io::{self, Result};
use std::sync::Arc;
use std::time::Instant;
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
use tracing::{debug, warn};

const MAX_HEADER: usize = 8192;

/// Cap020 then Cap019 on a precooked wire buffer (header+body).
/// Bench 200 templates stay unmodified on Cap020 Continue (benchmark integrity).
/// Returns the HTTP status actually written (200/206/304/416).
#[cfg(target_os = "linux")]
fn write_precooked_wire_with_range(fd: i32, head: &[u8], wire_bytes: &[u8]) -> Result<u16> {
    use crate::byte_range::{decide_range, range_header_from_raw_head, RangeDecision};
    use crate::conditional::{
        decide_conditional, header_from_raw_head, ConditionalDecision, StaticValidators,
    };
    let head_only = StaticRoot::is_head_wire_request(head);
    let Some((orig_hdr, body)) = wire::split_wire_header_body(wire_bytes) else {
        conn_io::write_response_fd(fd, wire_bytes)?;
        return Ok(200);
    };
    // Precooked buffers lack inode metadata; weak tag from length only (bench path).
    let validators = StaticValidators {
        etag: format!("W/\"exq-0-0-0-0-{}\"", body.len()),
        last_modified: None,
        mtime_secs: None,
    };
    let inm = header_from_raw_head(head, b"if-none-match");
    let ims = header_from_raw_head(head, b"if-modified-since");
    if decide_conditional(inm, ims, &validators) == ConditionalDecision::NotModified {
        conn_io::write_response_fd(
            fd,
            wire::not_modified_header(&validators.etag, validators.last_modified.as_deref())
                .as_ref(),
        )?;
        return Ok(304);
    }
    match decide_range(range_header_from_raw_head(head), body.len() as u64) {
        RangeDecision::Ignore => {
            if head_only {
                conn_io::write_response_fd(fd, orig_hdr)?;
            } else {
                conn_io::write_response_fd(fd, wire_bytes)?;
            }
            Ok(200)
        }
        RangeDecision::Unsatisfiable { full_length } => {
            conn_io::write_response_fd(fd, &wire::range_not_satisfiable_header(full_length))?;
            Ok(416)
        }
        RangeDecision::Satisfied(sel) => {
            let header = wire::partial_content_header_with_validators(
                sel.start,
                sel.end,
                sel.full_length,
                "application/octet-stream",
                Some(&validators.etag),
                validators.last_modified.as_deref(),
            );
            if head_only {
                conn_io::write_response_fd(fd, header.as_ref())?;
                return Ok(206);
            }
            let start = usize::try_from(sel.start)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "range start"))?;
            let end_excl = usize::try_from(sel.end.saturating_add(1))
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "range end"))?;
            if end_excl > body.len() || start >= end_excl {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "range exceeds wire body",
                ));
            }
            let mut out = Vec::with_capacity(header.len() + (end_excl - start));
            out.extend_from_slice(header.as_ref());
            out.extend_from_slice(&body[start..end_excl]);
            conn_io::write_response_fd(fd, &out)?;
            Ok(206)
        }
    }
}

fn wire_access_outcome(status: u16) -> &'static str {
    match status {
        304 => "static_wire_not_modified",
        206 => "static_wire_partial",
        416 => "static_wire_range_unsatisfiable",
        503 => "static_blocking_shed",
        404 => "static_wire_not_found",
        _ => "static_wire",
    }
}

fn header_read_timeout() -> std::time::Duration {
    crate::wire_io::header_read_timeout()
}

#[cfg(target_os = "linux")]
fn is_health_probe_head(head: &[u8]) -> bool {
    head.starts_with(b"GET /health ") || head.starts_with(b"HEAD /health ")
}

#[cfg(target_os = "linux")]
fn admit_owned_blocking_request(
    stream: &std::net::TcpStream,
    fd: i32,
    head: &[u8],
) -> Result<bool> {
    if is_health_probe_head(head) {
        return Ok(true);
    }
    let client_ip = stream.peer_addr()?.ip().to_string();
    match exyonq_module_api::wire_admit(&client_ip) {
        exyonq_module_api::WireAdmit::Allow => Ok(true),
        exyonq_module_api::WireAdmit::Reject429 { retry_after_secs } => {
            let response = exyonq_module_api::rate_limit_reject_wire(retry_after_secs);
            conn_io::write_response_fd(fd, &response)?;
            exyonq_module_api::wire_record_response(429);
            Ok(false)
        }
    }
}

#[cfg(target_os = "linux")]
async fn admit_owned_tcp_request(stream: &mut TcpStream, head: &[u8]) -> Result<bool> {
    if is_health_probe_head(head) {
        return Ok(true);
    }
    let client_ip = stream.peer_addr()?.ip().to_string();
    match exyonq_module_api::wire_admit(&client_ip) {
        exyonq_module_api::WireAdmit::Allow => Ok(true),
        exyonq_module_api::WireAdmit::Reject429 { retry_after_secs } => {
            let response = exyonq_module_api::rate_limit_reject_wire(retry_after_secs);
            stream.write_all(&response).await?;
            let _ = stream.shutdown().await;
            exyonq_module_api::wire_record_response(429);
            Ok(false)
        }
    }
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

/// Content-Length for wire keepalive body discard. `None` = invalid/ambiguous framing.
fn content_length_for_discard(head: &[u8]) -> Option<usize> {
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
        if header_line_name_eq(line, b"transfer-encoding") {
            return None;
        }
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

/// After headers at `hdr_end`, drop Content-Length body from the stack buffer (and stream if needed).
#[cfg(target_os = "linux")]
fn advance_stack_buf_past_request_body(
    buf: &mut [u8],
    len: &mut usize,
    hdr_end: usize,
    stream: &mut std::net::TcpStream,
) -> Result<()> {
    use std::io::Read;
    let Some(body_len) = content_length_for_discard(&buf[..hdr_end]) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ambiguous or unsupported request framing on wire",
        ));
    };
    let available = len.saturating_sub(hdr_end);
    let from_buf = body_len.min(available);
    let consume_end = hdr_end + from_buf;
    let mut remaining = body_len - from_buf;
    if consume_end >= *len {
        *len = 0;
    } else {
        buf.copy_within(consume_end..*len, 0);
        *len -= consume_end;
    }
    let mut tmp = [0u8; 4096];
    while remaining > 0 {
        let nread = remaining.min(tmp.len());
        let n = stream.read(&mut tmp[..nread])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eof while discarding request body",
            ));
        }
        remaining -= n;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn advance_stack_buf_past_request_body_async(
    buf: &mut [u8],
    len: &mut usize,
    hdr_end: usize,
    stream: &mut TcpStream,
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let Some(body_len) = content_length_for_discard(&buf[..hdr_end]) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ambiguous or unsupported request framing on wire",
        ));
    };
    let available = len.saturating_sub(hdr_end);
    let from_buf = body_len.min(available);
    let consume_end = hdr_end + from_buf;
    let mut remaining = body_len - from_buf;
    if consume_end >= *len {
        *len = 0;
    } else {
        buf.copy_within(consume_end..*len, 0);
        *len -= consume_end;
    }
    let mut tmp = [0u8; 4096];
    while remaining > 0 {
        let nread = remaining.min(tmp.len());
        let n = stream.read(&mut tmp[..nread]).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eof while discarding request body",
            ));
        }
        remaining -= n;
    }
    Ok(())
}

/// Linux: bounded OS thread pool for blocking static keep-alive.
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
    /// Cap041: connection admission ownership held until job completes.
    _lifecycle_hold: Option<Box<dyn Send>>,
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
        lifecycle_hold: Option<Box<dyn Send>>,
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
            _lifecycle_hold: lifecycle_hold,
        };

        match self.tx.try_send(job) {
            Ok(()) => {
                self.admission_accepted.fetch_add(1, Ordering::Relaxed);
                BlockingAdmission::Accepted
            }
            Err(mpsc::TrySendError::Full(job)) => {
                drop(job._permit);
                drop(job._lifecycle_hold);
                self.queue_full.fetch_add(1, Ordering::Relaxed);
                self.admission_rejected.fetch_add(1, Ordering::Relaxed);
                debug!("static blocking admission rejected: queue full");
                BlockingAdmission::Rejected(job.stream)
            }
            Err(mpsc::TrySendError::Disconnected(job)) => {
                drop(job._permit);
                drop(job._lifecycle_hold);
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
    let lifecycle_hold = exyonq_module_api::static_wire::take_lifecycle_hold();
    blocking_static_pool().try_submit(site, stream, first_head, carry, lifecycle_hold)
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
            if let Err(err) = shed_blocking_admission(stream).await {
                warn!(%err, "static blocking admission shed task: reject response write failed");
            }
        });
    }
}

#[cfg(target_os = "linux")]
/// Admission counters (ADR-025 names): `static_blocking_admission_accepted`,
/// `static_blocking_admission_rejected`, `static_blocking_queue_full`, `static_blocking_inflight`.
pub fn static_blocking_admission_metrics() -> (u64, u64, u64, usize) {
    blocking_static_pool().metrics()
}

/// Process-local count of failed STATIC_BLOCKING_ADMISSION_REJECTED response writes.
#[cfg(target_os = "linux")]
static SHED_BLOCKING_ADMISSION_WRITE_ERRORS: AtomicU64 = AtomicU64::new(0);

/// Snapshot of failed shed reject-response writes (test / ops observation).
#[cfg(target_os = "linux")]
pub fn shed_blocking_admission_write_errors() -> u64 {
    SHED_BLOCKING_ADMISSION_WRITE_ERRORS.load(Ordering::Relaxed)
}

/// Write the 503 admission-reject response after blocking-pool rejection.
///
/// `Ok(())` means the reject bytes were handed to the socket write path (delivered intent).
/// `Err` means the write failed — callers must not record `static_blocking_shed` as a
/// successful delivery.
#[cfg(target_os = "linux")]
pub async fn shed_blocking_admission(mut stream: TcpStream) -> io::Result<()> {
    if let Err(err) = stream.write_all(STATIC_BLOCKING_ADMISSION_REJECTED).await {
        SHED_BLOCKING_ADMISSION_WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
        warn!(%err, "static blocking admission reject response write failed");
        return Err(err);
    }
    let _ = stream.shutdown().await;
    Ok(())
}

/// Cap061: emit wire access (also used from epoll health probe writer).
#[cfg(target_os = "linux")]
pub(crate) fn notify_wire_access_for_hooks(
    head: &[u8],
    status: u16,
    outcome: &'static str,
    started: Instant,
) {
    notify_wire_access(head, status, outcome, started);
}

fn notify_wire_access(head: &[u8], status: u16, outcome: &'static str, started: Instant) {
    // Cap061/S2: skip head copy + hook when access events are disabled.
    if !exyonq_module_api::static_wire::access_notices_enabled() {
        return;
    }
    exyonq_module_api::static_wire::notify_access(
        exyonq_module_api::static_wire::StaticWireAccessNotice {
            head: Bytes::copy_from_slice(head),
            status,
            outcome,
            duration_ms: started.elapsed().as_millis() as u64,
        },
    );
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
            // Propagate write failure — do not report Ok as if 503 was delivered.
            shed_blocking_admission(stream).await
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

        carry = discard_request_body_blocking(stream, head.as_ref(), carry)?;
        if !admit_owned_blocking_request(stream, fd, head.as_ref())? {
            return Ok(());
        }

        let started = Instant::now();
        match write_static_response_blocking(site, fd, head.as_ref())? {
            None => {
                if head.is_empty() {
                    break;
                }
                conn_io::write_response_fd(fd, &wire::NOT_FOUND_KEEP_ALIVE)?;
                if !is_health_probe_head(head.as_ref()) {
                    exyonq_module_api::wire_record_response(404);
                }
                notify_wire_access(head.as_ref(), 404, "static_wire_not_found", started);
                continue;
            }
            Some(status) => {
                if !is_health_probe_head(head.as_ref()) {
                    exyonq_module_api::wire_record_response(status);
                }
                notify_wire_access(head.as_ref(), status, wire_access_outcome(status), started);
            }
        }
        // Cap061 LA-008: no filename-keyed keep-alive tight loops. Subsequent
        // requests continue this notified loop (health/metrics only on wire).
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_static_response_blocking(_site: &StaticRoot, fd: i32, head: &[u8]) -> Result<Option<u16>> {
    if head.starts_with(b"GET /health ") || head.starts_with(b"HEAD /health ") {
        conn_io::write_response_fd(fd, wire::health_keep_alive_for_head(head))?;
        return Ok(Some(200));
    }
    // LA-CAP054-008: wire must not serve OpenMetrics. /metrics stays an ops-probe
    // eligibility hit so sendfile does not treat it as a static asset; response is 404.
    if head.starts_with(b"GET /metrics ") || head.starts_with(b"GET /metrics\r") {
        return Ok(None);
    }
    Ok(None)
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
    // Cap061 LA-008: no filename-keyed early divert (sendfile/route bench loops).
    // Wire serves /health only; /metrics is not an OpenMetrics authority (LA-CAP054-008).
    // Site files use Hyper/static.
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

        carry = discard_request_body_async(&mut stream, head.as_ref(), carry).await?;

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

        #[cfg(target_os = "linux")]
        if !admit_owned_tcp_request(&mut stream, head.as_ref()).await? {
            return Ok(());
        }

        let started = Instant::now();
        let (next_stream, status) = write_static_response_tcp(&site, stream, head.as_ref()).await?;
        stream = next_stream;
        match status {
            None => {
                if head.is_empty() {
                    break;
                }
                write_wire_fast(&mut stream, &wire::NOT_FOUND_KEEP_ALIVE).await?;
                #[cfg(target_os = "linux")]
                if !is_health_probe_head(head.as_ref()) {
                    exyonq_module_api::wire_record_response(404);
                }
                notify_wire_access(head.as_ref(), 404, "static_wire_not_found", started);
            }
            Some(code) => {
                #[cfg(target_os = "linux")]
                if !is_health_probe_head(head.as_ref()) {
                    exyonq_module_api::wire_record_response(code);
                }
                notify_wire_access(head.as_ref(), code, wire_access_outcome(code), started);
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

        carry = discard_request_body_async(stream, head.as_ref(), carry).await?;

        let started = Instant::now();
        match write_static_response_async(&site, stream, head.as_ref()).await? {
            None => {
                if head.is_empty() {
                    break;
                }
                conn_io::write_all_async(stream, &wire::NOT_FOUND_KEEP_ALIVE).await?;
                notify_wire_access(head.as_ref(), 404, "static_wire_not_found", started);
            }
            Some(code) => {
                notify_wire_access(head.as_ref(), code, wire_access_outcome(code), started);
            }
        }
    }
    Ok(())
}

async fn write_static_response_tcp(
    _site: &StaticRoot,
    stream: TcpStream,
    head: &[u8],
) -> Result<(TcpStream, Option<u16>)> {
    if head.starts_with(b"GET /health ") || head.starts_with(b"HEAD /health ") {
        let mut stream = stream;
        write_wire_fast(&mut stream, wire::health_keep_alive_for_head(head)).await?;
        return Ok((stream, Some(200)));
    }
    #[cfg(target_os = "linux")]
    if head.starts_with(b"GET /metrics ") || head.starts_with(b"GET /metrics\r") {
        // LA-CAP054-008: no unauthenticated wire OpenMetrics (falls through → 404).
        return Ok((stream, None));
    }

    Ok((stream, None))
}

async fn write_static_response_async<S: AsyncWrite + Unpin>(
    _site: &StaticRoot,
    stream: &mut S,
    head: &[u8],
) -> Result<Option<u16>> {
    if head.starts_with(b"GET /health ") || head.starts_with(b"HEAD /health ") {
        conn_io::write_all_async(stream, wire::health_keep_alive_for_head(head)).await?;
        return Ok(Some(200));
    }
    #[cfg(target_os = "linux")]
    if head.starts_with(b"GET /metrics ") || head.starts_with(b"GET /metrics\r") {
        // LA-CAP054-008: no unauthenticated wire OpenMetrics (falls through → 404).
        return Ok(None);
    }
    Ok(None)
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

/// Drop request-body bytes so they are never parsed as the next request on keepalive.
#[cfg(target_os = "linux")]
fn discard_request_body_blocking(
    stream: &mut std::net::TcpStream,
    head: &[u8],
    mut carry: Bytes,
) -> Result<Bytes> {
    use std::io::Read;
    let Some(mut need) = content_length_for_discard(head) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ambiguous or unsupported request framing on wire",
        ));
    };
    if need == 0 {
        return Ok(carry);
    }
    if carry.len() >= need {
        return Ok(carry.slice(need..));
    }
    need -= carry.len();
    carry = Bytes::new();
    let mut buf = [0u8; 4096];
    while need > 0 {
        let nread = need.min(buf.len());
        let n = stream.read(&mut buf[..nread])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eof while discarding request body",
            ));
        }
        need -= n;
    }
    Ok(carry)
}

async fn discard_request_body_async<S: AsyncRead + Unpin>(
    stream: &mut S,
    head: &[u8],
    mut carry: Bytes,
) -> Result<Bytes> {
    use tokio::io::AsyncReadExt;
    let Some(mut need) = content_length_for_discard(head) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ambiguous or unsupported request framing on wire",
        ));
    };
    if need == 0 {
        return Ok(carry);
    }
    if carry.len() >= need {
        return Ok(carry.slice(need..));
    }
    need -= carry.len();
    carry = Bytes::new();
    let mut buf = [0u8; 4096];
    while need > 0 {
        let nread = need.min(buf.len());
        let n = stream.read(&mut buf[..nread]).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eof while discarding request body",
            ));
        }
        need -= n;
    }
    Ok(carry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_length_for_discard_reads_case_insensitive() {
        let head = b"GET /health HTTP/1.1\r\nHost: x\r\ncontent-length: 5\r\n\r\n";
        assert_eq!(content_length_for_discard(head), Some(5));
        let head0 = b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(content_length_for_discard(head0), Some(0));
        let bad = b"GET /health HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(content_length_for_discard(bad), None);
    }

    #[test]
    fn discard_body_from_carry_prevents_next_request_desync() {
        let body_as_next = b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n";
        let cl = body_as_next.len();
        let head = Bytes::from(format!(
            "GET /health HTTP/1.1\r\nHost: x\r\nContent-Length: {cl}\r\n\r\n"
        ));
        let carry = Bytes::copy_from_slice(body_as_next);
        let need = content_length_for_discard(head.as_ref()).expect("cl");
        assert_eq!(need, cl);
        let after = carry.slice(need..);
        assert!(after.is_empty());
    }

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

        const STATIC_NOT_FOUND_RESPONSE: &[u8] = b"HTTP/1.1 404 Not Found\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Length: 9\r\n\
Connection: keep-alive\r\n\
\r\n\
not found";
        const ADMISSION_REJECTED_RESPONSE: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\n\
Content-Type: text/plain\r\n\
Content-Length: 11\r\n\
Connection: close\r\n\
\r\n\
unavailable";

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

        fn assert_exact_response(client: &mut std::net::TcpStream, expected: &[u8], context: &str) {
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set read timeout");
            let mut actual = vec![0u8; expected.len()];
            client.read_exact(&mut actual).expect(context);
            assert_eq!(actual, expected, "{context}");
        }

        fn shed_in_test(stream: TcpStream) {
            test_runtime()
                .block_on(shed_blocking_admission(stream))
                .expect("shed write should succeed with live peer");
        }

        #[test]
        fn admission_reject_is_immediate_when_saturated() {
            let pool = BlockingStaticPool::new(1, 1);
            let (_dir, site) = minimal_site();

            let (stream1, mut client1) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream1, head_64k_get(), Bytes::new(), None),
                BlockingAdmission::Accepted
            ));

            assert_exact_response(
                &mut client1,
                STATIC_NOT_FOUND_RESPONSE,
                "admitted blocking static GET must return the health-only shim 404",
            );

            let (stream2, _client2) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream2, head_64k_get(), Bytes::new(), None),
                BlockingAdmission::Accepted
            ));

            let (stream3, mut client3) = tcp_pair();
            let start = Instant::now();
            let result = pool.try_submit(site, stream3, head_64k_get(), Bytes::new(), None);
            assert!(
                matches!(result, BlockingAdmission::Rejected(_)),
                "expected reject, got {result:?}"
            );
            assert!(
                start.elapsed() < Duration::from_millis(200),
                "rejection blocked caller for {:?}",
                start.elapsed()
            );
            assert_eq!(
                pool.metrics(),
                (2, 1, 0, 2),
                "two admitted jobs and one capacity rejection must be classified exactly"
            );
            if let BlockingAdmission::Rejected(stream) = result {
                shed_in_test(stream);
            }

            assert_exact_response(
                &mut client3,
                ADMISSION_REJECTED_RESPONSE,
                "rejected admission must receive the exact close-framed 503",
            );

            let _ = client1.shutdown(Shutdown::Both);
        }

        #[test]
        fn admission_metrics_classify_accept_and_reject() {
            let pool = BlockingStaticPool::new(1, 0);
            let (_dir, site) = minimal_site();
            let (stream, mut client) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream, head_64k_get(), Bytes::new(), None),
                BlockingAdmission::Accepted
            ));

            let (stream2, mut client2) = tcp_pair();
            let result = pool.try_submit(site, stream2, head_64k_get(), Bytes::new(), None);
            assert!(matches!(result, BlockingAdmission::Rejected(_)));
            if let BlockingAdmission::Rejected(stream) = result {
                shed_in_test(stream);
            }

            assert_eq!(
                pool.metrics(),
                (1, 1, 0, 1),
                "one admitted job and one admission-cap rejection must have exact counters"
            );
            assert_exact_response(
                &mut client,
                STATIC_NOT_FOUND_RESPONSE,
                "admitted blocking static GET must return the health-only shim 404",
            );
            assert_exact_response(
                &mut client2,
                ADMISSION_REJECTED_RESPONSE,
                "rejected admission must receive the exact close-framed 503",
            );
        }

        #[test]
        fn admitted_static_head_uses_blocking_shim_not_found_wire() {
            let pool = BlockingStaticPool::new(2, 2);
            let (_dir, site) = minimal_site();
            let (stream, mut client) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site, stream, head_64k_head(), Bytes::new(), None),
                BlockingAdmission::Accepted
            ));

            assert_exact_response(
                &mut client,
                STATIC_NOT_FOUND_RESPONSE,
                "blocking shim uses its deterministic generic miss wire for static HEAD",
            );
            assert_eq!(
                pool.metrics(),
                (1, 0, 0, 1),
                "admitted HEAD must remain accounted until keep-alive completion"
            );
        }

        #[tokio::test]
        async fn async_shed_writes_503_without_blocking_submit() {
            let before = shed_blocking_admission_write_errors();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let addr = listener.local_addr().expect("addr");
            let client_task = tokio::spawn(async move {
                tokio::net::TcpStream::connect(addr).await.expect("connect")
            });
            let (stream, _) = listener.accept().await.expect("accept");
            shed_blocking_admission(stream)
                .await
                .expect("shed write Ok with live peer");
            let mut client = client_task.await.expect("join");
            let mut buf = [0u8; 256];
            let n = client.read(&mut buf).await.expect("read");
            let response = std::str::from_utf8(&buf[..n]).expect("utf8");
            assert!(response.contains("503"));
            assert!(response.contains("unavailable"));
            assert!(response.contains("Connection: close"));
            assert_eq!(
                shed_blocking_admission_write_errors(),
                before,
                "successful shed must not count write failure"
            );
        }

        /// LET-180: after Reject is selected, real peer RST makes write_all fail;
        /// failure must be observed (Err + counter) — not silent Ok.
        #[tokio::test]
        async fn shed_write_failure_returns_err_on_real_rst() {
            use std::os::fd::AsRawFd;

            let before = shed_blocking_admission_write_errors();
            let mut observed = before;
            for _ in 0..80 {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind");
                let addr = listener.local_addr().expect("addr");
                let accept = tokio::spawn(async move {
                    let (stream, _) = listener.accept().await.expect("accept");
                    shed_blocking_admission(stream).await
                });
                let std_stream = std::net::TcpStream::connect(addr).expect("connect");
                std_stream.set_nodelay(true).ok();
                let linger = libc::linger {
                    l_onoff: 1,
                    l_linger: 0,
                };
                unsafe {
                    libc::setsockopt(
                        std_stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_LINGER,
                        &linger as *const _ as *const libc::c_void,
                        std::mem::size_of_val(&linger) as libc::socklen_t,
                    );
                }
                drop(std_stream);
                let res = tokio::time::timeout(Duration::from_secs(2), accept)
                    .await
                    .expect("join timeout")
                    .expect("join");
                if res.is_err() {
                    observed = shed_blocking_admission_write_errors();
                    if observed > before {
                        break;
                    }
                }
            }
            assert!(
                observed > before,
                "LET-180: reject write failure must be observed (before={before} after={observed})"
            );
        }

        #[test]
        fn try_submit_never_blocks_when_queue_full() {
            let pool = Arc::new(BlockingStaticPool::new(1, 1));
            let (_dir, site) = minimal_site();

            let (stream1, mut client1) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream1, head_64k_get(), Bytes::new(), None),
                BlockingAdmission::Accepted
            ));
            let mut buf = [0u8; 65536];
            client1
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let _ = client1.read(&mut buf).expect("first response");

            let (stream2, _client2) = tcp_pair();
            assert!(matches!(
                pool.try_submit(site.clone(), stream2, head_64k_get(), Bytes::new(), None),
                BlockingAdmission::Accepted
            ));

            let rejectors = (0..8)
                .map(|_| {
                    let pool = Arc::clone(&pool);
                    let site = site.clone();
                    std::thread::spawn(move || {
                        let (stream, _) = tcp_pair();
                        let start = Instant::now();
                        let result =
                            pool.try_submit(site, stream, head_64k_get(), Bytes::new(), None);
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
            crate::reset_header_read_timeout_cache_for_tests();
            std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "50");
            crate::reset_header_read_timeout_cache_for_tests();

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
            crate::reset_header_read_timeout_cache_for_tests();
        }
    }
}
