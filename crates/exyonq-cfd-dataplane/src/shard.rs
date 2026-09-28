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

//! Shard-local mio event loop — Phase 4 H1 routing + WAF + bounded CL body relay.
//!
//! Each client connection owns at most one registered upstream connection.
//! Body bytes move through fixed directional buffers; read interest is removed
//! from the producing fd whenever the matching buffer is full.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use exyonq_cfd_control::compile_input_from_projection;
use exyonq_cfd_gen::{BackendTarget, GenDir, RouteTable};
use exyonq_waf::{FixedWafEngine, MapHeaders};
use exyonq_waf_api::{WafEngine, WafPhase, WafRequest};
use mio::event::Event;
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Registry, Token};
use socket2::{Domain, Protocol, Socket, Type};

use crate::bounds::{MAX_PENDING_REQUEST_BYTES, MAX_PENDING_RESPONSE_BYTES};
use crate::fcgi_exec::{self, FcgiExecError, MAX_FCGI_HTTP_BODY};
use crate::fcgi_pool::FcgiPoolMap;
use crate::hop::{build_upstream_request, filter_response_headers};
use crate::http_parse::{
    find_header_end, headers_have_illegal_line_endings, try_parse_request, Header, HttpVersion,
    ParseError, ParsedRequest, MAX_HEADER_BYTES, MAX_REQUEST_LINE,
};
use crate::static_serve::{self, StaticErr, StaticWrite};
use crate::upstream_pool::UpstreamPool;

const LISTENER: Token = Token(0);
const FOUNDATION_PATH: &str = "/__exyonq_cfd/v1/foundation";
/// Absolute monotonic budget to finish request headers once acquisition starts.
/// Does **not** refresh on drip bytes (slowloris / L4-P2-001). Stricter than the
/// product default `EXYONQ_READ_TIMEOUT_MS` (30s) used by core/epoll wire paths.
const CLIENT_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const BODY_PROGRESS_TIMEOUT: Duration = Duration::from_secs(30);
const UPSTREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ShardArgs {
    pub shard_id: usize,
    pub listen: SocketAddr,
    pub gen_dir: PathBuf,
    pub stop: Arc<AtomicBool>,
    pub live_gen: Arc<AtomicU64>,
    pub bound: Arc<AtomicUsize>,
    pub bind_failed: Arc<AtomicBool>,
    pub obs: crate::obs::ObsHubHandle,
}

struct Counters {
    requests: u64,
    errors: u64,
    bytes_out: u64,
    backpressure_client_paused: u64,
    backpressure_upstream_paused: u64,
}

enum Phase {
    ReadClientHeaders {
        since: Instant,
    },
    RelayRequestBody {
        remaining: u64,
        last_progress: Instant,
    },
    ReadUpstreamHeaders {
        since: Instant,
    },
    RelayResponseBody {
        remaining: u64,
        last_progress: Instant,
    },
    WriteClientResponse {
        close: bool,
        since: Instant,
    },
    StaticSend {
        file: File,
        offset: u64,
        remaining: u64,
        close: bool,
        since: Instant,
    },
    Idle {
        since: Instant,
    },
}

struct RelayBuffer {
    buf: Vec<u8>,
    pos: usize,
    cap: usize,
}

impl RelayBuffer {
    fn new(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
            pos: 0,
            cap,
        }
    }

    fn pending_len(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    fn is_empty(&self) -> bool {
        self.pending_len() == 0
    }

    fn available(&self) -> usize {
        self.cap.saturating_sub(self.pending_len())
    }

    fn pending(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    fn append(&mut self, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        self.compact();
        if self.pending_len() + data.len() > self.cap {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "relay buffer limit exceeded",
            ));
        }
        self.buf.extend_from_slice(data);
        Ok(())
    }

    fn consume(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.buf.len());
        if self.pos == self.buf.len() {
            self.clear();
        }
    }

    fn clear(&mut self) {
        self.buf.clear();
        self.pos = 0;
    }

    fn compact(&mut self) {
        if self.pos == 0 {
            return;
        }
        if self.pos == self.buf.len() {
            self.clear();
            return;
        }
        self.buf.drain(..self.pos);
        self.pos = 0;
    }
}

struct UpstreamSide {
    stream: TcpStream,
    token: Token,
    target: SocketAddr,
    registered: bool,
    reuse_ok: bool,
    /// ADR-021: READABLE held until first upstream request byte is written.
    await_first_write: bool,
}

struct Conn {
    client: TcpStream,
    client_token: Token,
    client_registered: bool,
    client_head: Vec<u8>,
    upstream_head: Vec<u8>,
    /// Staged upstream request line+headers (bounded by header limits; not the body relay).
    to_upstream_head: Vec<u8>,
    /// Staged client response status+headers (bounded by header limits; not the body relay).
    to_client_head: Vec<u8>,
    to_upstream: RelayBuffer,
    to_client: RelayBuffer,
    upstream: Option<UpstreamSide>,
    phase: Phase,
    client_close: bool,
    request_head: bool,
    response_started: bool,
    /// Monotonic start of current request (headers acquisition).
    req_started: Instant,
    req_bytes_in: u64,
    last_status: u16,
    obs_bytes_out_expected: u64,
    obs_inflight: bool,
    obs_accounted: bool,
    obs_method: &'static str,
}

struct EventCtx<'a> {
    registry: &'a Registry,
    args: &'a ShardArgs,
    table: &'a RouteTable,
    waf: Option<&'a FixedWafEngine>,
    gen_id: u64,
    pool: &'a mut UpstreamPool,
    fcgi_pools: &'a mut FcgiPoolMap,
    upstream_to_client: &'a mut HashMap<Token, Token>,
    next_token: &'a mut Token,
    counters: &'a mut Counters,
}

struct ObsNote<'a> {
    args: &'a ShardArgs,
    event: crate::obs::ObsEvent,
    method: &'static str,
    status: u16,
    bytes_in: u64,
    bytes_out: u64,
    latency_us: u64,
    had_body: bool,
    error_class: &'static str,
}

