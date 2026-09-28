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
//! Map HTTP GET (+ optional small CL body) → FastCGI PARAMS+STDIN → CGI response bytes.
//!
//! SCRIPT_FILENAME = document_root + normalized path under root (reject `..`, NUL).
//! Never from client headers.

use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use exyonq_cfd_gen::CompiledFcgiPool;
use exyonq_fastcgi_wire::{
    exchange_once, parse_cgi_stdout, CommitStage, ExchangeAttempt, ExchangeError,
    MinForwardRequest, ParamsError, FCGI_KEEP_CONN,
};

use crate::fcgi_pool::{FcgiPoolMap, FcgiStream, Probe};
use crate::obs::ObsHubHandle;

/// Enforce `CompiledFcgiPool.total_timeout_ms` as an absolute exchange deadline.
///
/// Per-op socket timeouts are clamped to `min(configured, remaining_total)`.
struct BudgetedStream<'a> {
    inner: &'a mut FcgiStream,
    deadline: Instant,
    read_cap: Duration,
    write_cap: Duration,
}

impl BudgetedStream<'_> {
    fn remaining(&self) -> Option<Duration> {
        let now = Instant::now();
        if now >= self.deadline {
            return None;
        }
        Some(self.deadline.saturating_duration_since(now))
    }
}

impl Read for BudgetedStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(rem) = self.remaining() else {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "fcgi total_timeout_ms exceeded",
            ));
        };
        let t = rem.min(self.read_cap).max(Duration::from_millis(1));
        self.inner.set_read_timeout(Some(t))?;
        match self.inner.read(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                if self.remaining().is_none()
                    || matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    )
                {
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "fcgi total_timeout_ms exceeded",
                    ))
                } else {
                    Err(e)
                }
            }
        }
    }
}

impl Write for BudgetedStream<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(rem) = self.remaining() else {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "fcgi total_timeout_ms exceeded",
            ));
        };
        let t = rem.min(self.write_cap).max(Duration::from_millis(1));
        self.inner.set_write_timeout(Some(t))?;
        match self.inner.write(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                if self.remaining().is_none()
                    || matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    )
                {
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "fcgi total_timeout_ms exceeded",
                    ))
                } else {
                    Err(e)
                }
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let Some(rem) = self.remaining() else {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "fcgi total_timeout_ms exceeded",
            ));
        };
        let t = rem.min(self.write_cap).max(Duration::from_millis(1));
        self.inner.set_write_timeout(Some(t))?;
        match self.inner.flush() {
            Ok(()) => Ok(()),
            Err(e) => {
                if self.remaining().is_none()
                    || matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    )
                {
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "fcgi total_timeout_ms exceeded",
                    ))
                } else {
                    Err(e)
                }
            }
        }
    }
}

fn exchange_with_total_budget(
    stream: &mut FcgiStream,
    pool: &CompiledFcgiPool,
    params: &[(String, String)],
    stdin: &[u8],
) -> ExchangeAttempt {
    let total_ms = u64::from(pool.total_timeout_ms.max(1));
    let deadline = Instant::now() + Duration::from_millis(total_ms);
    let read_cap = Duration::from_millis(u64::from(pool.read_timeout_ms.max(1)));
    let write_cap = Duration::from_millis(u64::from(pool.write_timeout_ms.max(1)));
    let mut budgeted = BudgetedStream {
        inner: stream,
        deadline,
        read_cap,
        write_cap,
    };
    exchange_once(&mut budgeted, 1, true, params, stdin)
}

/// MVP bound on request body for CFD FastCGI.
pub const MAX_FCGI_HTTP_BODY: u64 = 64 * 1024;

