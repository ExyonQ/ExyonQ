/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::eligibility::{peek_sdp_eligible, SdpEligibility};
use crate::obs::DataplaneCounters;
use crate::parse::{self, ParseErr};
use crate::plan::{Method, ProxyExecutionPlan, WafAdmission};
use crate::projection::SharedProjection;
use crate::PATH_ID_SDP_P1;
use exyonq_waf_api::{
    apply_waf_mode, HeaderView, WafAction, WafEngine, WafMode, WafPhase, WafRequest,
};
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Token};
use std::collections::{HashMap, VecDeque};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const LISTENER: Token = Token(0);
const MAX_IDLE_UPSTREAM: usize = 64;
const BUF: usize = 16 * 1024;
/// Downstream write high-water before pausing upstream read (bounded relay).
pub const DOWNSTREAM_BUFFER_MAX: usize = 64 * 1024;
pub const UPSTREAM_READ_HIGH_WATER: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    ClientReadingHeaders,
    EligibilityCheck,
    PlanReady,
    WafAdmission,
    UpstreamConnecting,
    UpstreamWritingRequest,
    UpstreamReadingHeaders,
    RelayingBody,
    DownstreamFlush,
    KeepaliveIdle,
    Closing,
    Error,
}

struct Session {
    client: TcpStream,
    client_token: Token,
    upstream: Option<TcpStream>,
    upstream_token: Option<Token>,
    phase: Phase,
    client_in: Vec<u8>,
    upstream_in: Vec<u8>,
    pending_out: Vec<u8>,
    pending_off: usize,
    remaining_body: usize,
    client_ka: bool,
    upstream_ka: bool,
    last_active: Instant,
    plan: Option<ProxyExecutionPlan>,
    upstream_addr: Option<SocketAddr>,
}

pub(crate) struct ShardArgs {
    pub shard_id: usize,
    pub listen: SocketAddr,
    pub projection: SharedProjection,
    pub counters: Arc<DataplaneCounters>,
    pub stop: Arc<AtomicBool>,
    pub handoff: Arc<dyn Fn(std::net::TcpStream, Vec<u8>) + Send + Sync>,
}