fn obs_finalize(conn: &mut Conn, n: ObsNote<'_>) {
    if conn.obs_accounted {
        return;
    }
    conn.obs_accounted = true;
    conn.obs_inflight = false;
    conn.last_status = n.status;
    n.args
        .obs
        .note_request_complete(n.status, n.bytes_in, n.bytes_out, n.latency_us, n.had_body);
    match n.event {
        crate::obs::ObsEvent::WafDeny => {
            n.args.obs.waf_denies.fetch_add(1, Ordering::Relaxed);
        }
        crate::obs::ObsEvent::WafAllow => {
            n.args.obs.waf_allows.fetch_add(1, Ordering::Relaxed);
        }
        crate::obs::ObsEvent::RouteMiss => {
            n.args.obs.route_misses.fetch_add(1, Ordering::Relaxed);
        }
        crate::obs::ObsEvent::Access => {
            n.args.obs.route_hits.fetch_add(1, Ordering::Relaxed);
        }
        crate::obs::ObsEvent::FramingReject => {
            n.args.obs.framing_rejects.fetch_add(1, Ordering::Relaxed);
        }
        crate::obs::ObsEvent::Timeout => {
            n.args.obs.timeouts.fetch_add(1, Ordering::Relaxed);
        }
        crate::obs::ObsEvent::Backpressure => {
            n.args
                .obs
                .backpressure_activations
                .fetch_add(1, Ordering::Relaxed);
        }
        crate::obs::ObsEvent::InternalError | crate::obs::ObsEvent::UpstreamError => {
            n.args.obs.internal_errors.fetch_add(1, Ordering::Relaxed);
        }
        crate::obs::ObsEvent::ClientDisconnect => {
            n.args
                .obs
                .client_disconnects
                .fetch_add(1, Ordering::Relaxed);
        }
        crate::obs::ObsEvent::UpstreamDisconnect => {
            n.args
                .obs
                .upstream_disconnects
                .fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
    n.args.obs.try_push_event(crate::obs::ObsRecord {
        ts_unix_ms: crate::obs::ObsRecord::now_ms(),
        event: n.event,
        method: n.method,
        route_id: 0,
        status: n.status,
        status_class: crate::obs::StatusClass::from_status(n.status),
        bytes_in: n.bytes_in,
        bytes_out: n.bytes_out,
        latency_us: n.latency_us,
        generation_id: n.args.live_gen.load(Ordering::Relaxed),
        shard_id: n.args.shard_id as u32,
        error_class: n.error_class,
    });
}

fn obs_error(
    conn: &mut Conn,
    args: &ShardArgs,
    event: crate::obs::ObsEvent,
    status: u16,
    error_class: &'static str,
) {
    let latency_us = conn.req_started.elapsed().as_micros() as u64;
    let bytes_out =
        u64::try_from(conn.to_client_head.len().saturating_add(conn.to_client.pos)).unwrap_or(0);
    obs_finalize(
        conn,
        ObsNote {
            args,
            event,
            method: conn.obs_method,
            status,
            bytes_in: conn.req_bytes_in,
            bytes_out,
            latency_us,
            had_body: conn.req_bytes_in > 0,
            error_class,
        },
    );
}

pub fn run_shard(args: ShardArgs) -> io::Result<()> {
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(128);
    let listener = match bind_reuseport(args.listen) {
        Ok(l) => l,
        Err(e) => {
            args.bind_failed.store(true, Ordering::SeqCst);
            return Err(e);
        }
    };
    let mut listener = TcpListener::from_std(listener);
    if let Err(e) = poll
        .registry()
        .register(&mut listener, LISTENER, Interest::READABLE)
    {
        args.bind_failed.store(true, Ordering::SeqCst);
        return Err(e);
    }
    args.bound.fetch_add(1, Ordering::SeqCst);

    let mut conns: HashMap<Token, Conn> = HashMap::new();
    let mut upstream_to_client: HashMap<Token, Token> = HashMap::new();
    let mut next_token = Token(1);
    let mut pool = UpstreamPool::default();
    let mut fcgi_pools = FcgiPoolMap::default();
    let mut counters = Counters {
        requests: 0,
        errors: 0,
        bytes_out: 0,
        backpressure_client_paused: 0,
        backpressure_upstream_paused: 0,
    };
    let mut local_gen_id = 0u64;
    let mut table = RouteTable::default();
    let mut waf: Option<FixedWafEngine> = None;
    refresh_projection(&args, &mut local_gen_id, &mut table, &mut waf);

    eprintln!(
        "cfd-shard-{} listening {} (Phase-4 H1+routing+WAF+CL-body-streaming; SO_REUSEPORT among DP shards)",
        args.shard_id, args.listen
    );

    while !args.stop.load(Ordering::Relaxed) {
        let prev_gen = local_gen_id;
        refresh_projection(&args, &mut local_gen_id, &mut table, &mut waf);
        if local_gen_id != prev_gen {
            pool.flush_all(poll.registry());
            fcgi_pools.flush_generation(local_gen_id);
        }
        poll.poll(&mut events, Some(Duration::from_millis(50)))?;
        let now = Instant::now();

        let timed_out: Vec<Token> = conns
            .iter()
            .filter_map(|(tok, c)| {
                if conn_deadline(c, now) {
                    Some(*tok)
                } else {
                    None
                }
            })
            .collect();
        for tok in timed_out {
            if let Some(mut c) = conns.remove(&tok) {
                // Never inject a second HTTP message after response bytes have started.
                if !c.response_started {
                    let _ = write_simple(&mut c.client, 408, "timeout\n");
                }
                if c.obs_inflight && !c.obs_accounted {
                    obs_error(&mut c, &args, crate::obs::ObsEvent::Timeout, 408, "timeout");
                }
                cleanup_conn(poll.registry(), &mut c, &mut upstream_to_client, &args.obs);
                counters.errors += 1;
            }
        }

        for event in events.iter() {
            match event.token() {
                LISTENER => accept_ready(
                    poll.registry(),
                    &mut listener,
                    &mut conns,
                    &mut next_token,
                    &args.obs,
                )?,
                token => {
                    if pool.is_idle_token(token) {
                        pool.on_idle_ready(poll.registry(), token);
                        continue;
                    }
                    let client_token = upstream_to_client.get(&token).copied().unwrap_or(token);
                    let action = if let Some(conn) = conns.get_mut(&client_token) {
                        let mut ctx = EventCtx {
                            registry: poll.registry(),
                            args: &args,
                            table: &table,
                            waf: waf.as_ref(),
                            gen_id: local_gen_id,
                            pool: &mut pool,
                            fcgi_pools: &mut fcgi_pools,
                            upstream_to_client: &mut upstream_to_client,
                            next_token: &mut next_token,
                            counters: &mut counters,
                        };
                        if token == client_token {
                            handle_client_event(conn, event, &mut ctx)
                        } else {
                            handle_upstream_event(conn, event, &mut ctx)
                        }
                    } else {
                        Ok(Action::Keep)
                    };
                    let close = match action {
                        Ok(Action::Keep) => false,
                        Ok(Action::Close) => true,
                        Err(_) => {
                            counters.errors += 1;
                            true
                        }
                    };
                    if close {
                        if let Some(mut conn) = conns.remove(&client_token) {
                            if conn.obs_inflight && !conn.obs_accounted {
                                let status = conn.last_status;
                                obs_error(
                                    &mut conn,
                                    &args,
                                    crate::obs::ObsEvent::ClientDisconnect,
                                    status,
                                    "client_disconnect",
                                );
                            }
                            cleanup_conn(
                                poll.registry(),
                                &mut conn,
                                &mut upstream_to_client,
                                &args.obs,
                            );
                        }
                    } else if let Some(conn) = conns.get_mut(&client_token) {
                        update_interests(poll.registry(), conn, &mut counters, &args.obs)?;
                    }
                }
            }
        }
    }
    let _ = counters;
    Ok(())
}

enum Action {
    Keep,
    Close,
}

fn conn_deadline(conn: &Conn, now: Instant) -> bool {
    let deadline = match conn.phase {
        Phase::ReadClientHeaders { since } => since + CLIENT_HEADER_TIMEOUT,
        Phase::RelayRequestBody { last_progress, .. } => last_progress + BODY_PROGRESS_TIMEOUT,
        Phase::ReadUpstreamHeaders { since } => since + UPSTREAM_HEADER_TIMEOUT,
        Phase::RelayResponseBody { last_progress, .. } => last_progress + BODY_PROGRESS_TIMEOUT,
        Phase::WriteClientResponse { since, .. } => since + BODY_PROGRESS_TIMEOUT,
        Phase::StaticSend { since, .. } => since + BODY_PROGRESS_TIMEOUT,
        Phase::Idle { since } => since + CLIENT_IDLE_TIMEOUT,
    };
    now >= deadline
}

fn accept_ready(
    registry: &Registry,
    listener: &mut TcpListener,
    conns: &mut HashMap<Token, Conn>,
    next_token: &mut Token,
    obs: &crate::obs::ObsHubHandle,
) -> io::Result<()> {
    loop {
        match listener.accept() {
            Ok((mut stream, _peer)) => {
                let token = alloc_token(next_token);
                registry.register(&mut stream, token, Interest::READABLE)?;
                conns.insert(
                    token,
                    Conn {
                        client: stream,
                        client_token: token,
                        client_registered: true,
                        client_head: Vec::with_capacity(1024),
                        upstream_head: Vec::with_capacity(1024),
                        to_upstream_head: Vec::new(),
                        to_client_head: Vec::new(),
                        to_upstream: RelayBuffer::new(MAX_PENDING_REQUEST_BYTES),
                        to_client: RelayBuffer::new(MAX_PENDING_RESPONSE_BYTES),
                        upstream: None,
                        phase: Phase::ReadClientHeaders {
                            since: Instant::now(),
                        },
                        client_close: false,
                        request_head: false,
                        response_started: false,
                        req_started: Instant::now(),
                        req_bytes_in: 0,
                        last_status: 0,
                        obs_bytes_out_expected: 0,
                        obs_inflight: false,
                        obs_accounted: false,
                        obs_method: "UNKNOWN",
                    },
                );
                obs.accepted_connections.fetch_add(1, Ordering::Relaxed);
                obs.active_connections.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

fn alloc_token(next_token: &mut Token) -> Token {
    let token = *next_token;
    next_token.0 = next_token.0.wrapping_add(1).max(1);
    token
}

fn refresh_projection(
    args: &ShardArgs,
    local_gen_id: &mut u64,
    table: &mut RouteTable,
    waf: &mut Option<FixedWafEngine>,
) {
    let id = args.live_gen.load(Ordering::Acquire);
    if id == *local_gen_id {
        return;
    }
    let gd = GenDir::new(&args.gen_dir);
    match gd.load() {
        Ok(g) if g.generation_id == id => match g.composite() {
            Ok(Some(c)) => {
                let new_waf = match &c.waf {
                    None => None,
                    Some(proj) => match compile_input_from_projection(proj) {
                        Ok(input) => match FixedWafEngine::compile(input) {
                            Ok(eng) => Some(eng),
                            Err(e) => {
                                eprintln!(
                                    "cfd-shard-{} waf compile failed gen={id}: {e} (keeping gen={})",
                                    args.shard_id, *local_gen_id
                                );
                                return;
                            }
                        },
                        Err(e) => {
                            eprintln!(
                                "cfd-shard-{} waf projection invalid gen={id}: {e} (keeping gen={})",
                                args.shard_id, *local_gen_id
                            );
                            return;
                        }
                    },
                };
                *table = c.routes;
                *waf = new_waf;
                *local_gen_id = id;
            }
            Ok(None) => {
                *table = RouteTable::default();
                *waf = None;
                *local_gen_id = id;
            }
            Err(e) => {
                eprintln!(
                    "cfd-shard-{} corrupt projection gen={id} rejected: {e} (keeping gen={})",
                    args.shard_id, *local_gen_id
                );
            }
        },
        Ok(_) => {}
        Err(e) => eprintln!("cfd-shard-{} gen load: {e}", args.shard_id),
    }
}

fn handle_client_event(
    conn: &mut Conn,
    event: &Event,
    ctx: &mut EventCtx<'_>,
) -> io::Result<Action> {
    if event.is_error() || event.is_write_closed() {
        return Ok(Action::Close);
    }
    if event.is_writable() {
        write_pending_head_then_body(
            &mut conn.client,
            &mut conn.to_client_head,
            &mut conn.to_client,
        )?;
        if conn.to_client_head.is_empty() && conn.to_client.is_empty() {
            match conn.phase {
                Phase::WriteClientResponse { close, .. } => {
                    return Ok(if close {
                        Action::Close
                    } else {
                        reset_for_keepalive(conn);
                        Action::Keep
                    });
                }
                Phase::StaticSend { .. } => {
                    if conn.to_client_head.is_empty() {
                        return pump_static_send(conn, ctx.args);
                    }
                }
                Phase::RelayResponseBody { remaining: 0, .. } => {
                    return finish_response(
                        ctx.registry,
                        conn,
                        ctx.pool,
                        ctx.upstream_to_client,
                        ctx.next_token,
                        ctx.gen_id,
                        ctx.counters,
                        ctx.args,
                    );
                }
                _ => {}
            }
        }
    }
    if event.is_readable() {
        match conn.phase {
            Phase::Idle { .. } | Phase::ReadClientHeaders { .. } => {
                read_client_headers(conn, ctx)?;
            }
            Phase::RelayRequestBody { .. } => {
                read_request_body(conn)?;
            }
            _ => {}
        }
    }
    Ok(Action::Keep)
}

fn handle_upstream_event(
    conn: &mut Conn,
    event: &Event,
    ctx: &mut EventCtx<'_>,
) -> io::Result<Action> {
    if event.is_error() || event.is_write_closed() {
        if conn.response_started {
            return Ok(Action::Close);
        }
        queue_error(conn, 502, "upstream io error\n")?;
        obs_error(
            conn,
            ctx.args,
            crate::obs::ObsEvent::UpstreamError,
            502,
            "upstream_io",
        );
        discard_upstream(ctx.registry, conn, ctx.upstream_to_client, &ctx.args.obs);
        return Ok(Action::Keep);
    }
    // ADR-021: probe BEFORE any write on this event (closes READABLE|WRITABLE TOCTOU).
    if let Some(action) = prewrite_poison_action(conn, ctx)? {
        return Ok(action);
    }
    if event.is_writable() {
        // Re-probe immediately before write (closes post-probe / pre-syscall inject).
        if let Some(action) = prewrite_poison_action(conn, ctx)? {
            return Ok(action);
        }
        let write_result = match conn.upstream.as_mut() {
            Some(up) => write_pending_head_then_body(
                &mut up.stream,
                &mut conn.to_upstream_head,
                &mut conn.to_upstream,
            ),
            None => Ok(()),
        };
        if let Err(e) = write_result {
            if conn.response_started {
                return Err(e);
            }
            queue_error(conn, 502, "upstream write failed\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::UpstreamError,
                502,
                "upstream_write",
            );
            discard_upstream(ctx.registry, conn, ctx.upstream_to_client, &ctx.args.obs);
            return Ok(Action::Keep);
        }
        if conn.upstream.is_some() {
            if conn.to_upstream_head.is_empty() && conn.to_upstream.is_empty() {
                if let Phase::RelayRequestBody { remaining: 0, .. } = conn.phase {
                    if let Some(up) = conn.upstream.as_mut() {
                        up.await_first_write = false;
                    }
                    conn.phase = Phase::ReadUpstreamHeaders {
                        since: Instant::now(),
                    };
                }
            } else {
                mark_body_progress(conn);
            }
        }
    }
    if event.is_readable() {
        if let Some(action) = prewrite_poison_action(conn, ctx)? {
            return Ok(action);
        }
        match conn.phase {
            Phase::ReadUpstreamHeaders { .. } => {
                if let Err(e) = read_upstream_headers(conn) {
                    if conn.response_started {
                        return Err(e);
                    }
                    queue_error(conn, 502, "upstream framing error\n")?;
                    obs_error(
                        conn,
                        ctx.args,
                        crate::obs::ObsEvent::UpstreamError,
                        502,
                        "upstream_framing",
                    );
                    discard_upstream(ctx.registry, conn, ctx.upstream_to_client, &ctx.args.obs);
                    return Ok(Action::Keep);
                }
            }
            Phase::RelayResponseBody { .. } => {
                read_response_body(conn)?;
            }
            _ => {}
        }
    }
    if matches!(conn.phase, Phase::RelayResponseBody { remaining: 0, .. })
        && conn.to_client_head.is_empty()
        && conn.to_client.is_empty()
    {
        return finish_response(
            ctx.registry,
            conn,
            ctx.pool,
            ctx.upstream_to_client,
            ctx.next_token,
            ctx.gen_id,
            ctx.counters,
            ctx.args,
        );
    }
    Ok(Action::Keep)
}

/// If upstream still awaits first request write and has unread bytes/EOF, discard.
fn prewrite_poison_action(conn: &mut Conn, ctx: &mut EventCtx<'_>) -> io::Result<Option<Action>> {
    let Some(up) = conn.upstream.as_mut() else {
        return Ok(None);
    };
    if !up.await_first_write {
        return Ok(None);
    }
    if UpstreamPool::probe(&mut up.stream) != crate::upstream_pool::Probe::Dirty {
        return Ok(None);
    }
    if conn.response_started {
        return Ok(Some(Action::Close));
    }
    queue_error(conn, 502, "upstream poisoned before request write\n")?;
    obs_error(
        conn,
        ctx.args,
        crate::obs::ObsEvent::UpstreamError,
        502,
        "upstream_prewrite_poison",
    );
    discard_upstream(ctx.registry, conn, ctx.upstream_to_client, &ctx.args.obs);
    Ok(Some(Action::Keep))
}

fn read_client_headers(conn: &mut Conn, ctx: &mut EventCtx<'_>) -> io::Result<()> {
    // Preserve absolute `since` while headers are incomplete. Resetting on every
    // readable drip would let slowloris hold the conn indefinitely (L4-P2-001).
    if !matches!(conn.phase, Phase::ReadClientHeaders { .. }) {
        conn.phase = Phase::ReadClientHeaders {
            since: Instant::now(),
        };
    }
    let mut tmp = [0u8; 4096];
    loop {
        match conn.client.read(&mut tmp) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "client closed before request headers",
                ));
            }
            Ok(n) => {
                conn.client_head.extend_from_slice(&tmp[..n]);
                if conn.client_head.len() > MAX_HEADER_BYTES + MAX_REQUEST_LINE + 4 {
                    queue_error(conn, 431, "header limits\n")?;
                    obs_error(
                        conn,
                        ctx.args,
                        crate::obs::ObsEvent::FramingReject,
                        431,
                        "header_limit",
                    );
                    return Ok(());
                }
                match try_parse_request(&conn.client_head) {
                    Ok(parsed) => {
                        process_parsed_request(conn, parsed, ctx)?;
                        return Ok(());
                    }
                    Err(ParseError::Incomplete) => continue,
                    Err(e) => {
                        let (code, msg) = map_parse_error(&e);
                        queue_error(conn, code, msg)?;
                        match &e {
                            ParseError::BodyNotSupported | ParseError::AmbiguousFraming => {
                                ctx.args.obs.cl_te_rejects.fetch_add(1, Ordering::Relaxed);
                            }
                            _ => {}
                        }
                        conn.obs_inflight = true;
                        conn.obs_method = "UNKNOWN";
                        obs_error(
                            conn,
                            ctx.args,
                            crate::obs::ObsEvent::FramingReject,
                            code,
                            "parse_error",
                        );
                        ctx.counters.errors += 1;
                        return Ok(());
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

fn process_parsed_request(
    conn: &mut Conn,
    parsed: ParsedRequest,
    ctx: &mut EventCtx<'_>,
) -> io::Result<()> {
    let body_start_len = conn.client_head.len().saturating_sub(parsed.header_end);
    if u64::try_from(body_start_len).map_or(true, |n| n > parsed.content_length) {
        queue_error(conn, 400, "pipelining unsupported\n")?;
        obs_error(
            conn,
            ctx.args,
            crate::obs::ObsEvent::FramingReject,
            400,
            "pipelining",
        );
        return Ok(());
    }
    let body_start = conn.client_head.split_off(parsed.header_end);
    conn.client_head.clear();

    ctx.counters.requests += 1;
    conn.obs_inflight = true;
    conn.obs_accounted = false;
    conn.obs_method = method_static(&parsed.method);
    conn.req_bytes_in = parsed.content_length;
    conn.req_started = Instant::now();
    conn.client_close = parsed.connection_close;
    conn.request_head = parsed.method.eq_ignore_ascii_case("HEAD");

    if is_foundation(&parsed) {
        // Force close when a body was declared: foundation never drains remaining
        // body bytes. Keepalive would leave unread framing for the next request
        // (desync / smuggling-adjacent). Matches WAF-deny and route-miss policy.
        let close = parsed.connection_close || parsed.content_length > 0;
        let body = format!(
            "ok generation_id={} shard={}\n",
            ctx.gen_id, ctx.args.shard_id
        );
        let resp = client_response(
            200,
            "OK",
            body.as_bytes(),
            &[
                ("X-Exyonq-Cfd-Generation", &ctx.gen_id.to_string()),
                ("Content-Type", "text/plain"),
            ],
            close,
        );
        queue_response(conn, &resp, close)?;
        ctx.counters.bytes_out += u64::try_from(resp.len()).unwrap_or(u64::MAX);
        let latency_us = conn.req_started.elapsed().as_micros() as u64;
        let bytes_out = u64::try_from(resp.len()).unwrap_or(u64::MAX);
        obs_finalize(
            conn,
            ObsNote {
                args: ctx.args,
                event: crate::obs::ObsEvent::Access,
                method: method_static(&parsed.method),
                status: 200,
                bytes_in: parsed.content_length,
                bytes_out,
                latency_us,
                had_body: parsed.content_length > 0,
                error_class: "none",
            },
        );
        return Ok(());
    }

    if let Some(engine) = ctx.waf {
        let client_ip = conn
            .client
            .peer_addr()
            .map(|a| a.ip())
            .unwrap_or(IpAddr::from([0, 0, 0, 0]));
        let (path_only, query) = split_path_query(&parsed.path);
        let headers_view = map_headers_from_parsed(&parsed.headers);
        let req = WafRequest {
            method: &parsed.method,
            host: parsed.host.as_deref(),
            path: path_only,
            query,
            headers: &headers_view,
            client_ip,
            route_id: None,
        };
        let decision = engine.inspect(WafPhase::RequestHeaders, &req);
        if decision.is_blocking() {
            let rule = decision
                .violations
                .first()
                .map(|v| v.rule_id.as_str())
                .unwrap_or("unknown");
            let body = b"waf denied\n";
            let resp = client_response(
                403,
                "Forbidden",
                body,
                &[
                    ("X-Exyonq-Cfd-Generation", &ctx.gen_id.to_string()),
                    ("X-Exyonq-Cfd-Waf-Rule", rule),
                    ("Content-Type", "text/plain"),
                ],
                parsed.connection_close || parsed.content_length > 0,
            );
            queue_response(
                conn,
                &resp,
                parsed.connection_close || parsed.content_length > 0,
            )?;
            ctx.counters.errors += 1;
            let latency_us = conn.req_started.elapsed().as_micros() as u64;
            let bytes_out = u64::try_from(resp.len()).unwrap_or(u64::MAX);
            obs_finalize(
                conn,
                ObsNote {
                    args: ctx.args,
                    event: crate::obs::ObsEvent::WafDeny,
                    method: method_static(&parsed.method),
                    status: 403,
                    bytes_in: parsed.content_length,
                    bytes_out,
                    latency_us,
                    had_body: parsed.content_length > 0,
                    error_class: "waf_deny",
                },
            );
            return Ok(());
        }
    }

    let host = parsed.host.as_deref();
    let route_path = path_without_query(&parsed.path);
    let Some((route, target)) = ctx.table.lookup(route_path, host) else {
        let body = b"route miss\n";
        let resp = client_response(
            404,
            "Not Found",
            body,
            &[
                ("X-Exyonq-Cfd-Generation", &ctx.gen_id.to_string()),
                ("Content-Type", "text/plain"),
            ],
            parsed.connection_close || parsed.content_length > 0,
        );
        queue_response(
            conn,
            &resp,
            parsed.connection_close || parsed.content_length > 0,
        )?;
        let latency_us = conn.req_started.elapsed().as_micros() as u64;
        let bytes_out = u64::try_from(resp.len()).unwrap_or(u64::MAX);
        obs_finalize(
            conn,
            ObsNote {
                args: ctx.args,
                event: crate::obs::ObsEvent::RouteMiss,
                method: method_static(&parsed.method),
                status: 404,
                bytes_in: parsed.content_length,
                bytes_out,
                latency_us,
                had_body: parsed.content_length > 0,
                error_class: "route_miss",
            },
        );
        return Ok(());
    };
    match target {
        BackendTarget::Reject => {
            queue_error(conn, 501, "backend rejected\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::UpstreamError,
                501,
                "backend_reject",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
        BackendTarget::Fastcgi(fcgi_pool) => {
            dispatch_fcgi(conn, ctx, fcgi_pool, &parsed, body_start)
        }
        BackendTarget::Static(static_policy) => {
            dispatch_static(conn, ctx, route, static_policy, &parsed)
        }
        BackendTarget::Proxy(upstream) => {
            let upstream_req = build_upstream_request(
                &parsed.method,
                request_target_for_upstream(&parsed.path),
                &parsed.headers,
                &upstream.authority_host,
                parsed.content_length,
            );
            // Request head is staged separately from the 16 KiB body relay buffer.
            conn.to_upstream_head = upstream_req;
            if !body_start.is_empty() {
                conn.to_upstream.append(&body_start)?;
            }
            let sent_body = u64::try_from(body_start.len()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "prefetched body length overflow",
                )
            })?;
            let remaining = parsed.content_length.saturating_sub(sent_body);
            if attach_upstream(
                ctx.registry,
                conn,
                upstream.connect,
                ctx.pool,
                ctx.upstream_to_client,
                ctx.next_token,
                ctx.gen_id,
                &ctx.args.obs,
            )
            .is_err()
            {
                conn.to_upstream_head.clear();
                conn.to_upstream.clear();
                queue_error(conn, 502, "upstream connect failed\n")?;
                obs_error(
                    conn,
                    ctx.args,
                    crate::obs::ObsEvent::UpstreamError,
                    502,
                    "upstream_connect",
                );
                ctx.counters.errors += 1;
                return Ok(());
            }
            conn.phase = Phase::RelayRequestBody {
                remaining,
                last_progress: Instant::now(),
            };
            Ok(())
        }
    }
}

fn dispatch_fcgi(
    conn: &mut Conn,
    ctx: &mut EventCtx<'_>,
    fcgi_pool: &exyonq_cfd_gen::CompiledFcgiPool,
    parsed: &ParsedRequest,
    body_start: Vec<u8>,
) -> io::Result<()> {
    if parsed.content_length > MAX_FCGI_HTTP_BODY {
        queue_error(conn, 413, "fcgi body too large\n")?;
        obs_error(
            conn,
            ctx.args,
            crate::obs::ObsEvent::FramingReject,
            413,
            "fcgi_body_too_large",
        );
        ctx.counters.errors += 1;
        return Ok(());
    }

    // Collect remaining body synchronously (MVP: small bounded CL only).
    let mut stdin = body_start;
    let need = usize::try_from(parsed.content_length).unwrap_or(usize::MAX);
    if stdin.len() > need {
        stdin.truncate(need);
    }
    while stdin.len() < need {
        let mut tmp = [0u8; 4096];
        let want = (need - stdin.len()).min(tmp.len());
        match conn.client.read(&mut tmp[..want]) {
            Ok(0) => {
                queue_error(conn, 400, "incomplete body\n")?;
                obs_error(
                    conn,
                    ctx.args,
                    crate::obs::ObsEvent::ClientDisconnect,
                    400,
                    "incomplete_body",
                );
                ctx.counters.errors += 1;
                return Ok(());
            }
            Ok(n) => stdin.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Brief spin for mio non-blocking client — MVP deadline.
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => {
                queue_error(conn, 400, "body read failed\n")?;
                ctx.counters.errors += 1;
                return Ok(());
            }
        }
        if conn.req_started.elapsed() > BODY_PROGRESS_TIMEOUT {
            queue_error(conn, 408, "body timeout\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::Timeout,
                408,
                "fcgi_body_timeout",
            );
            ctx.counters.errors += 1;
            return Ok(());
        }
    }

    let (path_only, query) = split_query(&parsed.path);
    let http_headers: Vec<(String, String)> = parsed
        .headers
        .iter()
        .map(|h| (h.name.clone(), h.value.clone()))
        .collect();
    let server_name = parsed.host.as_deref().unwrap_or("localhost");
    let server_port = ctx.args.listen.port();
    let declared_cl = if parsed.content_length > 0 || !stdin.is_empty() {
        Some(stdin.len())
    } else {
        None
    };

    let script_uri = match crate::fcgi_route::resolve_fcgi_script(fcgi_pool, path_only) {
        crate::fcgi_route::FcgiRouteResolve::Script { script_uri } => script_uri,
        crate::fcgi_route::FcgiRouteResolve::NotFound => {
            queue_error(conn, 404, "not found\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::RouteMiss,
                404,
                "fcgi_script_not_found",
            );
            ctx.counters.errors += 1;
            return Ok(());
        }
        crate::fcgi_route::FcgiRouteResolve::Misconfigured => {
            queue_error(conn, 502, "fcgi routing misconfigured\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::UpstreamError,
                502,
                "fcgi_routing_misconfigured",
            );
            ctx.counters.errors += 1;
            return Ok(());
        }
        crate::fcgi_route::FcgiRouteResolve::Forbidden => {
            queue_error(conn, 400, "fcgi request invalid\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::FramingReject,
                400,
                "fcgi_script_forbidden",
            );
            ctx.counters.errors += 1;
            return Ok(());
        }
    };

    let result = fcgi_exec::execute_get(
        ctx.fcgi_pools,
        fcgi_pool,
        ctx.gen_id,
        &parsed.method,
        &parsed.path,
        &script_uri,
        query,
        server_name,
        server_port,
        &http_headers,
        &stdin,
        declared_cl,
        &ctx.args.obs,
    );

    match result {
        Ok(ok) => {
            let reason = reason_phrase(ok.status);
            let close = parsed.connection_close || parsed.content_length > 0;
            let mut headers = ok.headers;
            headers.push(("x-exyonq-cfd-generation".into(), ctx.gen_id.to_string()));
            // Drop CGI Content-Length; format_client_response recomputes from wire body.
            headers.retain(|(n, _)| !n.eq_ignore_ascii_case("content-length"));
            let bodyless = fcgi_exec::response_must_be_bodyless(conn.request_head, ok.status);
            let resp = fcgi_exec::format_client_response(
                ok.status, reason, &headers, &ok.body, close, bodyless,
            );
            let bytes_out = u64::try_from(resp.len()).unwrap_or(u64::MAX);
            queue_response(conn, &resp, close)?;
            let latency_us = conn.req_started.elapsed().as_micros() as u64;
            obs_finalize(
                conn,
                ObsNote {
                    args: ctx.args,
                    event: crate::obs::ObsEvent::Access,
                    method: method_static(&parsed.method),
                    status: ok.status,
                    bytes_in: parsed.content_length,
                    bytes_out,
                    latency_us,
                    had_body: parsed.content_length > 0,
                    error_class: "ok",
                },
            );
            Ok(())
        }
        Err(FcgiExecError::MethodNotAllowed) => {
            queue_error(conn, 501, "fcgi method not allowed\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::UpstreamError,
                501,
                "fcgi_method",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
        Err(FcgiExecError::Connect) => {
            queue_error(conn, 502, "fcgi connect failed\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::UpstreamError,
                502,
                "fcgi_connect",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
        Err(FcgiExecError::Timeout) => {
            queue_error(conn, 504, "fcgi timeout\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::Timeout,
                504,
                "fcgi_timeout",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
        Err(FcgiExecError::Params(_))
        | Err(FcgiExecError::BodyTooLarge)
        | Err(FcgiExecError::ScriptPath) => {
            queue_error(conn, 400, "fcgi request invalid\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::FramingReject,
                400,
                "fcgi_request",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
        Err(FcgiExecError::Protocol(_)) | Err(FcgiExecError::CgiParse) => {
            queue_error(conn, 502, "fcgi protocol failed\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::UpstreamError,
                502,
                "fcgi_protocol",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
    }
}

fn dispatch_static(
    conn: &mut Conn,
    ctx: &mut EventCtx<'_>,
    route: &exyonq_cfd_gen::CompiledRoute,
    static_policy: &exyonq_cfd_gen::CompiledStaticPolicy,
    parsed: &ParsedRequest,
) -> io::Result<()> {
    if let Err(StaticErr::MethodNotAllowed) = static_serve::method_allowed(&parsed.method) {
        queue_error(conn, 405, "static method not allowed\n")?;
        obs_error(
            conn,
            ctx.args,
            crate::obs::ObsEvent::FramingReject,
            405,
            "static_method",
        );
        ctx.counters.errors += 1;
        return Ok(());
    }
    match static_serve::open_static(static_policy, &route.path, path_without_query(&parsed.path)) {
        Ok(opened) => {
            // Static handlers do not consume request bodies. If one is declared,
            // close after the response so unread bytes cannot poison keepalive.
            let close = parsed.connection_close || parsed.content_length > 0;
            let head = static_response_head(200, opened.len, opened.content_type, close);
            let head_len = head.len();
            conn.to_client_head = head;
            conn.to_client.clear();
            conn.last_status = 200;
            let head_only = parsed.method.eq_ignore_ascii_case("HEAD") || opened.len == 0;
            conn.obs_bytes_out_expected = if head_only {
                u64::try_from(head_len).unwrap_or(u64::MAX)
            } else {
                u64::try_from(head_len)
                    .unwrap_or(u64::MAX)
                    .saturating_add(opened.len)
            };
            conn.response_started = true;
            if head_only {
                queue_static_head_only(conn, close);
                let latency_us = conn.req_started.elapsed().as_micros() as u64;
                obs_finalize(
                    conn,
                    ObsNote {
                        args: ctx.args,
                        event: crate::obs::ObsEvent::Access,
                        method: method_static(&parsed.method),
                        status: 200,
                        bytes_in: parsed.content_length,
                        bytes_out: conn.obs_bytes_out_expected,
                        latency_us,
                        had_body: parsed.content_length > 0,
                        error_class: "static_ok",
                    },
                );
            } else {
                conn.phase = Phase::StaticSend {
                    file: opened.file,
                    offset: 0,
                    remaining: opened.len,
                    close,
                    since: Instant::now(),
                };
            }
            Ok(())
        }
        Err(StaticErr::NotFound) => {
            queue_error(conn, 404, "static not found\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::RouteMiss,
                404,
                "static_not_found",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
        Err(StaticErr::Forbidden) => {
            queue_error(conn, 403, "static forbidden\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::FramingReject,
                403,
                "static_forbidden",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
        Err(StaticErr::MethodNotAllowed) => {
            queue_error(conn, 405, "static method not allowed\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::FramingReject,
                405,
                "static_method",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
        Err(StaticErr::Io(_)) => {
            queue_error(conn, 500, "static io error\n")?;
            obs_error(
                conn,
                ctx.args,
                crate::obs::ObsEvent::InternalError,
                500,
                "static_io",
            );
            ctx.counters.errors += 1;
            Ok(())
        }
    }
}

fn queue_static_head_only(conn: &mut Conn, close: bool) {
    conn.phase = Phase::WriteClientResponse {
        close,
        since: Instant::now(),
    };
}

fn pump_static_send(conn: &mut Conn, args: &ShardArgs) -> io::Result<Action> {
    let (state, close_conn) = {
        let Phase::StaticSend {
            file,
            offset,
            remaining,
            close,
            since,
        } = &mut conn.phase
        else {
            return Ok(Action::Keep);
        };
        let state = send_static_body_to_client(&mut conn.client, file, offset, remaining)?;
        *since = Instant::now();
        (state, *close)
    };
    match state {
        StaticWrite::Pending => Ok(Action::Keep),
        StaticWrite::Complete => {
            if conn.obs_inflight && !conn.obs_accounted {
                let latency_us = conn.req_started.elapsed().as_micros() as u64;
                obs_finalize(
                    conn,
                    ObsNote {
                        args,
                        event: crate::obs::ObsEvent::Access,
                        method: conn.obs_method,
                        status: 200,
                        bytes_in: conn.req_bytes_in,
                        bytes_out: conn.obs_bytes_out_expected,
                        latency_us,
                        had_body: conn.req_bytes_in > 0,
                        error_class: "static_ok",
                    },
                );
            }
            Ok(if close_conn {
                Action::Close
            } else {
                reset_for_keepalive(conn);
                Action::Keep
            })
        }
    }
}

fn send_static_body_to_client(
    client: &mut TcpStream,
    file: &File,
    offset: &mut u64,
    remaining: &mut u64,
) -> io::Result<StaticWrite> {
    #[cfg(target_os = "linux")]
    {
        static_serve::send_static_body_sendfile(client, file, offset, remaining)
    }
    #[cfg(not(target_os = "linux"))]
    {
        static_serve::send_static_body(client, file, offset, remaining)
    }
}

fn split_query(path: &str) -> (&str, Option<&str>) {
    match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    }
}

fn read_request_body(conn: &mut Conn) -> io::Result<()> {
    let mut tmp = [0u8; 4096];
    loop {
        let remaining = match conn.phase {
            Phase::RelayRequestBody { remaining, .. } => remaining,
            _ => return Ok(()),
        };
        if remaining == 0 || conn.to_upstream.available() == 0 {
            return Ok(());
        }
        let cap = conn.to_upstream.available().min(tmp.len());
        let want = usize::try_from(remaining).unwrap_or(usize::MAX).min(cap);
        match conn.client.read(&mut tmp[..want]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "client closed during request body",
                ))
            }
            Ok(n) => {
                conn.to_upstream.append(&tmp[..n])?;
                if let Phase::RelayRequestBody {
                    remaining,
                    last_progress,
                } = &mut conn.phase
                {
                    *remaining = remaining.saturating_sub(u64::try_from(n).unwrap_or(0));
                    *last_progress = Instant::now();
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn attach_upstream(
    registry: &Registry,
    conn: &mut Conn,
    target: SocketAddr,
    pool: &mut UpstreamPool,
    upstream_to_client: &mut HashMap<Token, Token>,
    next_token: &mut Token,
    gen_id: u64,
    obs: &crate::obs::ObsHubHandle,
) -> io::Result<()> {
    let (mut stream, reused) = match pool.take(registry, target, gen_id) {
        Some(s) => (s, true),
        None => (UpstreamPool::connect(target)?, false),
    };
    if reused {
        obs.upstream_reuse.fetch_add(1, Ordering::Relaxed);
    } else {
        obs.upstream_connects.fetch_add(1, Ordering::Relaxed);
    }
    let token = alloc_token(next_token);
    // Hold READABLE through first request write (ADR-021 checkout→write TOCTOU).
    registry.register(
        &mut stream,
        token,
        Interest::READABLE.add(Interest::WRITABLE),
    )?;
    upstream_to_client.insert(token, conn.client_token);
    conn.upstream = Some(UpstreamSide {
        stream,
        token,
        target,
        registered: true,
        reuse_ok: true,
        await_first_write: true,
    });
    Ok(())
}

fn read_upstream_headers(conn: &mut Conn) -> io::Result<()> {
    let mut tmp = [0u8; 4096];
    loop {
        match conn.upstream.as_mut() {
            Some(up) => match up.stream.read(&mut tmp) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "upstream eof")),
                Ok(n) => {
                    conn.upstream_head.extend_from_slice(&tmp[..n]);
                    if conn.upstream_head.len() > MAX_HEADER_BYTES + MAX_REQUEST_LINE + 4 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "upstream header limit",
                        ));
                    }
                    let Some(header_end) = find_header_end(&conn.upstream_head) else {
                        continue;
                    };
                    let parsed = parse_upstream_response_head(
                        &conn.upstream_head[..header_end],
                        conn.request_head,
                    )?;
                    let body_start = conn.upstream_head.split_off(header_end);
                    conn.upstream_head.clear();
                    let head = assemble_client_response_head(
                        &parsed.status_line,
                        &parsed.headers,
                        parsed.content_length,
                        conn.client_close || parsed.force_client_close,
                    );
                    // Response head is staged outside the 16 KiB body relay buffer.
                    conn.to_client_head = head;
                    let allowed_body = usize::try_from(parsed.content_length).unwrap_or(usize::MAX);
                    let mut body = body_start;
                    let mut reuse_ok = parsed.reuse_ok;
                    // Extra bytes after declared CL are never forwarded to the client.
                    // Poison upstream reuse (do not 502 the already-framed response).
                    if body.len() > allowed_body {
                        body.truncate(allowed_body);
                        reuse_ok = false;
                    }
                    if !body.is_empty() {
                        conn.to_client.append(&body)?;
                    }
                    if let Some(up) = conn.upstream.as_mut() {
                        up.reuse_ok = reuse_ok;
                    }
                    conn.response_started = true;
                    if let Some(code) = parsed
                        .status_line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|c| c.parse::<u16>().ok())
                    {
                        conn.last_status = code;
                    } else {
                        conn.last_status = 200;
                    }
                    conn.phase = Phase::RelayResponseBody {
                        remaining: parsed
                            .content_length
                            .saturating_sub(u64::try_from(body.len()).unwrap_or(0)),
                        last_progress: Instant::now(),
                    };
                    return Ok(());
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            },
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "missing upstream",
                ))
            }
        }
    }
}

fn read_response_body(conn: &mut Conn) -> io::Result<()> {
    let mut tmp = [0u8; 4096];
    loop {
        let remaining = match conn.phase {
            Phase::RelayResponseBody { remaining, .. } => remaining,
            _ => return Ok(()),
        };
        if remaining == 0 || conn.to_client.available() == 0 {
            return Ok(());
        }
        let cap = conn.to_client.available().min(tmp.len());
        let want = usize::try_from(remaining).unwrap_or(usize::MAX).min(cap);
        match conn.upstream.as_mut() {
            Some(up) => match up.stream.read(&mut tmp[..want]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "upstream closed during response body",
                    ))
                }
                Ok(n) => {
                    conn.to_client.append(&tmp[..n])?;
                    if let Phase::RelayResponseBody {
                        remaining,
                        last_progress,
                    } = &mut conn.phase
                    {
                        *remaining = remaining.saturating_sub(u64::try_from(n).unwrap_or(0));
                        *last_progress = Instant::now();
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            },
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "missing upstream",
                ))
            }
        }
    }
}

struct ParsedUpstreamResponse {
    status_line: String,
    headers: Vec<Header>,
    content_length: u64,
    reuse_ok: bool,
    force_client_close: bool,
}

fn parse_upstream_response_head(
    head: &[u8],
    request_head: bool,
) -> io::Result<ParsedUpstreamResponse> {
    if headers_have_illegal_line_endings(head) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "upstream illegal header line endings",
        ));
    }
    let text = std::str::from_utf8(head)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "upstream header utf8"))?;
    let mut lines = text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing status line"))?;
    if !status_line.starts_with("HTTP/1.") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported upstream version",
        ));
    }
    let mut headers = Vec::new();
    let mut content_length = None;
    let mut saw_te = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed upstream header",
            ));
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate upstream content-length",
                ));
            }
            let n = value.parse::<u64>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid upstream content-length",
                )
            })?;
            content_length = Some(n);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            saw_te = true;
        }
        headers.push(Header {
            name: name.to_string(),
            value: value.to_string(),
        });
    }
    if saw_te {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "upstream chunked unsupported",
        ));
    }
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let bodyless = request_head || matches!(code, 204 | 304);
    let cl = if bodyless {
        0
    } else {
        content_length.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "missing upstream content-length",
            )
        })?
    };
    let (mut filtered, up_close) = filter_response_headers(&headers);
    if bodyless {
        filtered.retain(|h| !h.name.eq_ignore_ascii_case("content-length"));
    }
    Ok(ParsedUpstreamResponse {
        status_line: status_line.to_string(),
        headers: filtered,
        content_length: cl,
        reuse_ok: !up_close,
        force_client_close: false,
    })
}