#[derive(Debug)]
pub enum FcgiExecError {
    MethodNotAllowed,
    BodyTooLarge,
    ScriptPath,
    Params(#[allow(dead_code)] ParamsError),
    Connect,
    Timeout,
    Protocol(#[allow(dead_code)] ExchangeError),
    CgiParse,
}

#[derive(Debug)]
pub struct FcgiExecOk {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    #[allow(dead_code)]
    pub pool_hit: bool,
    // ADR-041: removed dead always-true `reuse_ok`. Checkin SSOT = FcgiPoolMap::put.
}

/// Build trusted SCRIPT_FILENAME under document_root.
pub fn script_filename_under_root(
    document_root: &str,
    request_path: &str,
) -> Result<(String, String), FcgiExecError> {
    if document_root.contains('\0') || request_path.contains('\0') {
        return Err(FcgiExecError::ScriptPath);
    }
    let root = Path::new(document_root);
    if !root.is_absolute() {
        return Err(FcgiExecError::ScriptPath);
    }
    let path = if request_path.is_empty() || request_path == "/" {
        "/index.php"
    } else {
        request_path
    };
    let rel = path.trim_start_matches('/');
    let mut joined = PathBuf::from(root);
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(s) => joined.push(s),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(FcgiExecError::ScriptPath);
            }
        }
    }
    // Ensure still under root (no escape via odd components).
    if !joined.starts_with(root) {
        return Err(FcgiExecError::ScriptPath);
    }
    let script_filename = joined.to_string_lossy().into_owned();
    if script_filename.contains('\0') {
        return Err(FcgiExecError::ScriptPath);
    }
    let script_name = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    Ok((script_filename, script_name))
}

/// Final activation before BEGIN: always probe, then restore pool timeouts (ADR-039).
///
/// On Dirty while still NotStarted: discard and reconnect at most once, then re-probe.
fn prepare_stream_for_begin(
    mut stream: FcgiStream,
    mut pool_hit: bool,
    pool: &CompiledFcgiPool,
    obs: &ObsHubHandle,
) -> Result<(FcgiStream, bool), FcgiExecError> {
    let mut activation_reconnects: u8 = 0;
    loop {
        if FcgiPoolMap::activate_before_begin(&mut stream) == Probe::Clean {
            break;
        }
        FcgiPoolMap::discard(stream);
        obs.fcgi_pool_discard
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if activation_reconnects >= 1 {
            obs.fcgi_connect_fail
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Err(FcgiExecError::Connect);
        }
        activation_reconnects += 1;
        stream = match FcgiPoolMap::connect(pool) {
            Ok(s) => s,
            Err(_) => {
                obs.fcgi_connect_fail
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Err(FcgiExecError::Connect);
            }
        };
        pool_hit = false;
    }
    if FcgiPoolMap::apply_timeouts(&stream, pool).is_err() {
        FcgiPoolMap::discard(stream);
        obs.fcgi_pool_discard
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Err(FcgiExecError::Connect);
    }
    Ok((stream, pool_hit))
}