pub(crate) fn run_shard(args: ShardArgs) -> io::Result<()> {
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(1024);
    let listener = bind_reuseport(args.listen)?;
    let mut listener = listener;
    poll.registry()
        .register(&mut listener, LISTENER, Interest::READABLE)?;

    let mut sessions: HashMap<Token, Session> = HashMap::new();
    let mut up_to_client: HashMap<Token, Token> = HashMap::new();
    let mut idle_upstream: VecDeque<(SocketAddr, TcpStream)> = VecDeque::new();
    let mut next_token = Token(1);
    let mut scratch = vec![0u8; BUF];
    let idle_timeout = Duration::from_secs(60);

    while !args.stop.load(Ordering::Relaxed) {
        poll.poll(&mut events, Some(Duration::from_millis(200)))?;
        let now = Instant::now();

        for event in events.iter() {
            match event.token() {
                LISTENER => loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = stream.set_nodelay(true);
                            let mut peek_buf = [0u8; 512];
                            let n = match stream.peek(&mut peek_buf) {
                                Ok(n) => n,
                                Err(e) if e.kind() == ErrorKind::WouldBlock => 0,
                                Err(_) => {
                                    let _ = stream.shutdown(Shutdown::Both);
                                    continue;
                                }
                            };
                            match peek_sdp_eligible(&peek_buf[..n]) {
                                SdpEligibility::HandoffLegacy => {
                                    args.counters.handoff_legacy.fetch_add(1, Ordering::Relaxed);
                                    // peek is non-consuming — hand off raw socket; legacy path reads wire.
                                    let std = mio_to_std(stream);
                                    (args.handoff)(std, Vec::new());
                                }
                                SdpEligibility::RejectClose => {
                                    args.counters.errors.fetch_add(1, Ordering::Relaxed);
                                    let _ = stream.shutdown(Shutdown::Both);
                                }
                                SdpEligibility::Specialized => {
                                    let token = next_token;
                                    next_token = Token(next_token.0 + 1);
                                    poll.registry().register(
                                        &mut stream,
                                        token,
                                        Interest::READABLE | Interest::WRITABLE,
                                    )?;
                                    sessions.insert(
                                        token,
                                        Session {
                                            client: stream,
                                            client_token: token,
                                            upstream: None,
                                            upstream_token: None,
                                            phase: Phase::ClientReadingHeaders,
                                            client_in: Vec::with_capacity(512),
                                            upstream_in: Vec::with_capacity(512),
                                            pending_out: Vec::new(),
                                            pending_off: 0,
                                            remaining_body: 0,
                                            client_ka: true,
                                            upstream_ka: false,
                                            last_active: now,
                                            plan: None,
                                            upstream_addr: None,
                                        },
                                    );
                                }
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                },
                token => {
                    let client_tok = up_to_client.get(&token).copied().unwrap_or(token);
                    if !sessions.contains_key(&client_tok) {
                        continue;
                    }
                    let mut drive = DriveCtx {
                        sessions: &mut sessions,
                        up_to_client: &mut up_to_client,
                        idle_upstream: &mut idle_upstream,
                        poll: &mut poll,
                        scratch: &mut scratch,
                        args: &args,
                        now,
                    };
                    let action = drive_session(&mut drive, client_tok, &mut next_token);
                    if action == Action::Close {
                        close_session(
                            &mut sessions,
                            &mut up_to_client,
                            &mut idle_upstream,
                            &mut poll,
                            client_tok,
                        );
                    }
                }
            }
        }

        // Idle reap
        let stale: Vec<Token> = sessions
            .iter()
            .filter(|(_, s)| {
                s.phase == Phase::KeepaliveIdle && now.duration_since(s.last_active) > idle_timeout
            })
            .map(|(t, _)| *t)
            .collect();
        for t in stale {
            close_session(
                &mut sessions,
                &mut up_to_client,
                &mut idle_upstream,
                &mut poll,
                t,
            );
        }
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
enum Action {
    Continue,
    Close,
}

/// Shard-local mutable handles for one session drive step.
///
/// ADR-021 / clippy `too_many_arguments`: these are not independent domain
/// parameters — they are one event-loop context. Bundling keeps ownership
/// explicit without Arc/clone on the hot path. `next_token` stays a separate
/// `&mut Token` so destructuring does not produce `&mut &mut Token`.
struct DriveCtx<'a> {
    sessions: &'a mut HashMap<Token, Session>,
    up_to_client: &'a mut HashMap<Token, Token>,
    idle_upstream: &'a mut VecDeque<(SocketAddr, TcpStream)>,
    poll: &'a mut Poll,
    scratch: &'a mut [u8],
    args: &'a ShardArgs,
    now: Instant,
}

fn drive_session(ctx: &mut DriveCtx<'_>, client_tok: Token, next_token: &mut Token) -> Action {
    let DriveCtx {
        sessions,
        up_to_client,
        idle_upstream,
        poll,
        scratch,
        args,
        now,
    } = ctx;
    let now = *now;
    let Some(sess) = sessions.get_mut(&client_tok) else {
        return Action::Close;
    };
    sess.last_active = now;

    // Flush pending client writes first.
    if sess.pending_off < sess.pending_out.len() {
        match sess.client.write(&sess.pending_out[sess.pending_off..]) {
            Ok(0) => return Action::Close,
            Ok(n) => {
                sess.pending_off += n;
                if sess.pending_off >= sess.pending_out.len() {
                    sess.pending_out.clear();
                    sess.pending_off = 0;
                } else {
                    return Action::Continue;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Action::Continue,
            Err(_) => return Action::Close,
        }
    }

    loop {
        match sess.phase {
            Phase::ClientReadingHeaders | Phase::EligibilityCheck => {
                match sess.client.read(scratch) {
                    Ok(0) => return Action::Close,
                    Ok(n) => sess.client_in.extend_from_slice(&scratch[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => return Action::Continue,
                    Err(_) => return Action::Close,
                }
                match parse::parse_client_request(&sess.client_in) {
                    Err(ParseErr::Incomplete) => return Action::Continue,
                    Err(_) => {
                        args.counters.errors.fetch_add(1, Ordering::Relaxed);
                        queue_simple(sess, b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                        sess.phase = Phase::DownstreamFlush;
                        sess.client_ka = false;
                        continue;
                    }
                    Ok(req) => {
                        // Drop header bytes; GET has no body.
                        sess.client_in.drain(..req.header_block_len);
                        sess.client_ka = req.keepalive;
                        sess.phase = Phase::PlanReady;
                        let proj = args.projection.load_full();
                        let Some((route_idx, cluster_id, upstream, host_hdr, timeout)) =
                            proj.resolve_proxy_upstream(&req.path_query, req.host.as_deref())
                        else {
                            args.counters.errors.fetch_add(1, Ordering::Relaxed);
                            queue_simple(sess, b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                            sess.phase = Phase::DownstreamFlush;
                            sess.client_ka = false;
                            continue;
                        };
                        let client_ip = sess
                            .client
                            .peer_addr()
                            .map(|a| a.ip())
                            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
                        let waf =
                            evaluate_waf(&proj, &req.path_query, req.host.as_deref(), client_ip);
                        let plan = ProxyExecutionPlan {
                            generation_id: proj.generation_id,
                            method: Method::Get,
                            path_query: req.path_query.clone(),
                            host: req.host.clone(),
                            route_idx,
                            cluster_id,
                            upstream,
                            upstream_host_header: host_hdr,
                            client_keepalive: req.keepalive,
                            waf,
                            connect_timeout: timeout,
                            header_timeout: proj.default_header_timeout,
                            idle_timeout: proj.default_idle_timeout,
                            path_identity: PATH_ID_SDP_P1,
                        };
                        sess.upstream_addr = Some(upstream);
                        sess.plan = Some(plan);
                        sess.phase = Phase::WafAdmission;
                        args.counters.requests.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Phase::PlanReady => {
                sess.phase = Phase::WafAdmission;
            }
            Phase::WafAdmission => {
                let plan = sess.plan.as_ref().expect("plan");
                match &plan.waf {
                    WafAdmission::Reject {
                        status,
                        body,
                        content_type,
                    } => {
                        args.counters.waf_deny.fetch_add(1, Ordering::Relaxed);
                        args.counters.responses_4xx.fetch_add(1, Ordering::Relaxed);
                        let mut resp = format!(
                            "HTTP/1.1 {status} \r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\ncache-control: no-store\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        resp.extend_from_slice(body);
                        queue_bytes(sess, resp);
                        sess.phase = Phase::DownstreamFlush;
                        sess.client_ka = false;
                    }
                    WafAdmission::Allow => {
                        sess.phase = Phase::UpstreamConnecting;
                    }
                }
            }
            Phase::UpstreamConnecting => {
                let addr = sess.upstream_addr.expect("addr");
                let mut up = if let Some(i) = idle_upstream.iter().position(|(a, _)| *a == addr) {
                    args.counters.upstream_reuse.fetch_add(1, Ordering::Relaxed);
                    idle_upstream.remove(i).expect("idle").1
                } else {
                    match TcpStream::connect(addr) {
                        Ok(s) => s,
                        Err(_) => {
                            args.counters.responses_5xx.fetch_add(1, Ordering::Relaxed);
                            queue_simple(sess, b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                            sess.phase = Phase::DownstreamFlush;
                            sess.client_ka = false;
                            continue;
                        }
                    }
                };
                let _ = up.set_nodelay(true);
                let ut = *next_token;
                *next_token = Token(next_token.0 + 1);
                if poll
                    .registry()
                    .register(&mut up, ut, Interest::READABLE | Interest::WRITABLE)
                    .is_err()
                {
                    return Action::Close;
                }
                up_to_client.insert(ut, client_tok);
                sess.upstream = Some(up);
                sess.upstream_token = Some(ut);
                // Build upstream request from plan.
                let plan = sess.plan.as_ref().expect("plan");
                let req = format!(
                    "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: keep-alive\r\n\r\n",
                    plan.path_query, plan.upstream_host_header
                );
                queue_bytes(sess, req.into_bytes());
                // pending_out is for client — we need separate upstream write buffer.
                // Reuse: write directly in UpstreamWritingRequest.
                sess.upstream_in = sess.pending_out.clone();
                sess.pending_out.clear();
                sess.pending_off = 0;
                sess.phase = Phase::UpstreamWritingRequest;
            }
            Phase::UpstreamWritingRequest => {
                let Some(up) = sess.upstream.as_mut() else {
                    return Action::Close;
                };
                match up.write(&sess.upstream_in[sess.pending_off..]) {
                    Ok(0) => return Action::Close,
                    Ok(n) => {
                        sess.pending_off += n;
                        if sess.pending_off >= sess.upstream_in.len() {
                            sess.upstream_in.clear();
                            sess.pending_off = 0;
                            sess.phase = Phase::UpstreamReadingHeaders;
                        } else {
                            return Action::Continue;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => return Action::Continue,
                    Err(_) => {
                        args.counters.responses_5xx.fetch_add(1, Ordering::Relaxed);
                        return Action::Close;
                    }
                }
            }
            Phase::UpstreamReadingHeaders => {
                let Some(up) = sess.upstream.as_mut() else {
                    return Action::Close;
                };
                match up.read(scratch) {
                    Ok(0) => {
                        args.counters.responses_5xx.fetch_add(1, Ordering::Relaxed);
                        return Action::Close;
                    }
                    Ok(n) => sess.upstream_in.extend_from_slice(&scratch[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => return Action::Continue,
                    Err(_) => return Action::Close,
                }
                match parse::parse_upstream_response(&sess.upstream_in) {
                    Err(ParseErr::Incomplete) => return Action::Continue,
                    Err(_) => {
                        args.counters.responses_5xx.fetch_add(1, Ordering::Relaxed);
                        queue_simple(sess, b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                        sess.phase = Phase::DownstreamFlush;
                        sess.client_ka = false;
                        continue;
                    }
                    Ok(resp) => {
                        sess.upstream_ka = resp.keepalive;
                        sess.remaining_body = resp.content_length;
                        let header = sess.upstream_in[..resp.header_end].to_vec();
                        let extra = sess.upstream_in[resp.header_end..].to_vec();
                        sess.upstream_in.clear();
                        // Rewrite Connection for client KA preference.
                        let out_hdr = rewrite_response_connection(&header, sess.client_ka);
                        queue_bytes(sess, out_hdr);
                        if !extra.is_empty() {
                            let take = extra.len().min(sess.remaining_body);
                            sess.remaining_body -= take;
                            sess.pending_out.extend_from_slice(&extra[..take]);
                        }
                        if sess.remaining_body == 0 {
                            sess.phase = Phase::DownstreamFlush;
                        } else {
                            sess.phase = Phase::RelayingBody;
                        }
                        if resp.status >= 500 {
                            args.counters.responses_5xx.fetch_add(1, Ordering::Relaxed);
                        } else if resp.status >= 400 {
                            args.counters.responses_4xx.fetch_add(1, Ordering::Relaxed);
                        } else {
                            args.counters.responses_2xx.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
            Phase::RelayingBody => {
                // Bound: if pending client buffer large, wait for writable.
                if sess.pending_out.len().saturating_sub(sess.pending_off) >= DOWNSTREAM_BUFFER_MAX
                {
                    return Action::Continue;
                }
                let want = sess
                    .remaining_body
                    .min(scratch.len())
                    .min(UPSTREAM_READ_HIGH_WATER);
                let Some(up) = sess.upstream.as_mut() else {
                    return Action::Close;
                };
                match up.read(&mut scratch[..want]) {
                    Ok(0) => {
                        if sess.remaining_body > 0 {
                            args.counters.errors.fetch_add(1, Ordering::Relaxed);
                        }
                        return Action::Close;
                    }
                    Ok(n) => {
                        let n = n.min(sess.remaining_body);
                        sess.remaining_body -= n;
                        sess.pending_out.extend_from_slice(&scratch[..n]);
                        if sess.remaining_body == 0 {
                            sess.phase = Phase::DownstreamFlush;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => return Action::Continue,
                    Err(_) => return Action::Close,
                }
            }
            Phase::DownstreamFlush => {
                if sess.pending_off < sess.pending_out.len() {
                    return Action::Continue; // wait writable
                }
                if sess.client_ka {
                    // Recycle upstream if KA.
                    if sess.upstream_ka {
                        if let (Some(mut up), Some(ut), Some(addr)) = (
                            sess.upstream.take(),
                            sess.upstream_token.take(),
                            sess.upstream_addr,
                        ) {
                            let _ = poll.registry().deregister(&mut up);
                            up_to_client.remove(&ut);
                            if idle_upstream.len() < MAX_IDLE_UPSTREAM {
                                idle_upstream.push_back((addr, up));
                            }
                        }
                    } else if let (Some(mut up), Some(ut)) =
                        (sess.upstream.take(), sess.upstream_token.take())
                    {
                        let _ = poll.registry().deregister(&mut up);
                        up_to_client.remove(&ut);
                    }
                    sess.plan = None;
                    sess.client_in.clear();
                    sess.upstream_in.clear();
                    sess.phase = Phase::KeepaliveIdle;
                    return Action::Continue;
                }
                return Action::Close;
            }
            Phase::KeepaliveIdle => {
                sess.phase = Phase::ClientReadingHeaders;
            }
            Phase::Closing | Phase::Error => return Action::Close,
        }
        // After phase transition, try flush pending if any.
        if sess.pending_off < sess.pending_out.len() {
            match sess.client.write(&sess.pending_out[sess.pending_off..]) {
                Ok(0) => return Action::Close,
                Ok(n) => {
                    sess.pending_off += n;
                    if sess.pending_off >= sess.pending_out.len() {
                        sess.pending_out.clear();
                        sess.pending_off = 0;
                    } else {
                        return Action::Continue;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Action::Continue,
                Err(_) => return Action::Close,
            }
        }
    }
}

fn evaluate_waf(
    proj: &crate::projection::GenerationProjection,
    path_query: &str,
    host: Option<&str>,
    client_ip: std::net::IpAddr,
) -> WafAdmission {
    evaluate_waf_admission(
        proj.waf_wire_inspection_active,
        proj.waf_enforce,
        proj.waf.as_ref(),
        path_query,
        host,
        client_ip,
    )
}

/// SDP-001: always consult WAF when present; pass real peer IP (not 0.0.0.0).
fn evaluate_waf_admission(
    waf_wire_inspection_active: bool,
    waf_enforce: bool,
    waf: &dyn WafEngine,
    path_query: &str,
    host: Option<&str>,
    client_ip: std::net::IpAddr,
) -> WafAdmission {
    // SDP-001: do not short-circuit to Allow when wire-inspection flag is off —
    // still evaluate the engine so Block decisions are not silently dropped.
    let _ = waf_wire_inspection_active;
    let (path, query) = match path_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_query, None),
    };
    struct HostHdr<'a>(Option<&'a str>);
    impl HeaderView for HostHdr<'_> {
        fn get(&self, name: &str) -> Option<&[u8]> {
            if name.eq_ignore_ascii_case("host") {
                self.0.map(|h| h.as_bytes())
            } else {
                None
            }
        }
        fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) {
            if let Some(h) = self.0 {
                f(b"host", h.as_bytes());
            }
        }
    }
    let headers = HostHdr(host);
    let req = WafRequest {
        method: "GET",
        host,
        path,
        query,
        headers: &headers,
        client_ip,
        route_id: None,
    };
    let decision = waf.inspect(WafPhase::RequestHeaders, &req);
    let mode = if waf_enforce {
        WafMode::Block
    } else {
        WafMode::Monitor
    };
    let decided = apply_waf_mode(mode, decision);
    if decided.action == WafAction::Block && waf_enforce {
        WafAdmission::Reject {
            status: 403,
            body: b"forbidden\n".to_vec(),
            content_type: "text/plain".into(),
        }
    } else {
        WafAdmission::Allow
    }
}

fn queue_simple(sess: &mut Session, bytes: &'static [u8]) {
    sess.pending_out.extend_from_slice(bytes);
    sess.pending_off = 0;
}

fn queue_bytes(sess: &mut Session, bytes: Vec<u8>) {
    sess.pending_out.extend_from_slice(&bytes);
}

fn rewrite_response_connection(header_block: &[u8], client_ka: bool) -> Vec<u8> {
    // Pass through upstream headers but force Connection to match client KA.
    let end = parse::find_header_end(header_block).unwrap_or(header_block.len());
    let head = &header_block[..end];
    let mut out = Vec::with_capacity(head.len() + 32);
    for line in head.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if line.len() >= 11 && line[..11].eq_ignore_ascii_case(b"connection:") {
            continue;
        }
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    if client_ka {
        out.extend_from_slice(b"Connection: keep-alive\r\n\r\n");
    } else {
        out.extend_from_slice(b"Connection: close\r\n\r\n");
    }
    out
}

fn close_session(
    sessions: &mut HashMap<Token, Session>,
    up_to_client: &mut HashMap<Token, Token>,
    idle_upstream: &mut VecDeque<(SocketAddr, TcpStream)>,
    poll: &mut Poll,
    client_tok: Token,
) {
    let Some(mut sess) = sessions.remove(&client_tok) else {
        return;
    };
    let _ = poll.registry().deregister(&mut sess.client);
    if let (Some(mut up), Some(ut)) = (sess.upstream.take(), sess.upstream_token.take()) {
        let _ = poll.registry().deregister(&mut up);
        up_to_client.remove(&ut);
        let _ = up;
    }
    let _ = idle_upstream;
}

fn bind_reuseport(addr: SocketAddr) -> io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
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
    Ok(TcpListener::from_std(socket.into()))
}

fn mio_to_std(stream: TcpStream) -> std::net::TcpStream {
    std::net::TcpStream::from(stream)
}

#[cfg(test)]
mod sdp001_waf_theater_tests {
    use super::{evaluate_waf_admission, WafAdmission};
    use exyonq_waf_api::{
        BodyInspectionState, WafAction, WafDecision, WafEngine, WafEvidence, WafPhase, WafRequest,
        WafRuleId, WafViolation,
    };
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Mutex;

    struct AlwaysBlockWaf;
    impl WafEngine for AlwaysBlockWaf {
        fn inspect(&self, _phase: WafPhase, _req: &WafRequest<'_>) -> WafDecision {
            WafDecision {
                action: WafAction::Block,
                violations: vec![WafViolation {
                    rule_id: WafRuleId::new("SDP-001-STUB"),
                    phase: WafPhase::RequestHeaders,
                    message: "stub block".into(),
                    evidence: WafEvidence {
                        field: None,
                        matched: None,
                        transforms: Vec::new(),
                    },
                }],
                inspection_truncated: false,
                retry_after_secs: None,
            }
        }
        fn inspect_body_chunk(
            &self,
            _req: &WafRequest<'_>,
            _chunk: &[u8],
            _end_of_stream: bool,
            _state: &mut BodyInspectionState,
        ) -> WafDecision {
            WafDecision::allow()
        }
    }

    /// SDP-001: `waf_wire_inspection_active=false` no debe Allow silencioso si el WAF bloquearía.
    #[test]
    fn inactive_wire_inspection_must_not_unconditionally_allow_when_waf_would_block() {
        let admission = evaluate_waf_admission(
            false,
            true,
            &AlwaysBlockWaf,
            "/api/../secret",
            Some("evil.example"),
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, 7)),
        );
        assert!(
            matches!(admission, WafAdmission::Reject { .. }),
            "SDP-001: waf_wire_inspection_active=false no debe cortocircuitar a Allow cuando el WAF bloquea"
        );
    }

    /// SDP-001: client_ip forzado a 0.0.0.0 — el engine no ve el peer real.
    #[test]
    fn evaluate_waf_passes_unspecified_client_ip_not_peer() {
        struct CaptureIpWaf {
            seen: Mutex<Option<IpAddr>>,
        }
        impl WafEngine for CaptureIpWaf {
            fn inspect(&self, _phase: WafPhase, req: &WafRequest<'_>) -> WafDecision {
                *self.seen.lock().unwrap() = Some(req.client_ip);
                WafDecision::allow()
            }
            fn inspect_body_chunk(
                &self,
                _req: &WafRequest<'_>,
                _chunk: &[u8],
                _end_of_stream: bool,
                _state: &mut BodyInspectionState,
            ) -> WafDecision {
                WafDecision::allow()
            }
        }
        let waf = CaptureIpWaf {
            seen: Mutex::new(None),
        };
        let peer = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7));
        let _ = evaluate_waf_admission(true, true, &waf, "/", Some("h"), peer);
        let seen = *waf.seen.lock().unwrap();
        assert_eq!(
            seen,
            Some(peer),
            "SDP-001: evaluate_waf debe pasar el peer real, no 0.0.0.0"
        );
        assert_ne!(
            seen,
            Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            "SDP-001: evaluate_waf no debe fijar client_ip=0.0.0.0 (peer real requerido)"
        );
    }
}