fn write_pending_bytes(stream: &mut TcpStream, buf: &mut Vec<u8>) -> io::Result<()> {
    while !buf.is_empty() {
        match stream.write(buf) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "write zero")),
            Ok(n) => {
                buf.drain(..n);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Write staged HTTP head bytes fully before any body relay buffer bytes.
fn write_pending_head_then_body(
    stream: &mut TcpStream,
    head: &mut Vec<u8>,
    body: &mut RelayBuffer,
) -> io::Result<()> {
    write_pending_bytes(stream, head)?;
    if !head.is_empty() {
        return Ok(());
    }
    write_pending(stream, body)
}

fn write_pending(stream: &mut TcpStream, buf: &mut RelayBuffer) -> io::Result<()> {
    while !buf.is_empty() {
        match stream.write(buf.pending()) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "write zero")),
            Ok(n) => buf.consume(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_response(
    registry: &Registry,
    conn: &mut Conn,
    pool: &mut UpstreamPool,
    upstream_to_client: &mut HashMap<Token, Token>,
    next_token: &mut Token,
    gen_id: u64,
    counters: &mut Counters,
    args: &ShardArgs,
) -> io::Result<Action> {
    if let Some(mut up) = conn.upstream.take() {
        if up.registered {
            let _ = registry.deregister(&mut up.stream);
            up.registered = false;
        }
        upstream_to_client.remove(&up.token);
        // ADR-021 Hybrid F: put only when reuse_ok after dual-boundary validation inside put().
        if up.reuse_ok {
            pool.put(registry, up.target, up.stream, gen_id, next_token);
        } else {
            UpstreamPool::discard(up.stream);
        }
    }
    counters.bytes_out = counters
        .bytes_out
        .saturating_add(u64::try_from(conn.to_client.pos).unwrap_or(0));
    if conn.obs_inflight && !conn.obs_accounted {
        let latency_us = conn.req_started.elapsed().as_micros() as u64;
        let bytes_out = u64::try_from(conn.to_client.pos).unwrap_or(0);
        let status = if conn.last_status == 0 {
            200
        } else {
            conn.last_status
        };
        obs_finalize(
            conn,
            ObsNote {
                args,
                event: crate::obs::ObsEvent::Access,
                method: conn.obs_method,
                status,
                bytes_in: conn.req_bytes_in,
                bytes_out,
                latency_us,
                had_body: conn.req_bytes_in > 0,
                error_class: "none",
            },
        );
    }
    if conn.client_close {
        Ok(Action::Close)
    } else {
        reset_for_keepalive(conn);
        Ok(Action::Keep)
    }
}

fn reset_for_keepalive(conn: &mut Conn) {
    conn.client_head.clear();
    conn.upstream_head.clear();
    conn.to_upstream_head.clear();
    conn.to_client_head.clear();
    conn.to_upstream.clear();
    conn.to_client.clear();
    conn.upstream = None;
    conn.client_close = false;
    conn.request_head = false;
    conn.response_started = false;
    conn.req_bytes_in = 0;
    conn.last_status = 0;
    conn.obs_bytes_out_expected = 0;
    conn.obs_inflight = false;
    conn.obs_accounted = false;
    conn.obs_method = "UNKNOWN";
    conn.phase = Phase::Idle {
        since: Instant::now(),
    };
}

fn queue_error(conn: &mut Conn, code: u16, body: &str) -> io::Result<()> {
    let reason = reason_phrase(code);
    let msg = client_response(code, reason, body.as_bytes(), &[], true);
    queue_response(conn, &msg, true)
}

fn queue_response(conn: &mut Conn, response: &[u8], close: bool) -> io::Result<()> {
    // Synthetic responses are small and complete; stage as head so body relay stays empty.
    conn.to_client_head.clear();
    conn.to_client.clear();
    conn.to_client_head.extend_from_slice(response);
    conn.phase = Phase::WriteClientResponse {
        close,
        since: Instant::now(),
    };
    Ok(())
}

fn update_interests(
    registry: &Registry,
    conn: &mut Conn,
    counters: &mut Counters,
    obs: &crate::obs::ObsHubHandle,
) -> io::Result<()> {
    let client_read = match conn.phase {
        Phase::ReadClientHeaders { .. } | Phase::Idle { .. } => true,
        Phase::RelayRequestBody { remaining, .. } => {
            remaining > 0 && conn.to_upstream.available() > 0
        }
        _ => false,
    };
    let client_write = !conn.to_client_head.is_empty()
        || !conn.to_client.is_empty()
        || matches!(conn.phase, Phase::StaticSend { remaining, .. } if remaining > 0);
    if matches!(conn.phase, Phase::RelayRequestBody { remaining, .. } if remaining > 0)
        && conn.to_upstream.available() == 0
    {
        counters.backpressure_client_paused = counters.backpressure_client_paused.saturating_add(1);
        obs.backpressure_activations.fetch_add(1, Ordering::Relaxed);
    }
    apply_interest(
        registry,
        &mut conn.client,
        &mut conn.client_registered,
        conn.client_token,
        client_read,
        client_write,
    )?;

    let await_first = conn
        .upstream
        .as_ref()
        .map(|u| u.await_first_write)
        .unwrap_or(false);
    let upstream_read = match conn.phase {
        Phase::ReadUpstreamHeaders { .. } => true,
        Phase::RelayResponseBody { remaining, .. } => {
            remaining > 0 && conn.to_client.available() > 0
        }
        Phase::RelayRequestBody { .. } if await_first => true,
        _ => await_first,
    };
    let upstream_write = !conn.to_upstream_head.is_empty() || !conn.to_upstream.is_empty();
    if matches!(conn.phase, Phase::RelayResponseBody { remaining, .. } if remaining > 0)
        && conn.to_client.available() == 0
    {
        counters.backpressure_upstream_paused =
            counters.backpressure_upstream_paused.saturating_add(1);
        obs.backpressure_activations.fetch_add(1, Ordering::Relaxed);
    }
    if let Some(up) = conn.upstream.as_mut() {
        apply_interest(
            registry,
            &mut up.stream,
            &mut up.registered,
            up.token,
            upstream_read,
            upstream_write,
        )?;
    }
    Ok(())
}

fn apply_interest(
    registry: &Registry,
    stream: &mut TcpStream,
    registered: &mut bool,
    token: Token,
    readable: bool,
    writable: bool,
) -> io::Result<()> {
    let Some(interest) = interest(readable, writable) else {
        if *registered {
            registry.deregister(stream)?;
            *registered = false;
        }
        return Ok(());
    };
    if *registered {
        registry.reregister(stream, token, interest)?;
    } else {
        registry.register(stream, token, interest)?;
        *registered = true;
    }
    Ok(())
}

fn interest(readable: bool, writable: bool) -> Option<Interest> {
    match (readable, writable) {
        (true, true) => Some(Interest::READABLE.add(Interest::WRITABLE)),
        (true, false) => Some(Interest::READABLE),
        (false, true) => Some(Interest::WRITABLE),
        (false, false) => None,
    }
}

fn mark_body_progress(conn: &mut Conn) {
    let now = Instant::now();
    match &mut conn.phase {
        Phase::RelayRequestBody { last_progress, .. }
        | Phase::RelayResponseBody { last_progress, .. } => *last_progress = now,
        _ => {}
    }
}

fn cleanup_conn(
    registry: &Registry,
    conn: &mut Conn,
    upstream_to_client: &mut HashMap<Token, Token>,
    obs: &crate::obs::ObsHubHandle,
) {
    if conn.client_registered {
        let _ = registry.deregister(&mut conn.client);
        conn.client_registered = false;
    }
    discard_upstream(registry, conn, upstream_to_client, obs);
    obs.closed_connections.fetch_add(1, Ordering::Relaxed);
    let _ = obs
        .active_connections
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            Some(v.saturating_sub(1))
        });
}