/// Execute one FastCGI request using shard-local pool.
#[allow(clippy::too_many_arguments)]
pub fn execute_get(
    pools: &mut FcgiPoolMap,
    pool: &CompiledFcgiPool,
    generation: u64,
    method: &str,
    request_uri: &str,
    path_for_script: &str,
    query: Option<&str>,
    server_name: &str,
    server_port: u16,
    http_headers: &[(String, String)],
    stdin: &[u8],
    declared_cl: Option<usize>,
    obs: &ObsHubHandle,
) -> Result<FcgiExecOk, FcgiExecError> {
    obs.fcgi_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    if !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("HEAD") {
        // MVP: GET (+ optional small body) — POST deferred.
        if method.eq_ignore_ascii_case("POST") && !stdin.is_empty() {
            // allow small POST for CL body tests
        } else if !method.eq_ignore_ascii_case("POST") {
            return Err(FcgiExecError::MethodNotAllowed);
        }
    }
    if stdin.len() as u64 > MAX_FCGI_HTTP_BODY {
        return Err(FcgiExecError::BodyTooLarge);
    }
    if let Some(cl) = declared_cl {
        if cl != stdin.len() {
            obs.fcgi_protocol_fail
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Err(FcgiExecError::Params(ParamsError::ContentLengthMismatch {
                expected: cl,
                got: stdin.len(),
            }));
        }
    }

    let (script_filename, script_name) =
        script_filename_under_root(&pool.document_root, path_for_script)?;

    let mut req = MinForwardRequest::get(request_uri, &script_name, &script_filename);
    req.document_root = pool.document_root.clone();
    req.request_method = method.to_ascii_uppercase();
    req.server_name = server_name.to_string();
    req.server_port = server_port;
    req.query_string = query.map(str::to_string);
    req.http_headers = http_headers.to_vec();
    req.stdin = stdin.to_vec();
    req.declared_content_length = declared_cl;
    if !stdin.is_empty() && req.content_type.is_none() {
        if let Some((_, v)) = http_headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))
        {
            req.content_type = Some(v.clone());
        } else {
            req.content_type = Some("application/octet-stream".into());
        }
    }

    let params = req.to_fcgi_params().map_err(|e| {
        obs.fcgi_protocol_fail
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        FcgiExecError::Params(e)
    })?;

    let (stream, pool_hit) = match pools.take(pool, generation) {
        Some((s, true)) => {
            obs.fcgi_pool_hit
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            (s, true)
        }
        _ => {
            obs.fcgi_pool_miss
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match FcgiPoolMap::connect(pool) {
                Ok(s) => (s, false),
                Err(_) => {
                    obs.fcgi_connect_fail
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Err(FcgiExecError::Connect);
                }
            }
        }
    };

    // ADR-039: activate probe → apply_timeouts → exchange (always, every entry).
    let (mut stream, pool_hit) = prepare_stream_for_begin(stream, pool_hit, pool, obs)?;

    let mut attempt = exchange_with_total_budget(&mut stream, pool, &params, stdin);
    // One safe retry only at NotStarted (e.g. dead socket before any write).
    if attempt.result.is_err() && attempt.stage.allows_safe_retry() {
        FcgiPoolMap::discard(stream);
        obs.fcgi_pool_discard
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let s = match FcgiPoolMap::connect(pool) {
            Ok(s) => s,
            Err(_) => {
                obs.fcgi_connect_fail
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Err(FcgiExecError::Connect);
            }
        };
        let prepared = prepare_stream_for_begin(s, false, pool, obs)?;
        stream = prepared.0;
        attempt = exchange_with_total_budget(&mut stream, pool, &params, stdin);
    }

    match attempt.result {
        Ok(resp) => match parse_cgi_stdout(&resp.stdout) {
            Ok(cgi) => {
                pools.put(pool, stream, generation);
                obs.fcgi_success
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let _ = FCGI_KEEP_CONN;
                let _ = CommitStage::NotStarted;
                Ok(FcgiExecOk {
                    status: cgi.status,
                    headers: cgi.headers,
                    body: cgi.body,
                    pool_hit,
                })
            }
            Err(_) => {
                obs.fcgi_protocol_fail
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                FcgiPoolMap::discard(stream);
                obs.fcgi_pool_discard
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Err(FcgiExecError::CgiParse)
            }
        },
        Err(ExchangeError::TrailingData) => {
            obs.fcgi_protocol_fail
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            FcgiPoolMap::discard(stream);
            obs.fcgi_pool_discard
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(FcgiExecError::Protocol(ExchangeError::TrailingData))
        }
        Err(ExchangeError::Timeout) => {
            obs.fcgi_timeout
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            FcgiPoolMap::discard(stream);
            obs.fcgi_pool_discard
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(FcgiExecError::Timeout)
        }
        Err(e) => {
            obs.fcgi_protocol_fail
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            FcgiPoolMap::discard(stream);
            obs.fcgi_pool_discard
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(FcgiExecError::Protocol(e))
        }
    }
}