fn discard_upstream(
    registry: &Registry,
    conn: &mut Conn,
    upstream_to_client: &mut HashMap<Token, Token>,
    obs: &crate::obs::ObsHubHandle,
) {
    conn.to_upstream_head.clear();
    conn.to_upstream.clear();
    if let Some(mut up) = conn.upstream.take() {
        if up.registered {
            let _ = registry.deregister(&mut up.stream);
        }
        upstream_to_client.remove(&up.token);
        UpstreamPool::discard(up.stream);
        obs.upstream_discard.fetch_add(1, Ordering::Relaxed);
        obs.upstream_disconnects.fetch_add(1, Ordering::Relaxed);
    }
}

fn path_without_query(target: &str) -> &str {
    let no_frag = target.split('#').next().unwrap_or(target);
    no_frag.split('?').next().unwrap_or(no_frag)
}

fn split_path_query(target: &str) -> (&str, Option<&str>) {
    match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target, None),
    }
}

fn map_headers_from_parsed(headers: &[Header]) -> MapHeaders {
    MapHeaders {
        entries: headers
            .iter()
            .map(|h| (h.name.to_ascii_lowercase(), h.value.as_bytes().to_vec()))
            .collect(),
    }
}

fn request_target_for_upstream(target: &str) -> &str {
    target.split('#').next().unwrap_or(target)
}

fn method_static(method: &str) -> &'static str {
    match method {
        "GET" | "get" => "GET",
        "POST" | "post" => "POST",
        "PUT" | "put" => "PUT",
        "HEAD" | "head" => "HEAD",
        "DELETE" | "delete" => "DELETE",
        "OPTIONS" | "options" => "OPTIONS",
        "PATCH" | "patch" => "PATCH",
        _ => "OTHER",
    }
}

fn is_foundation(p: &ParsedRequest) -> bool {
    p.version == HttpVersion::Http11 && path_without_query(&p.path) == FOUNDATION_PATH
}

fn map_parse_error(e: &ParseError) -> (u16, &'static str) {
    match e {
        ParseError::Incomplete => (400, "incomplete\n"),
        ParseError::InvalidMethod => (405, "method not allowed\n"),
        ParseError::BodyNotSupported | ParseError::AmbiguousFraming => {
            (400, "request body framing unsupported\n")
        }
        ParseError::MissingHost | ParseError::DuplicateHost => (400, "host error\n"),
        ParseError::RequestLineTooLong
        | ParseError::HeaderTooLarge
        | ParseError::TooManyHeaders => (431, "header limits\n"),
        _ => (400, "bad request\n"),
    }
}

fn assemble_client_response_head(
    status_line: &str,
    headers: &[Header],
    content_length: u64,
    client_close: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(status_line.as_bytes());
    out.extend_from_slice(b"\r\n");
    for h in headers {
        if h.name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        out.extend_from_slice(h.name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(h.value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"Connection: ");
    if client_close {
        out.extend_from_slice(b"close\r\n");
    } else {
        out.extend_from_slice(b"keep-alive\r\n");
    }
    out.extend_from_slice(b"Content-Length: ");
    out.extend_from_slice(content_length.to_string().as_bytes());
    out.extend_from_slice(b"\r\n\r\n");
    out
}

fn client_response(
    code: u16,
    reason: &str,
    body: &[u8],
    extras: &[(&str, &str)],
    close: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(256 + body.len());
    out.extend_from_slice(format!("HTTP/1.1 {code} {reason}\r\n").as_bytes());
    for (k, v) in extras {
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if close {
        out.extend_from_slice(b"Connection: close\r\n");
    } else {
        out.extend_from_slice(b"Connection: keep-alive\r\n");
    }
    out.extend_from_slice(b"Content-Length: ");
    out.extend_from_slice(body.len().to_string().as_bytes());
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(body);
    out
}

fn static_response_head(
    code: u16,
    content_length: u64,
    content_type: &'static str,
    close: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(192);
    out.extend_from_slice(b"HTTP/1.1 ");
    out.extend_from_slice(code.to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(reason_phrase(code).as_bytes());
    out.extend_from_slice(b"\r\nContent-Length: ");
    out.extend_from_slice(content_length.to_string().as_bytes());
    out.extend_from_slice(b"\r\nContent-Type: ");
    out.extend_from_slice(content_type.as_bytes());
    out.extend_from_slice(b"\r\nConnection: ");
    if close {
        out.extend_from_slice(b"close\r\n\r\n");
    } else {
        out.extend_from_slice(b"keep-alive\r\n\r\n");
    }
    out
}

fn write_all_timeout(stream: &mut TcpStream, data: &[u8], timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut off = 0;
    while off < data.len() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "write timeout"));
        }
        match stream.write(&data[off..]) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "write zero")),
            Ok(n) => off += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn write_simple(stream: &mut TcpStream, code: u16, body: &str) -> io::Result<()> {
    let msg = client_response(code, reason_phrase(code), body.as_bytes(), &[], true);
    write_all_timeout(stream, &msg, Duration::from_secs(2))
}

fn reason_phrase(code: u16) -> &'static str {
    match code {
        200 => "OK",
        204 => "No Content",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}

fn bind_reuseport(addr: SocketAddr) -> io::Result<std::net::TcpListener> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    Ok(socket.into())
}