/// Format CGI result as HTTP/1.1 response bytes for the client.
///
/// When `bodyless` is true (HEAD, or status 204/304), no message body is written.
/// Content-Length is forced to 0 to match the CFD proxy bodyless path.
pub fn format_client_response(
    status: u16,
    reason: &str,
    headers: &[(String, String)],
    body: &[u8],
    close: bool,
    bodyless: bool,
) -> Vec<u8> {
    let wire_body: &[u8] = if bodyless { &[] } else { body };
    let mut out = Vec::with_capacity(256 + wire_body.len());
    out.extend_from_slice(format!("HTTP/1.1 {status} {reason}\r\n").as_bytes());
    for (n, v) in headers {
        if n.eq_ignore_ascii_case("content-length") {
            continue;
        }
        out.extend_from_slice(n.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("Content-Length: {}\r\n", wire_body.len()).as_bytes());
    if close {
        out.extend_from_slice(b"Connection: close\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(wire_body);
    out
}

/// True when HTTP/1.1 forbids a response message body.
#[inline]
pub fn response_must_be_bodyless(request_head: bool, status: u16) -> bool {
    request_head || matches!(status, 204 | 304)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{Duration, Instant};

    use exyonq_cfd_gen::{CompiledFcgiPool, FcgiTransport};
    use exyonq_fastcgi_wire::{
        encode_record_frame, parse_record, CommitStage, FCGI_END_REQUEST, FCGI_PARAMS,
        FCGI_REQUEST_COMPLETE, FCGI_STDIN, FCGI_STDOUT, RECORD_HEADER_LEN,
    };

    use crate::obs::ObsHub;

    fn pool_at(addr: std::net::SocketAddr) -> CompiledFcgiPool {
        CompiledFcgiPool {
            id: 1,
            transport: FcgiTransport::Tcp(addr),
            document_root: "/var/www".into(),
            script_suffix: String::new(),
            max_connections: 4,
            idle_timeout_ms: 60_000,
            connect_timeout_ms: 2_000,
            read_timeout_ms: 5_000,
            write_timeout_ms: 5_000,
            total_timeout_ms: 10_000,
            directory_index: Vec::new(),
            front_controller: None,
        }
    }

    fn serve_one_fcgi_ok(mut sock: std::net::TcpStream) {
        let mut buf = Vec::new();
        let mut scratch = [0u8; 4096];
        let mut saw_empty_stdin = false;
        while !saw_empty_stdin {
            let n = match sock.read(&mut scratch) {
                Ok(0) => return,
                Ok(n) => n,
                Err(_) => return,
            };
            buf.extend_from_slice(&scratch[..n]);
            while buf.len() >= RECORD_HEADER_LEN {
                let cl = u16::from_be_bytes([buf[4], buf[5]]) as usize;
                let pad = buf[6] as usize;
                let flen = RECORD_HEADER_LEN + cl + pad;
                if buf.len() < flen {
                    break;
                }
                let frame = buf[..flen].to_vec();
                buf.drain(..flen);
                let parsed = parse_record(&frame).unwrap();
                let _ = FCGI_PARAMS;
                if parsed.header.record_type == FCGI_STDIN && parsed.content.is_empty() {
                    saw_empty_stdin = true;
                    break;
                }
            }
        }
        let stdout = encode_record_frame(
            1,
            FCGI_STDOUT,
            b"Status: 200\r\nContent-Type: text/plain\r\n\r\nok-race01",
        )
        .unwrap();
        let mut end = [0u8; 8];
        end[4] = FCGI_REQUEST_COMPLETE;
        let end_frame = encode_record_frame(1, FCGI_END_REQUEST, &end).unwrap();
        let _ = sock.write_all(&stdout);
        let _ = sock.write_all(&end_frame);
    }

    #[test]
    fn rejects_dotdot_and_builds_under_root() {
        assert!(script_filename_under_root("/var/www", "/../etc/passwd").is_err());
        let (sf, sn) = script_filename_under_root("/var/www", "/app/index.php").unwrap();
        assert_eq!(sf, "/var/www/app/index.php");
        assert_eq!(sn, "/app/index.php");
    }

    #[test]
    fn head_and_204_304_omit_wire_body() {
        let headers = vec![("Content-Type".into(), "text/plain".into())];
        let body = b"secret-entity";
        let head = format_client_response(200, "OK", &headers, body, false, true);
        let cl = b"Content-Length:";
        assert!(
            head.windows(cl.len()).any(|w| w == cl),
            "HEAD/bodyless must advertise Content-Length"
        );
        assert!(head.ends_with(b"\r\n\r\n"));
        assert!(!head.windows(body.len()).any(|w| w == body));

        let n204 = format_client_response(204, "No Content", &headers, body, false, true);
        assert!(!n204.windows(body.len()).any(|w| w == body));
        assert!(response_must_be_bodyless(false, 204));
        assert!(response_must_be_bodyless(false, 304));
        assert!(response_must_be_bodyless(true, 200));
        assert!(!response_must_be_bodyless(false, 200));
    }

    #[test]
    fn prepare_eof_after_checkout_reconnects_not_begin_on_dead() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let b2 = barrier.clone();
        thread::spawn(move || {
            let (_peer, _) = listener.accept().unwrap();
            b2.wait();
            drop(_peer);
            if let Ok((peer2, _)) = listener.accept() {
                serve_one_fcgi_ok(peer2);
            }
        });
        let pool = pool_at(addr);
        let mut stream = FcgiPoolMap::connect(&pool).unwrap();
        assert_eq!(FcgiPoolMap::probe(&mut stream), Probe::Clean);
        barrier.wait();
        thread::sleep(Duration::from_millis(30));
        let obs = ObsHub::new();
        let before_discard = obs.fcgi_pool_discard.load(Ordering::Relaxed);
        let (mut stream, pool_hit) =
            prepare_stream_for_begin(stream, true, &pool, &obs).expect("reconnect");
        assert!(!pool_hit, "EOF activation must clear pool_hit");
        assert!(obs.fcgi_pool_discard.load(Ordering::Relaxed) > before_discard);
        let params = vec![
            ("SCRIPT_FILENAME".into(), "/var/www/x.php".into()),
            ("REQUEST_METHOD".into(), "GET".into()),
        ];
        let attempt = exchange_with_total_budget(&mut stream, &pool, &params, &[]);
        let resp = attempt.result.expect("exchange on fresh socket");
        assert!(resp.stdout.windows(9).any(|w| w == b"ok-race01"));
        assert_eq!(attempt.stage, CommitStage::ResponseStarted);
    }

    #[test]
    fn prepare_pending_byte_discards_never_begins_on_dirty() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let b2 = barrier.clone();
        let saw_begin = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_begin2 = saw_begin.clone();
        thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            b2.wait();
            let _ = peer.write_all(&[0x01]);
            let mut scratch = [0u8; 64];
            let _ = peer.set_read_timeout(Some(Duration::from_millis(200)));
            if let Ok(n) = peer.read(&mut scratch) {
                if n > 0 {
                    saw_begin2.store(true, Ordering::Relaxed);
                }
            }
            drop(peer);
            if let Ok((peer2, _)) = listener.accept() {
                serve_one_fcgi_ok(peer2);
            }
        });
        let pool = pool_at(addr);
        let mut stream = FcgiPoolMap::connect(&pool).unwrap();
        assert_eq!(FcgiPoolMap::probe(&mut stream), Probe::Clean);
        barrier.wait();
        thread::sleep(Duration::from_millis(30));
        let obs = ObsHub::new();
        let (mut stream, _) =
            prepare_stream_for_begin(stream, true, &pool, &obs).expect("fresh after dirty");
        assert!(
            !saw_begin.load(Ordering::Relaxed),
            "BEGIN_ON_DIRTY_SOCKET must be 0"
        );
        let params = vec![
            ("SCRIPT_FILENAME".into(), "/var/www/x.php".into()),
            ("REQUEST_METHOD".into(), "GET".into()),
        ];
        let attempt = exchange_with_total_budget(&mut stream, &pool, &params, &[]);
        assert!(attempt.result.is_ok());
    }

    #[test]
    fn dirty_put_never_requeues() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let _ = peer.write_all(&[0x99]);
            thread::sleep(Duration::from_millis(500));
        });
        let pool = pool_at(addr);
        let mut map = FcgiPoolMap::default();
        let stream = FcgiPoolMap::connect(&pool).unwrap();
        thread::sleep(Duration::from_millis(30));
        map.put(&pool, stream, 1);
        assert!(
            map.take(&pool, 1).is_none(),
            "DIRTY_CONNECTION_REUSE_COUNT must stay 0"
        );
    }

    #[test]
    fn total_timeout_ms_enforced_before_slow_read_budget() {
        // Silent peer after accept: never replies. total_ms=300 must beat read_ms=30_000.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let ready = std::sync::Arc::new(std::sync::Barrier::new(2));
        let ready2 = ready.clone();
        thread::spawn(move || {
            ready2.wait();
            let Ok((mut peer, _)) = listener.accept() else {
                return;
            };
            let _ = peer.set_read_timeout(Some(Duration::from_secs(60)));
            let mut sink = [0u8; 64];
            loop {
                match peer.read(&mut sink) {
                    Ok(0) => break,
                    Ok(_) => continue,
                    Err(e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        });
        ready.wait();
        thread::sleep(Duration::from_millis(20));
        let pool = CompiledFcgiPool {
            id: 1,
            transport: FcgiTransport::Tcp(addr),
            document_root: "/var/www".into(),
            script_suffix: String::new(),
            max_connections: 1,
            idle_timeout_ms: 60_000,
            connect_timeout_ms: 2_000,
            read_timeout_ms: 30_000,
            write_timeout_ms: 30_000,
            total_timeout_ms: 300,
            directory_index: Vec::new(),
            front_controller: None,
        };
        let mut stream = FcgiPoolMap::connect(&pool).unwrap();
        let params = vec![
            ("SCRIPT_FILENAME".into(), "/var/www/x.php".into()),
            ("REQUEST_METHOD".into(), "GET".into()),
        ];
        let started = Instant::now();
        let attempt = exchange_with_total_budget(&mut stream, &pool, &params, &[]);
        let elapsed = started.elapsed();
        assert!(
            matches!(attempt.result, Err(ExchangeError::Timeout)),
            "expected Timeout, got {:?}",
            attempt.result
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "total_timeout must not wait for full read_timeout; elapsed={elapsed:?}"
        );
    }
}
