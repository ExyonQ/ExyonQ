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
//! Shared HTTP/1.1 wire fast-path dispatch (plain TCP + TLS).
//!
//! **Wire static (PR-6d3b):** `might_use_static_wire` + `site_static` is a permanent
//! byte-prefix fast path — not a legacy debt path. Runtime does **not** call
//! `RouteIndex` or `BackendTable` here; semantic alignment to `Backend::Static` for route
//! `"site"` is enforced at compile time (PR-6d3a invariants) and in offline tests below.
#![cfg_attr(target_os = "linux", allow(dead_code))]

use crate::kernel::generation::GenerationView;
use crate::kernel::wire::{plan_wire_decision, WirePlanDecision};
use crate::lifecycle::{self};
use crate::observability::{self, AccessTerminal};
use crate::server::drain_probes;
use crate::server::io::{self, PrefixedStream};
use crate::server::state::ServerState;
use crate::Backend;
use bytes::Bytes;
use exyonq_mod_proxy::ProxyClient;
use exyonq_module_api::proxy_wire;
#[cfg(target_os = "linux")]
use exyonq_module_api::static_wire;
use hyper::header::HeaderValue;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, warn};

const BAD_REQUEST_RESPONSE: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";

/// Process-local count of failed WirePlan::Reject 400 response writes.
static WIRE_REJECT_RESPONSE_WRITE_ERRORS: AtomicU64 = AtomicU64::new(0);

/// Process-local count of failed WAF reject response writes on the wire path.
static WIRE_WAF_REJECT_RESPONSE_WRITE_ERRORS: AtomicU64 = AtomicU64::new(0);

/// Snapshot of failed reject-response writes (test / ops observation).
pub fn wire_reject_response_write_errors() -> u64 {
    WIRE_REJECT_RESPONSE_WRITE_ERRORS.load(Ordering::Relaxed)
}

/// Snapshot of failed WAF-reject response writes (test / ops observation).
pub fn wire_waf_reject_response_write_errors() -> u64 {
    WIRE_WAF_REJECT_RESPONSE_WRITE_ERRORS.load(Ordering::Relaxed)
}

/// Write the wire Reject 400 response.
///
/// `Ok(())` means the response bytes were handed to the socket write path successfully
/// (delivery of intended 400). `Err` means the write failed — must not be recorded as a
/// successful `bad_request` delivery.
async fn write_wire_reject_bad_request<S>(stream: &mut S) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(BAD_REQUEST_RESPONSE).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

/// Access / observation after WirePlan::Reject.
///
/// - `write_ok == true`: `status=400` + `outcome=bad_request` means the 400 response write
///   completed (intended status was emitted onto the socket).
/// - `write_ok == false`: `status=400` is **INTENDED_RESPONSE_STATUS only**;
///   `outcome=response_write_failed` means the 400 was **not** successfully written.
///   Do not interpret this event as DELIVERED_400_SUCCESS.
fn emit_wire_reject_access(started: Instant, client_ip: Option<&str>, write_ok: bool) {
    if !observability::access_event_required() {
        return;
    }
    let internal = observability::generate_internal_request_id();
    let (outcome, error_class) = if write_ok {
        ("bad_request", Some("http_parse"))
    } else {
        ("response_write_failed", Some("response_write_failed"))
    };
    observability::emit_access_terminal(AccessTerminal {
        method: "?",
        path: "-",
        status: 400,
        protocol: "http1.1-wire",
        duration_ms: observability::duration_ms(started),
        bytes_sent: None,
        client_ip,
        external_request_id: None,
        internal_request_id: &internal,
        upstream: None,
        error_class,
        outcome: Some(outcome),
    });
}

async fn complete_wire_reject<S>(stream: &mut S, started: Instant, client_ip: Option<&str>)
where
    S: AsyncWrite + Unpin,
{
    match write_wire_reject_bad_request(stream).await {
        Ok(()) => {
            emit_wire_reject_access(started, client_ip, true);
        }
        Err(err) => {
            WIRE_REJECT_RESPONSE_WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
            warn!(%err, "wire reject 400 response write failed");
            emit_wire_reject_access(started, client_ip, false);
        }
    }
}

pub(crate) fn emit_wire_access(
    head: &[u8],
    status: u16,
    outcome: &str,
    client_ip: Option<&str>,
    duration_ms: u64,
) -> Option<String> {
    // Cap061/S2: no identity/urandom/formatting when no access consumer is active.
    if !observability::access_event_required() {
        return None;
    }
    let path_q = parse_wire_get_path(head).unwrap_or("-");
    let path = path_q.split('?').next().unwrap_or(path_q);
    let pairs = parse_wire_header_pairs(head);
    let raw_xid = pairs
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("x-request-id"))
        .map(|(_, v)| v.as_str());
    let identity = observability::resolve_request_identity(raw_xid);
    observability::emit_access_terminal(AccessTerminal {
        method: "GET",
        path,
        status,
        protocol: "http1.1-wire",
        duration_ms,
        bytes_sent: None,
        client_ip,
        external_request_id: identity.external_request_id.as_deref(),
        internal_request_id: &identity.internal_request_id,
        upstream: None,
        error_class: None,
        outcome: Some(outcome),
    });
    Some(identity.internal_request_id)
}

/// Cap061: bridge static-wire terminal statuses from module-api into access events.
pub fn install_static_wire_access_bridge() {
    let _ = exyonq_module_api::static_wire::install_access_notice_hook(|notice| {
        emit_wire_access(
            notice.head.as_ref(),
            notice.status,
            notice.outcome,
            None,
            notice.duration_ms,
        )
    });
}

/// P15-WS5-PROBE-002: on drain reject, classify probes from request head when available.
async fn write_drain_boundary_response<S>(stream: &mut S, head: Option<&[u8]>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let owned;
    let bytes = if let Some(h) = head {
        h
    } else {
        let mut buf = [0u8; 1024];
        owned = match tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf)).await
        {
            Ok(Ok(n)) if n > 0 => buf[..n].to_vec(),
            _ => Vec::new(),
        };
        owned.as_slice()
    };
    let resp = drain_probes::drain_boundary_response(bytes);
    let _ = stream.write_all(resp).await;
    let _ = stream.shutdown().await;
}

/// Linux epoll/sync accept predicate; Tokio wire path uses `plan_wire_decision` instead.
#[cfg(target_os = "linux")]
pub(crate) fn might_use_static_wire(head: &[u8]) -> bool {
    static_wire::might_use_static_wire(head)
}

/// Cap034 host ranking + Cap056 cache-policy gate inputs.
fn resolve_proxy_route(
    state: &ServerState,
    path: &str,
    host: Option<&str>,
) -> Option<(usize, u32)> {
    // Cap034: wire must use the same Host/:authority ranking as Hyper. Passing `None`
    // preferred hostless routes over named hosts when both matched the path.
    let (route_idx, _) = state.route_index.match_route_index_with_host(path, host)?;
    let Backend::Proxy { cluster_id } = state.snapshot.resolve_backend(route_idx)? else {
        return None;
    };
    Some((route_idx, *cluster_id))
}

/// Cap033/Cap034: strip `:port` without destroying IPv6 literal authorities (`[::1]:8080`).
fn strip_host_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return &host[..=end];
        }
        return host;
    }
    match host.rsplit_once(':') {
        Some((name, port)) if !name.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    }
}

fn head_matches_htaccess_route(state: &ServerState, head: &[u8]) -> bool {
    let Some(path) = parse_wire_origin_path(head) else {
        return false;
    };
    let Some((route_idx, _)) = state
        .route_index
        .match_route_index_with_host(path, None)
    else {
        return false;
    };
    state
        .snapshot
        .htaccess_site_id_for_route(route_idx)
        .is_some()
}

fn parse_wire_origin_path(head: &[u8]) -> Option<&str> {
    let rest = if head.starts_with(b"GET ") {
        &head[4..]
    } else if head.starts_with(b"HEAD ") {
        &head[5..]
    } else {
        return None;
    };
    let end = rest.iter().position(|&b| b == b' ')?;
    let target = std::str::from_utf8(&rest[..end]).ok()?;
    Some(target.split('?').next().unwrap_or(target))
}

fn parse_wire_get_path(head: &[u8]) -> Option<&str> {
    if !head.starts_with(b"GET ") {
        return None;
    }
    let rest = &head[4..];
    let end = rest.iter().position(|&b| b == b' ')?;
    std::str::from_utf8(&rest[..end]).ok()
}

/// Request-target for WAF: lossy UTF-8 so invalid bytes cannot skip wire WAF (P1-H).
fn parse_wire_request_target_lossy(head: &[u8]) -> Option<String> {
    let rest = if head.starts_with(b"GET ") {
        &head[4..]
    } else if head.starts_with(b"HEAD ") {
        &head[5..]
    } else {
        return None;
    };
    let end = rest.iter().position(|&b| b == b' ')?;
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
}

fn parse_wire_header_pairs(head: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // Line-oriented: a single non-UTF8 header must not erase the entire header view (P2-F).
    let mut lines = head.split(|&b| b == b'\n');
    let _ = lines.next(); // request line
    for line in lines {
        let line = match line.strip_suffix(b"\r") {
            Some(l) => l,
            None => line,
        };
        if line.is_empty() {
            break;
        }
        // P2-G: lossy decode so obs-text / invalid bytes cannot erase a header from WAF.
        let text = String::from_utf8_lossy(line);
        let Some((name, value)) = text.split_once(':') else {
            continue;
        };
        out.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    out
}

fn parse_wire_method(head: &[u8]) -> &'static str {
    if head.starts_with(b"HEAD ") {
        "HEAD"
    } else {
        "GET"
    }
}

/// ARCH-002: whether this generation requires WAF wire-header materialization.
///
/// Uses the generation-frozen binding flag (composition-resolved mode, including
/// `EXYONQ_WAF_MODE`), not IR `mode=Disabled` alone. When inactive, Cap015 still
/// demands a per-request Cap067 entry point — but must not parse/copy headers
/// solely for Nop/Disabled engine plumbing. Monitor and Block remain fully active.
#[inline]
pub(crate) fn waf_wire_materialization_active(state: &ServerState) -> bool {
    state.waf_wire_inspection_active
}

/// Cap015: evaluate generation-frozen WAF on wire heads before static/proxy serve.
pub(crate) fn evaluate_wire_waf(
    state: &ServerState,
    generation: u64,
    head: &[u8],
    x_forwarded_for: &HeaderValue,
) -> crate::waf::WafHookResult {
    // ARCH-002: skip parse_wire_header_pairs / owned Strings / engine inspect when WAF is off.
    if !waf_wire_materialization_active(state) {
        return crate::waf::WafHookResult::Continue;
    }
    // P1-H: never Continue solely because the request-target fails strict UTF-8.
    // Lossy decode keeps query/path signatures visible; missing target still inspects headers.
    let owned = parse_wire_request_target_lossy(head).unwrap_or_default();
    let (path, query) = match owned.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (owned.as_str(), None),
    };
    let header_pairs = parse_wire_header_pairs(head);
    let header_view = crate::waf::PairsHeaderView(&header_pairs);
    // Same host identity as Hyper `request_host` (port stripped). The challenge
    // MAC and grant cookie bind this value; a Cap067 issue with `host:port`
    // cannot be redeemed by the Hyper POST to `/.exyonq/waf-challenge`.
    let host = wire_host_from_pairs(&header_pairs).map(strip_host_port);
    let client_ip = crate::waf::parse_client_ip(x_forwarded_for.to_str().ok());
    crate::waf::inspect_request_headers(
        state.waf.as_ref(),
        parse_wire_method(head),
        host,
        path,
        query,
        &header_view,
        client_ip,
        None,
        generation,
        false,
        state.waf_enforce,
        state.waf_abuse.as_deref(),
    )
}

fn wire_host_from_pairs(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(n, _)| n == "host")
        .map(|(_, v)| v.as_str())
}

pub(crate) fn format_waf_reject_http(reject: &crate::waf::WafReject) -> Vec<u8> {
    let body = reject.body.as_slice();
    let mut resp = format!(
        "HTTP/1.1 {} \r\ncontent-type: {}\r\ncache-control: no-store\r\npragma: no-cache\r\ncontent-length: {}\r\nconnection: close\r\n",
        reject.status.as_u16(),
        reject.content_type,
        body.len()
    );
    if let Some(secs) = reject.retry_after_secs {
        resp.push_str(&format!("retry-after: {secs}\r\n"));
    }
    if let Some(cookie) = reject.set_cookie.as_deref() {
        resp.push_str(&format!("set-cookie: {cookie}\r\n"));
    }
    if let Some(loc) = reject.location.as_deref() {
        resp.push_str(&format!("location: {loc}\r\n"));
    }
    resp.push_str("\r\n");
    let mut out = resp.into_bytes();
    out.extend_from_slice(body);
    out
}

/// Write a WAF reject response on the HTTP/1.1 wire path.
///
/// `Ok(())` means the reject bytes were handed to the socket write path successfully.
/// `Err` means the write failed — callers must not record `waf_block` /
/// `waf_rate_limited` / `waf_reject` as a delivered response.
async fn write_waf_reject_wire<S>(
    stream: &mut S,
    reject: &crate::waf::WafReject,
) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let bytes = format_waf_reject_http(reject);
    stream.write_all(&bytes).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

fn waf_reject_access_outcome(reject: &crate::waf::WafReject) -> &'static str {
    match reject.status.as_u16() {
        429 => "waf_rate_limited",
        403 => "waf_block",
        _ => "waf_reject",
    }
}

/// Complete a wire WAF reject: write response, then emit access with delivery truth.
///
/// - write Ok → `outcome=waf_*` means the reject was written onto the socket.
/// - write Err → `outcome=response_write_failed` (status is INTENDED only).
async fn complete_waf_reject_wire<S>(
    stream: &mut S,
    reject: &crate::waf::WafReject,
    head: &[u8],
    client_ip: Option<&str>,
    started: Instant,
) where
    S: AsyncWrite + Unpin,
{
    match write_waf_reject_wire(stream, reject).await {
        Ok(()) => {
            exyonq_module_api::wire_record_response(reject.status.as_u16());
            emit_wire_access(
                head,
                reject.status.as_u16(),
                waf_reject_access_outcome(reject),
                client_ip,
                observability::duration_ms(started),
            );
        }
        Err(err) => {
            WIRE_WAF_REJECT_RESPONSE_WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
            warn!(%err, "wire waf reject response write failed");
            emit_wire_access(
                head,
                reject.status.as_u16(),
                "response_write_failed",
                client_ip,
                observability::duration_ms(started),
            );
        }
    }
}

enum ProxyWireOutcome<S> {
    Handled,
    Fallback(S, Bytes, Bytes),
}

/// Wire proxy fast path; returns `Fallback` when Hyper must complete the request.
async fn serve_proxy_wire<S>(
    state: &ServerState,
    generation: u64,
    mut stream: S,
    head: Bytes,
    rest: Bytes,
    x_forwarded_for: HeaderValue,
) -> ProxyWireOutcome<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Defense: WebSocket / ineligible heads must complete via Hyper, not GET wire.
    if !proxy_wire::might_use_proxy_wire(head.as_ref()) {
        return ProxyWireOutcome::Fallback(stream, head, rest);
    }
    let Some(path_and_query) = parse_wire_get_path(head.as_ref()) else {
        return ProxyWireOutcome::Fallback(stream, head, rest);
    };
    let path = path_and_query.split('?').next().unwrap_or(path_and_query);
    let header_pairs = parse_wire_header_pairs(head.as_ref());
    let host_owned = wire_host_from_pairs(&header_pairs).map(strip_host_port);
    let Some((route_idx, cluster_id)) = resolve_proxy_route(state, path, host_owned) else {
        return ProxyWireOutcome::Fallback(stream, head, rest);
    };

    // Cap056: explicit `route.cache` / [[cache_policy]] is owned by Hyper
    // `proxy_dispatch_maybe_cached`. The GET `/api/*` wire fast path must not
    // silently bypass store/hit when a policy is bound (PRODUCTION_REACHABLE defect).
    if state.snapshot.cache_policy_for_route(route_idx).is_some() {
        return ProxyWireOutcome::Fallback(stream, head, rest);
    }

    // Wire-cheap Cap055: admit before WAF / upstream (same process limiter as Hyper).
    let client_ip = x_forwarded_for.to_str().unwrap_or("127.0.0.1");
    match exyonq_module_api::wire_admit(client_ip) {
        exyonq_module_api::WireAdmit::Allow => {}
        exyonq_module_api::WireAdmit::Reject429 { retry_after_secs } => {
            let started = Instant::now();
            let reject = exyonq_module_api::rate_limit_reject_wire(retry_after_secs);
            match tokio::io::AsyncWriteExt::write_all(&mut stream, &reject).await {
                Ok(()) => exyonq_module_api::wire_record_response(429),
                Err(err) => debug!(%err, "wire rate-limit 429 write failed"),
            }
            emit_wire_access(
                head.as_ref(),
                429,
                "wire_rate_limited",
                x_forwarded_for.to_str().ok(),
                observability::duration_ms(started),
            );
            return ProxyWireOutcome::Handled;
        }
    }

    // Cap015: proxy wire must not bypass signature/abuse WAF (same contract as Hyper path).
    match evaluate_wire_waf(state, generation, head.as_ref(), &x_forwarded_for) {
        crate::waf::WafHookResult::Reject(reject) => {
            let started = Instant::now();
            complete_waf_reject_wire(
                &mut stream,
                &reject,
                head.as_ref(),
                x_forwarded_for.to_str().ok(),
                started,
            )
            .await;
            return ProxyWireOutcome::Handled;
        }
        crate::waf::WafHookResult::Continue => {}
    }

    let started = Instant::now();
    let xff = x_forwarded_for.to_str().unwrap_or("127.0.0.1").to_string();
    let boxed = exyonq_mod_proxy::box_wire_stream(stream);
    // Bench/edge hot path: access logging is usually off — do not Bytes::clone the
    // request head solely for a no-op emit (refcount churn on every P4 wire GET).
    let head_for_access = observability::access_event_required().then(|| head.clone());
    match proxy_wire::serve_proxy_wire(cluster_id, generation, xff, boxed, head, rest).await {
        Ok(status) => {
            if let Some(ref h) = head_for_access {
                emit_wire_access(
                    h.as_ref(),
                    status,
                    "proxy_wire",
                    x_forwarded_for.to_str().ok(),
                    observability::duration_ms(started),
                );
            }
        }
        Err(err) => {
            debug!(%err, "proxy wire serve ended");
            if let Some(ref h) = head_for_access {
                emit_wire_access(
                    h.as_ref(),
                    502,
                    "proxy_wire_error",
                    x_forwarded_for.to_str().ok(),
                    observability::duration_ms(started),
                );
            }
        }
    }
    ProxyWireOutcome::Handled
}

#[cfg(target_os = "linux")]
pub(crate) fn epoll_keep_alive_eligible(head: &[u8]) -> bool {
    static_wire::epoll_keep_alive_eligible(head)
}

#[cfg(target_os = "linux")]
#[cfg(target_os = "linux")]
pub(crate) fn epoll_sendfile_eligible(head: &[u8]) -> bool {
    static_wire::epoll_sendfile_eligible(head)
}

/// Routes that must use Tokio accept/dispatch (proxy, modules, unknown).
/// Cap061 LA-008: no filename-based force for former bench assets — they are
/// ordinary Hyper/static traffic via `!might_use_static_wire`.
#[cfg(target_os = "linux")]
pub(crate) fn tokio_accept_required(head: &[u8]) -> bool {
    head.starts_with(b"GET /api/") || !might_use_static_wire(head)
}

pub(crate) struct WireDispatchContext {
    /// Live generation swap handle — Hyper refreshes `state` from this per request.
    pub shared: crate::reload::SharedServerState,
    pub state: Arc<ServerState>,
    pub proxy_client: ProxyClient,
    pub x_forwarded_for: HeaderValue,
    pub ops: Arc<crate::lifecycle::LifecycleState>,
    /// Pinned at handoff/admit time (Phase 0); proxy wire uses this instead of re-reading.
    pub pinned_generation: u64,
}

enum WirePlan<S> {
    Proxy(S, Bytes, Bytes),
    Static(S, Bytes, Bytes),
    Hyper(S, Bytes, Bytes),
    /// Ambiguous/malformed framing: emit 400 and close — never route.
    Reject(S),
}

async fn read_wire_plan<S>(mut stream: S, state: &ServerState) -> Option<WirePlan<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Ok((head, rest)) = io::read_until_headers_async(&mut stream).await else {
        return None;
    };
    plan_wire_after_headers(stream, state, head, rest)
}

fn generation_view_from_state(state: &ServerState) -> GenerationView {
    GenerationView {
        generation: state.generation,
        modules_enabled: state.modules_enabled(),
        site_static_slot: state.site_static_slot,
    }
}

fn plan_wire_after_headers<S>(
    stream: S,
    state: &ServerState,
    head: Bytes,
    rest: Bytes,
) -> Option<WirePlan<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Fail closed: never route/static/proxy on ambiguous HTTP/1 framing.
    if !io::request_headers_safe_for_wire(head.as_ref()) {
        return Some(WirePlan::Reject(stream));
    }
    // Cap067 early AE gate: compression configured ∧ AE prefers coding → Hyper
    // (no mid-connection Cap067→Hyper handoff later on this socket).
    if state.modules_require_hyper_for_request_head(head.as_ref()) {
        return Some(WirePlan::Hyper(stream, head, rest));
    }
    // A static miss on the epoll path is a terminal 404. `.htaccess` Redirect
    // is applied only by the full handler, so an overlay route must not be
    // claimed as sendfile.
    if head_matches_htaccess_route(state, head.as_ref()) {
        return Some(WirePlan::Hyper(stream, head, rest));
    }
    let view = generation_view_from_state(state);
    let decision = plan_wire_decision(&view, head.as_ref()).ok()?;
    // A path with no static slot is not sendfile. Planning Static would
    // re-enter Cap067 after a Hyper handoff and hide proxy / metrics routes.
    #[cfg(target_os = "linux")]
    let decision = if decision == WirePlanDecision::Static
        && !head.starts_with(b"GET /health ")
        && !head.starts_with(b"HEAD /health ")
        && !exyonq_mod_static::static_slot_owns_request(head.as_ref())
    {
        WirePlanDecision::Hyper
    } else {
        decision
    };
    Some(match decision {
        WirePlanDecision::Proxy => WirePlan::Proxy(stream, head, rest),
        WirePlanDecision::Static => WirePlan::Static(stream, head, rest),
        WirePlanDecision::Hyper => WirePlan::Hyper(stream, head, rest),
    })
}

async fn execute_wire_plan(plan: WirePlan<TcpStream>, ctx: WireDispatchContext) {
    match plan {
        WirePlan::Proxy(stream, head, rest) => {
            #[cfg(target_os = "linux")]
            let peer_fd = Some(std::os::fd::AsRawFd::as_raw_fd(&stream));
            #[cfg(target_os = "linux")]
            let outcome = exyonq_mod_proxy::with_proxy_peer_fd(peer_fd, async {
                serve_proxy_wire(
                    &ctx.state,
                    ctx.pinned_generation,
                    stream,
                    head,
                    rest,
                    ctx.x_forwarded_for.clone(),
                )
                .await
            })
            .await;
            #[cfg(not(target_os = "linux"))]
            let outcome = serve_proxy_wire(
                &ctx.state,
                ctx.pinned_generation,
                stream,
                head,
                rest,
                ctx.x_forwarded_for.clone(),
            )
            .await;
            match outcome {
                ProxyWireOutcome::Handled => {}
                ProxyWireOutcome::Fallback(stream, head, rest) => {
                    serve_hyper_fallback(stream, head, rest, ctx).await;
                }
            }
        }
        WirePlan::Static(stream, head, rest) => {
            if let Some(site_slot) = ctx.state.site_static_slot {
                // Final admission/WAF ownership belongs to the selected terminal owner:
                // Cap067 handles admitted sendfile requests per request; Hyper handles
                // registration fallback. This avoids pre-admitting a fallback twice.
                #[cfg(target_os = "linux")]
                {
                    // Cap067: eligible cleartext GET/HEAD → epoll sendfile FSM.
                    // Register failure or missing pool → canonical Hyper (NOT_STARTED).
                    // CAP067_MISS_NEEDHYPER_HANG: Cap067 completes Cap004/missing
                    // terminals in-epoll; NeedHyper is decline-only (no Static re-loop).
                    if epoll_sendfile_eligible(head.as_ref()) {
                        if static_wire::keepalive_handoff_enabled() {
                            match static_wire::register_sendfile_from_tokio_first(
                                stream,
                                head.as_ref(),
                                rest.as_ref(),
                            ) {
                                Ok(()) => {}
                                Err(restored) => {
                                    static_wire::note_sendfile_fallback();
                                    serve_hyper_fallback(restored, head, rest, ctx).await;
                                }
                            }
                        } else {
                            static_wire::note_sendfile_fallback();
                            serve_hyper_fallback(stream, head, rest, ctx).await;
                        }
                    } else if head.starts_with(b"GET /health ")
                        || head.starts_with(b"HEAD /health ")
                    {
                        // Cap061 LA-CAP061-001: do NOT hardcode 200 here.
                        // `/health` is the only exempt inline wire owner.
                        let _ = serve_static_tcp(site_slot, stream, head, rest).await;
                    } else {
                        // Cap067 declined terminal ownership before admission. Hyper now
                        // owns admission, WAF, static dispatch, metrics, and any fallback.
                        static_wire::note_sendfile_fallback();
                        serve_hyper_fallback(stream, head, rest, ctx).await;
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = site_slot;
                    serve_hyper_fallback(stream, head, rest, ctx).await;
                }
            }
        }
        WirePlan::Hyper(stream, head, rest) => {
            serve_hyper_fallback(stream, head, rest, ctx).await;
        }
        WirePlan::Reject(mut stream) => {
            let started = Instant::now();
            complete_wire_reject(&mut stream, started, ctx.x_forwarded_for.to_str().ok()).await;
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) async fn dispatch_tcp_prefixed(
    mut stream: TcpStream,
    head: Bytes,
    rest: Bytes,
    ctx: WireDispatchContext,
) {
    let Ok(token) = ctx.ops.try_enter() else {
        write_drain_boundary_response(&mut stream, Some(head.as_ref())).await;
        return;
    };
    lifecycle::run_with_connection_token(token, async move {
        dispatch_tcp_prefixed_inner(stream, head, rest, ctx).await;
    })
    .await;
}

#[cfg(target_os = "linux")]
pub(crate) async fn dispatch_tcp_prefixed_inner(
    stream: TcpStream,
    head: Bytes,
    rest: Bytes,
    ctx: WireDispatchContext,
) {
    if ctx
        .state
        .modules_require_hyper_for_request_head(head.as_ref())
        || ctx.state.hyper_required_for_modules()
        || ctx.ops.is_draining()
    {
        super::serve_hyper(
            PrefixedStream::new(io::concat_bytes(head, rest), stream),
            crate::reload::SharedServerState::clone(&ctx.shared),
            ctx.proxy_client,
            ctx.x_forwarded_for.clone(),
            ctx.ops,
        )
        .await;
        return;
    }

    let Some(plan) = plan_wire_after_headers(stream, &ctx.state, head, rest) else {
        return;
    };
    execute_wire_plan(plan, ctx).await;
}

#[cfg(target_os = "linux")]
async fn serve_static_tcp(
    site_slot: u32,
    stream: TcpStream,
    head: Bytes,
    rest: Bytes,
) -> std::io::Result<()> {
    if !epoll_sendfile_eligible(head.as_ref())
        && static_wire::static_tcp_blocking_pool_admission(head.as_ref())
    {
        let std_stream = stream.into_std().map_err(std::io::Error::other)?;
        match static_wire::try_spawn_blocking_static(site_slot, std_stream, head.clone(), rest) {
            static_wire::BlockingAdmission::Accepted => return Ok(()),
            static_wire::BlockingAdmission::Rejected(std_stream) => {
                let started = Instant::now();
                match static_wire::shed_blocking_admission(std_stream).await {
                    Ok(()) => {
                        emit_wire_access(
                            head.as_ref(),
                            503,
                            "static_blocking_shed",
                            None,
                            observability::duration_ms(started),
                        );
                    }
                    Err(_err) => {
                        // status=503 is INTENDED_RESPONSE_STATUS only; outcome marks
                        // non-delivery (LET-180 / SUB-LET-STATIC-ADMISSION-REJECT-WRITE).
                        emit_wire_access(
                            head.as_ref(),
                            503,
                            "response_write_failed",
                            None,
                            observability::duration_ms(started),
                        );
                    }
                }
                return Ok(());
            }
        }
    }
    static_wire::serve_wire_site_tcp(site_slot, stream, head, rest).await
}

async fn serve_hyper_fallback<S>(stream: S, head: Bytes, rest: Bytes, ctx: WireDispatchContext)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let prefixed = PrefixedStream::new(io::concat_bytes(head, rest), stream);
    super::serve_hyper(
        prefixed,
        crate::reload::SharedServerState::clone(&ctx.shared),
        ctx.proxy_client,
        ctx.x_forwarded_for,
        ctx.ops,
    )
    .await;
}

pub(crate) async fn dispatch_tcp(mut stream: TcpStream, ctx: WireDispatchContext) {
    let Ok(token) = ctx.ops.try_enter() else {
        write_drain_boundary_response(&mut stream, None).await;
        return;
    };
    lifecycle::run_with_connection_token(token, async move {
        dispatch_tcp_inner(stream, ctx).await;
    })
    .await;
}

pub(crate) async fn dispatch_tcp_inner(stream: TcpStream, ctx: WireDispatchContext) {
    if ctx.state.hyper_required_for_modules() || ctx.ops.is_draining() {
        super::serve_hyper(
            stream,
            crate::reload::SharedServerState::clone(&ctx.shared),
            ctx.proxy_client,
            ctx.x_forwarded_for.clone(),
            ctx.ops,
        )
        .await;
        return;
    }

    let Some(plan) = read_wire_plan(stream, &ctx.state).await else {
        return;
    };
    execute_wire_plan(plan, ctx).await;
}

pub(crate) async fn dispatch_stream<S>(stream: S, ctx: WireDispatchContext)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut stream = stream;
    let Ok(token) = ctx.ops.try_enter() else {
        write_drain_boundary_response(&mut stream, None).await;
        return;
    };
    lifecycle::run_with_connection_token(token, async move {
        dispatch_stream_inner(stream, ctx).await;
    })
    .await;
}

pub(crate) async fn dispatch_stream_inner<S>(stream: S, ctx: WireDispatchContext)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    if ctx.state.hyper_required_for_modules() || ctx.ops.is_draining() {
        super::serve_hyper(
            stream,
            crate::reload::SharedServerState::clone(&ctx.shared),
            ctx.proxy_client,
            ctx.x_forwarded_for.clone(),
            ctx.ops,
        )
        .await;
        return;
    }

    let Some(plan) = read_wire_plan(stream, &ctx.state).await else {
        return;
    };

    match plan {
        WirePlan::Proxy(stream, head, rest) => {
            match serve_proxy_wire(
                &ctx.state,
                ctx.pinned_generation,
                stream,
                head,
                rest,
                ctx.x_forwarded_for.clone(),
            )
            .await
            {
                ProxyWireOutcome::Handled => {}
                ProxyWireOutcome::Fallback(stream, head, rest) => {
                    serve_hyper_fallback(stream, head, rest, ctx).await;
                }
            }
        }
        WirePlan::Static(stream, head, rest) => {
            serve_hyper_fallback(stream, head, rest, ctx).await;
        }
        WirePlan::Hyper(stream, head, rest) => {
            serve_hyper_fallback(stream, head, rest, ctx).await;
        }
        WirePlan::Reject(mut stream) => {
            let started = Instant::now();
            complete_wire_reject(&mut stream, started, ctx.x_forwarded_for.to_str().ok()).await;
        }
    }
}

#[cfg(test)]
mod waf_wire_parse_tests {
    use super::{parse_wire_get_path, parse_wire_request_target_lossy};

    #[test]
    fn wire_request_target_lossy_keeps_xss_query_with_invalid_utf8() {
        let head = b"GET /site/routes/route001.bin?q=%3Cscript%3E\xff HTTP/1.1\r\nHost: x\r\n\r\n";
        assert!(parse_wire_get_path(head).is_none());
        let target = parse_wire_request_target_lossy(head).expect("target");
        assert!(target.contains("%3Cscript%3E"));
        assert!(target.contains('\u{FFFD}'));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{
        epoll_keep_alive_eligible, epoll_sendfile_eligible, might_use_static_wire,
        plan_wire_after_headers, tokio_accept_required, WirePlan,
    };
    use crate::config::{
        AppConfig, RouteConfig, RouteMatch, ServerConfig, ServerNames, UpstreamConfig,
    };
    use crate::execute_backend;
    use crate::server::state::ServerState;
    use crate::Backend;
    use bytes::Bytes;
    use exyonq_mod_proxy::build_incoming_client;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, Once};

    fn ensure_wire_hooks() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            exyonq_mod_static::install_kernel_hooks(Arc::new(
                exyonq_mod_static::StaticRuntime::new(),
            ));
            let proxy_rt = Arc::new(exyonq_mod_proxy::ProxyRuntime::new());
            exyonq_mod_proxy::install_kernel_hooks(proxy_rt);
        });
    }
    use std::time::Duration;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};

    /// Process-global write-error counter is shared; serialize these two tests.
    static WIRE_REJECT_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Cap061 LA-008: wire static admits `/health` only (not `/metrics`, not bench filenames).
    const WIRE_STATIC_HEADS: &[&[u8]] =
        &[b"GET /health HTTP/1.1\r\n", b"HEAD /health HTTP/1.1\r\n"];

    #[test]
    fn wire_static_predicate_paths_are_health() {
        ensure_wire_hooks();
        for head in WIRE_STATIC_HEADS {
            assert!(
                might_use_static_wire(head),
                "expected wire static predicate: {}",
                String::from_utf8_lossy(head)
            );
            let path = wire_path_prefix(head);
            assert_eq!(
                path, "/health",
                "wire static head must target health: {path}"
            );
        }
        // LA-CAP054-008 / Cap067: `/metrics` must Hyper-handoff (never idle on EPOLL wire).
        assert!(!might_use_static_wire(b"GET /metrics HTTP/1.1\r\n"));
        assert!(!might_use_static_wire(b"GET /metrics\r\n"));
        // Cap067: ordinary GET/HEAD static candidates are wire-admitted (sendfile divert);
        // Cap061: must NOT key off bench filenames — eligibility is product properties.
        #[cfg(target_os = "linux")]
        {
            assert!(might_use_static_wire(b"GET /site/1k.bin HTTP/1.1\r\n"));
            assert!(might_use_static_wire(b"GET /site/64k.bin HTTP/1.1\r\n"));
            assert!(might_use_static_wire(b"GET /site/1m.bin HTTP/1.1\r\n"));
            assert!(might_use_static_wire(
                b"GET /site/routes/route050.bin HTTP/1.1\r\n"
            ));
            assert!(might_use_static_wire(b"GET /site/ok.txt HTTP/1.1\r\n"));
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(!might_use_static_wire(b"GET /site/1k.bin HTTP/1.1\r\n"));
            assert!(!might_use_static_wire(b"GET /site/64k.bin HTTP/1.1\r\n"));
            assert!(!might_use_static_wire(b"GET /site/1m.bin HTTP/1.1\r\n"));
            assert!(!might_use_static_wire(
                b"GET /site/routes/route050.bin HTTP/1.1\r\n"
            ));
            assert!(!might_use_static_wire(b"GET /site/ok.txt HTTP/1.1\r\n"));
        }
    }

    #[test]
    fn wire_static_predicates_exclude_proxy_paths() {
        ensure_wire_hooks();
        assert!(!might_use_static_wire(b"GET /api/health HTTP/1.1\r\n"));
        assert!(!might_use_static_wire(b"GET /api/ HTTP/1.1\r\n"));
        // Cap067: non-API GET is a sendfile wire candidate on Linux; proxy prefix stays excluded.
        #[cfg(target_os = "linux")]
        {
            assert!(might_use_static_wire(b"GET /assets/page.html HTTP/1.1\r\n"));
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(!might_use_static_wire(
                b"GET /assets/page.html HTTP/1.1\r\n"
            ));
        }
    }

    fn bench_site_config(root: Option<std::path::PathBuf>) -> AppConfig {
        let site = RouteConfig {
            name: "site".into(),
            r#match: RouteMatch {
                path: "/site".into(),
                host: None,
            },
            upstream: None,
            root,
            index: Some("index.html".into()),
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        };
        let api = RouteConfig {
            name: "api".into(),
            r#match: RouteMatch {
                path: "/api".into(),
                host: None,
            },
            upstream: Some("backend".into()),
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        };
        let mut upstreams = HashMap::new();
        upstreams.insert(
            "backend".into(),
            UpstreamConfig::legacy("backend", "http://127.0.0.1:9000", 5000),
        );
        AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into(), "api".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![site, api],
            upstreams,
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        }
    }

    async fn state_for(
        config: AppConfig,
    ) -> (Arc<ServerState>, execute_backend::StaticDispatchTestGuard) {
        let runtime = Arc::new(exyonq_mod_static::StaticRuntime::new());
        let service: Arc<dyn exyonq_module_api::static_dispatch::StaticDispatchService> =
            runtime.clone();
        let static_guard = execute_backend::StaticDispatchTestGuard::install(service);
        exyonq_mod_static::install_kernel_hooks(runtime);
        ensure_wire_hooks();
        let proxy_client = build_incoming_client();
        let state = ServerState::new_with_generation(1, config, proxy_client)
            .await
            .expect("server state");
        (state, static_guard)
    }

    /// `/site/*` wire heads must match route `"site"` and resolve to `Backend::Static` offline.
    #[tokio::test]
    async fn wire_static_site_heads_align_with_site_route_backend() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _static_guard) =
            state_for(bench_site_config(Some(dir.path().to_path_buf()))).await;
        assert!(state.site_static_slot.is_some());

        for head in WIRE_STATIC_HEADS {
            if !might_use_static_wire(head) {
                continue;
            }
            let path = wire_path_prefix(head);
            if path == "/health" || path.starts_with("/metrics") {
                continue;
            }
            assert!(
                path.starts_with("/site/"),
                "expected /site bench path, got {path}"
            );
            let (route_idx, route) = state
                .route_index
                .match_route_index_with_host(path, None)
                .unwrap_or_else(|| panic!("RouteIndex miss for wire static path {path}"));
            assert_eq!(route.name, "site");
            let backend = state
                .snapshot
                .resolve_backend(route_idx)
                .unwrap_or_else(|| panic!("no backend for site route idx {route_idx}"));
            assert!(matches!(backend, Backend::Static { .. }));
            let via_table = state
                .snapshot
                .static_slot_for_route_backend(route_idx)
                .expect("static slot for site route");
            assert_eq!(via_table.route_name, "site");
            assert_eq!(state.site_static_slot, Some(0));
        }
    }

    /// Health is wire inline; metrics is Hyper (not a static route match).
    #[tokio::test]
    async fn wire_static_health_is_service_path_metrics_is_hyper() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _static_guard) =
            state_for(bench_site_config(Some(dir.path().to_path_buf()))).await;

        assert!(might_use_static_wire(b"GET /health HTTP/1.1\r\n"));
        assert!(!might_use_static_wire(b"GET /metrics HTTP/1.1\r\n"));
        for head in [
            b"GET /health HTTP/1.1\r\n".as_slice(),
            b"GET /metrics HTTP/1.1\r\n".as_slice(),
        ] {
            let path = wire_path_prefix(head);
            assert!(
                state
                    .route_index
                    .match_route_index_with_host(path, None)
                    .is_none(),
                "service path {path} must not match IR site/api routes"
            );
        }
    }

    struct PlanStream;

    impl AsyncRead for PlanStream {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for PlanStream {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<Result<usize, std::io::Error>> {
            std::task::Poll::Ready(Ok(0))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn wire_static_plan_includes_sendfile_candidates_on_linux() {
        let dir = tempfile::tempdir().unwrap();
        let (with_site, _static_guard) =
            state_for(bench_site_config(Some(dir.path().to_path_buf()))).await;
        let health = Bytes::from_static(b"GET /health HTTP/1.1\r\n");
        let plan =
            plan_wire_after_headers(PlanStream, &with_site, health, Bytes::new()).expect("plan");
        assert!(matches!(plan, WirePlan::Static(..)));

        let site_asset = Bytes::from_static(b"GET /site/1k.bin HTTP/1.1\r\n");
        let plan =
            plan_wire_after_headers(PlanStream, &with_site, site_asset.clone(), Bytes::new())
                .expect("plan");
        #[cfg(target_os = "linux")]
        assert!(
            matches!(plan, WirePlan::Static(..)),
            "Cap067: site GET must plan Static for sendfile divert"
        );
        #[cfg(not(target_os = "linux"))]
        assert!(matches!(plan, WirePlan::Hyper(..)));

        let (api_only, _static_guard2) = state_for(bench_site_config(None)).await;
        assert!(api_only.site_static_slot.is_none());
        let plan =
            plan_wire_after_headers(PlanStream, &api_only, site_asset, Bytes::new()).expect("plan");
        assert!(matches!(plan, WirePlan::Hyper(..)));
    }

    fn wire_path_prefix(head: &[u8]) -> &str {
        let s = std::str::from_utf8(head).expect("ascii head");
        let after_method = s.split_once(' ').map(|(_, rest)| rest).unwrap_or(s);
        after_method
            .split_once(' ')
            .map(|(path, _)| path)
            .unwrap_or(after_method)
    }

    #[test]
    fn blocking_pool_facade_matches_module_hooks() {
        ensure_wire_hooks();
        let heads = [
            b"GET /site/1k.bin HTTP/1.1\r\n".as_slice(),
            b"HEAD /site/1k.bin HTTP/1.1\r\n",
            b"GET /site/64k.bin HTTP/1.1\r\n",
            b"GET /site/1m.bin HTTP/1.1\r\n",
            b"GET /api/ HTTP/1.1\r\n",
            b"GET /assets/x HTTP/1.1\r\n",
        ];
        for head in heads {
            assert_eq!(
                exyonq_module_api::static_wire::static_use_blocking_pool(head),
                exyonq_module_api::static_wire::static_wire_use_blocking_pool(head)
                    || exyonq_module_api::static_wire::static_sendfile_use_blocking_pool(head),
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }

    #[test]
    fn tcp_admission_facade_matches_module_hooks() {
        ensure_wire_hooks();
        for head in [
            b"GET /site/1k.bin HTTP/1.1\r\n".as_slice(),
            b"GET /site/1m.bin HTTP/1.1\r\n",
            b"GET /api/ HTTP/1.1\r\n",
        ] {
            assert_eq!(
                exyonq_module_api::static_wire::static_tcp_blocking_pool_admission(head),
                exyonq_module_api::static_wire::static_use_blocking_pool(head),
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }

    /// Core call-site contract: TCP blocking admission matches combined use_blocking_pool.
    #[test]
    fn core_blocking_path_semantics_use_module_api_facades() {
        ensure_wire_hooks();
        let heads = [
            b"GET /site/1k.bin HTTP/1.1\r\n".as_slice(),
            b"HEAD /site/1k.bin HTTP/1.1\r\n",
            b"GET /site/64k.bin HTTP/1.1\r\n",
            b"GET /site/1m.bin HTTP/1.1\r\n",
            b"GET /api/ HTTP/1.1\r\n",
        ];
        for head in heads {
            let sync = exyonq_module_api::static_wire::static_use_blocking_pool(head);
            let tcp = exyonq_module_api::static_wire::static_tcp_blocking_pool_admission(head);
            assert_eq!(tcp, sync, "head={}", String::from_utf8_lossy(head));
            if head.starts_with(b"GET /api/") {
                assert!(!exyonq_module_api::static_wire::epoll_sendfile_eligible(
                    head
                ));
            } else if head.starts_with(b"GET /site/") || head.starts_with(b"HEAD /site/") {
                assert!(exyonq_module_api::static_wire::epoll_sendfile_eligible(
                    head
                ));
            }
        }
    }

    #[test]
    fn filename_is_not_special_for_sendfile_eligibility() {
        ensure_wire_hooks();
        // Cap067: eligibility is capability-based; same predicate for any static GET/HEAD.
        // Cap061: must NOT require bench filenames — /site/foo.bin and /site/64k.bin equal.
        assert_eq!(
            epoll_sendfile_eligible(b"GET /site/64k.bin HTTP/1.1\r\n"),
            epoll_sendfile_eligible(b"GET /site/ordinary.bin HTTP/1.1\r\n")
        );
        assert!(epoll_sendfile_eligible(
            b"GET /site/ordinary.bin HTTP/1.1\r\n"
        ));
        assert!(!epoll_sendfile_eligible(b"GET /health HTTP/1.1\r\n"));
        // RFC 9112 absolute-form request-target uses the same origin path as origin-form.
        assert_eq!(
            epoll_sendfile_eligible(b"GET /site/1k.bin HTTP/1.1\r\n"),
            epoll_sendfile_eligible(b"GET http://exyonq:8080/site/1k.bin HTTP/1.1\r\n")
        );
        assert!(epoll_sendfile_eligible(
            b"GET http://exyonq:8080/site/1k.bin HTTP/1.1\r\n"
        ));
        assert!(!epoll_sendfile_eligible(
            b"GET http://exyonq:8080/api/foo HTTP/1.1\r\n"
        ));
        assert!(!epoll_sendfile_eligible(
            b"GET http://exyonq:8080/health HTTP/1.1\r\n"
        ));
    }

    #[test]
    fn sendfile_eligibility_kill_switch_disables() {
        ensure_wire_hooks();
        exyonq_mod_static::reset_epoll_sendfile_enabled_cache_for_tests();
        std::env::set_var("EXYONQ_EPOLL_SENDFILE", "0");
        assert!(!epoll_sendfile_eligible(b"GET /site/64k.bin HTTP/1.1\r\n"));
        std::env::remove_var("EXYONQ_EPOLL_SENDFILE");
        exyonq_mod_static::reset_epoll_sendfile_enabled_cache_for_tests();
        assert!(epoll_sendfile_eligible(b"GET /site/64k.bin HTTP/1.1\r\n"));
    }

    #[test]
    fn epoll_eligible_health_only() {
        ensure_wire_hooks();
        assert!(epoll_keep_alive_eligible(b"GET /health HTTP/1.1\r\n"));
        // Cap061 LA-008: former bench filenames are not epoll KA eligible.
        assert!(!epoll_keep_alive_eligible(b"GET /site/1k.bin HTTP/1.1\r\n"));
        assert!(!epoll_keep_alive_eligible(
            b"GET /site/routes/route050.bin HTTP/1.1\r\n"
        ));
    }

    #[test]
    fn epoll_not_eligible_proxy_sendfile() {
        ensure_wire_hooks();
        assert!(!epoll_keep_alive_eligible(b"GET /api/ HTTP/1.1\r\n"));
        assert!(!epoll_keep_alive_eligible(b"GET /api/stream HTTP/1.1\r\n"));
        assert!(!epoll_keep_alive_eligible(
            b"GET /site/64k.bin HTTP/1.1\r\n"
        ));
        assert!(!epoll_keep_alive_eligible(b"GET /site/1m.bin HTTP/1.1\r\n"));
    }

    #[test]
    fn tokio_required_proxy_and_site_files() {
        ensure_wire_hooks();
        assert!(tokio_accept_required(b"GET /api/ HTTP/1.1\r\n"));
        assert!(tokio_accept_required(b"GET /api/stream HTTP/1.1\r\n"));
        // Cap067: GET site assets are wire/sendfile candidates → not Tokio-required on Linux.
        #[cfg(target_os = "linux")]
        {
            assert!(!tokio_accept_required(b"GET /site/64k.bin HTTP/1.1\r\n"));
            assert!(!tokio_accept_required(b"GET /site/1m.bin HTTP/1.1\r\n"));
            assert!(!tokio_accept_required(b"GET /site/1k.bin HTTP/1.1\r\n"));
            assert!(!tokio_accept_required(
                b"GET /site/routes/route001.bin HTTP/1.1\r\n"
            ));
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(tokio_accept_required(b"GET /site/64k.bin HTTP/1.1\r\n"));
            assert!(tokio_accept_required(b"GET /site/1m.bin HTTP/1.1\r\n"));
            assert!(tokio_accept_required(b"GET /site/1k.bin HTTP/1.1\r\n"));
            assert!(tokio_accept_required(
                b"GET /site/routes/route001.bin HTTP/1.1\r\n"
            ));
        }
        // Non-GET still cannot take sendfile wire.
        assert!(tokio_accept_required(b"POST /site/1k.bin HTTP/1.1\r\n"));
    }

    #[test]
    fn tokio_not_required_health_metrics_hyper() {
        ensure_wire_hooks();
        assert!(!tokio_accept_required(b"GET /health HTTP/1.1\r\n"));
        // Cap067 hang fix: `/metrics` must Hyper at accept (modules scrape / LA-CAP054-008).
        assert!(tokio_accept_required(b"GET /metrics HTTP/1.1\r\n"));
    }

    /// SUB-LET-WIRE-REJECT-WRITE: TE on wire → Reject → real 400 write succeeds;
    /// must not increment write-error counter (no false failure).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wire_reject_te_emits_400_without_write_error() {
        use std::time::Duration;
        use tokio::io::AsyncReadExt;
        ensure_wire_hooks();
        let _lock = WIRE_REJECT_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let (state, _guard) = state_for(bench_site_config(None)).await;
        assert!(
            !state.modules_enabled(),
            "wire Reject path requires modules disabled"
        );
        let before = super::wire_reject_response_write_errors();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy_client = build_incoming_client();
        // LifecycleState::new() already returns Arc<Self>.
        let ops = crate::lifecycle::LifecycleState::new();
        let state_accept = Arc::clone(&state);
        let pinned_generation = state_accept.generation;
        tokio::spawn(async move {
            let Ok((stream, peer)) = listener.accept().await else {
                return;
            };
            let shared = Arc::new(std::sync::RwLock::new(Arc::clone(&state_accept)));
            let ctx = super::WireDispatchContext {
                shared,
                state: state_accept,
                proxy_client,
                x_forwarded_for: hyper::header::HeaderValue::from_str(&peer.ip().to_string())
                    .unwrap_or_else(|_| hyper::header::HeaderValue::from_static("127.0.0.1")),
                ops,
                pinned_generation,
            };
            super::dispatch_tcp(stream, ctx).await;
        });

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let req = b"GET /site/x HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
        tokio::io::AsyncWriteExt::write_all(&mut client, req)
            .await
            .unwrap();
        let mut buf = vec![0u8; 512];
        let n = tokio::time::timeout(Duration::from_secs(2), client.read(&mut buf))
            .await
            .expect("timed out")
            .expect("read");
        assert!(n > 0, "expected 400 response bytes");
        let text = String::from_utf8_lossy(&buf[..n]);
        assert!(
            text.starts_with("HTTP/1.1 400"),
            "expected 400, got: {text}"
        );
        assert_eq!(
            super::wire_reject_response_write_errors(),
            before,
            "successful 400 write must not count as write failure"
        );
    }

    /// SUB-LET-WIRE-REJECT-WRITE: after Reject is selected, a real peer RST makes
    /// write_all fail; failure must be observed (counter++) — not silent success.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wire_reject_write_failure_observed_on_real_rst() {
        use std::os::fd::AsRawFd;
        use std::time::Duration;
        ensure_wire_hooks();
        let _lock = WIRE_REJECT_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let (state, _guard) = state_for(bench_site_config(None)).await;
        let before = super::wire_reject_response_write_errors();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy_client = build_incoming_client();
        let ops = crate::lifecycle::LifecycleState::new();
        let state_accept = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else {
                    continue;
                };
                let shared = Arc::new(std::sync::RwLock::new(Arc::clone(&state_accept)));
                let ctx = super::WireDispatchContext {
                    shared,
                    state: Arc::clone(&state_accept),
                    proxy_client: proxy_client.clone(),
                    x_forwarded_for: hyper::header::HeaderValue::from_str(&peer.ip().to_string())
                        .unwrap_or_else(|_| hyper::header::HeaderValue::from_static("127.0.0.1")),
                    ops: Arc::clone(&ops),
                    pinned_generation: state_accept.generation,
                };
                tokio::spawn(async move {
                    super::dispatch_tcp(stream, ctx).await;
                });
            }
        });

        let req = b"GET /site/x HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
        let mut observed = before;
        for _ in 0..80 {
            let std_stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            std_stream.set_nodelay(true).ok();
            use std::io::Write;
            let mut std_stream = std_stream;
            std_stream.write_all(req).unwrap();
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
            tokio::time::sleep(Duration::from_millis(25)).await;
            observed = super::wire_reject_response_write_errors();
            if observed > before {
                break;
            }
        }
        assert!(
            observed > before,
            "reject write failure must be observed (before={before} after={observed}); false DELIVERED_400 forbidden"
        );
    }

    /// SUB-LET-PROTOCOL-WRITE-MISC / LET-059: WAF reject on wire → write succeeds;
    /// must not increment WAF write-error counter.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wire_waf_reject_emits_429_without_write_error() {
        use crate::waf::WafRuntimeBinding;
        use exyonq_waf_api::{NopWafEngine, WafAbuseGate, WafDecision, WafPhase};
        use std::net::IpAddr;
        use std::time::Duration;

        struct AlwaysRateLimit;
        impl WafAbuseGate for AlwaysRateLimit {
            fn check(&self, _client_ip: IpAddr, _route_id: Option<&str>) -> Option<WafDecision> {
                Some(WafDecision::abuse_rate_limit(WafPhase::RequestHeaders))
            }
        }

        ensure_wire_hooks();
        let _lock = WIRE_REJECT_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::waf::install_waf_runtime_binding(WafRuntimeBinding {
            engine: Arc::new(NopWafEngine),
            abuse: Some(Arc::new(AlwaysRateLimit)),
            enforce: true,
            wire_inspection_active: true,
        });
        let (state, _guard) = state_for(bench_site_config(None)).await;
        let before = super::wire_waf_reject_response_write_errors();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy_client = build_incoming_client();
        let ops = crate::lifecycle::LifecycleState::new();
        let state_accept = Arc::clone(&state);
        let pinned_generation = state_accept.generation;
        tokio::spawn(async move {
            let Ok((stream, peer)) = listener.accept().await else {
                return;
            };
            let shared = Arc::new(std::sync::RwLock::new(Arc::clone(&state_accept)));
            let ctx = super::WireDispatchContext {
                shared,
                state: state_accept,
                proxy_client,
                x_forwarded_for: hyper::header::HeaderValue::from_str(&peer.ip().to_string())
                    .unwrap_or_else(|_| hyper::header::HeaderValue::from_static("127.0.0.1")),
                ops,
                pinned_generation,
            };
            super::dispatch_tcp(stream, ctx).await;
        });

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let req = b"GET /api/x HTTP/1.1\r\nHost: x\r\n\r\n";
        tokio::io::AsyncWriteExt::write_all(&mut client, req)
            .await
            .unwrap();
        let mut buf = vec![0u8; 512];
        let n = tokio::time::timeout(Duration::from_secs(2), client.read(&mut buf))
            .await
            .expect("timed out")
            .expect("read");
        assert!(n > 0, "expected WAF reject response bytes");
        let text = String::from_utf8_lossy(&buf[..n]);
        assert!(
            text.starts_with("HTTP/1.1 429"),
            "expected 429, got: {text}"
        );
        assert_eq!(
            super::wire_waf_reject_response_write_errors(),
            before,
            "successful WAF reject write must not count as write failure"
        );
        crate::waf::install_waf_runtime_binding(WafRuntimeBinding::default());
    }

    /// SUB-LET-PROTOCOL-WRITE-MISC / LET-059: after WAF reject is selected, a real peer
    /// RST makes write_all fail; failure must be observed — not silent waf_* delivery.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wire_waf_reject_write_failure_observed_on_real_rst() {
        use crate::waf::WafRuntimeBinding;
        use exyonq_waf_api::{NopWafEngine, WafAbuseGate, WafDecision, WafPhase};
        use std::net::IpAddr;
        use std::os::fd::AsRawFd;
        use std::time::Duration;

        struct AlwaysRateLimit;
        impl WafAbuseGate for AlwaysRateLimit {
            fn check(&self, _client_ip: IpAddr, _route_id: Option<&str>) -> Option<WafDecision> {
                Some(WafDecision::abuse_rate_limit(WafPhase::RequestHeaders))
            }
        }

        ensure_wire_hooks();
        let _lock = WIRE_REJECT_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::waf::install_waf_runtime_binding(WafRuntimeBinding {
            engine: Arc::new(NopWafEngine),
            abuse: Some(Arc::new(AlwaysRateLimit)),
            enforce: true,
            wire_inspection_active: true,
        });
        let (state, _guard) = state_for(bench_site_config(None)).await;
        let before = super::wire_waf_reject_response_write_errors();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy_client = build_incoming_client();
        let ops = crate::lifecycle::LifecycleState::new();
        let state_accept = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else {
                    continue;
                };
                let shared = Arc::new(std::sync::RwLock::new(Arc::clone(&state_accept)));
                let ctx = super::WireDispatchContext {
                    shared,
                    state: Arc::clone(&state_accept),
                    proxy_client: proxy_client.clone(),
                    x_forwarded_for: hyper::header::HeaderValue::from_str(&peer.ip().to_string())
                        .unwrap_or_else(|_| hyper::header::HeaderValue::from_static("127.0.0.1")),
                    ops: Arc::clone(&ops),
                    pinned_generation: state_accept.generation,
                };
                tokio::spawn(async move {
                    super::dispatch_tcp(stream, ctx).await;
                });
            }
        });

        let req = b"GET /api/x HTTP/1.1\r\nHost: x\r\n\r\n";
        let mut observed = before;
        for _ in 0..80 {
            let std_stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            std_stream.set_nodelay(true).ok();
            use std::io::Write;
            let mut std_stream = std_stream;
            std_stream.write_all(req).unwrap();
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
            tokio::time::sleep(Duration::from_millis(25)).await;
            observed = super::wire_waf_reject_response_write_errors();
            if observed > before {
                break;
            }
        }
        crate::waf::install_waf_runtime_binding(WafRuntimeBinding::default());
        assert!(
            observed > before,
            "waf reject write failure must be observed (before={before} after={observed}); false DELIVERED_WAF_REJECT forbidden"
        );
    }
}

#[cfg(test)]
mod ws_tunnel_tests {
    use super::{dispatch_tcp, WireDispatchContext};
    use crate::config::{
        AppConfig, RouteConfig, RouteMatch, ServerConfig, ServerNames, UpstreamConfig,
    };
    use crate::execute_backend::StaticDispatchTestGuard;
    use crate::server::state::ServerState;
    use exyonq_mod_proxy::{build_incoming_client, install_kernel_hooks, ProxyRuntime};
    use exyonq_module_api::static_dispatch::StaticDispatchService;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn ws_frame_text(text: &str) -> Vec<u8> {
        let payload = text.as_bytes();
        let mut frame = vec![0x81, 0x80 | payload.len() as u8];
        let mask = [0x11, 0x22, 0x33, 0x44];
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ mask[i % 4])
                .collect::<Vec<_>>(),
        );
        frame
    }

    fn parse_ws_text(payload: &[u8]) -> Option<String> {
        if payload.len() < 2 {
            return None;
        }
        let masked = (payload[1] & 0x80) != 0;
        let mut idx = 2usize;
        let mut length = (payload[1] & 0x7f) as usize;
        if length == 126 {
            length = u16::from_be_bytes([payload[2], payload[3]]) as usize;
            idx = 4;
        }
        let mask = if masked {
            let m = &payload[idx..idx + 4];
            idx += 4;
            Some(m)
        } else {
            None
        };
        let data = &payload[idx..idx + length];
        let unmasked = if let Some(m) = mask {
            data.iter()
                .enumerate()
                .map(|(i, b)| b ^ m[i % 4])
                .collect::<Vec<_>>()
        } else {
            data.to_vec()
        };
        String::from_utf8(unmasked).ok()
    }

    async fn spawn_ws_echo_upstream() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    continue;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let key = text.lines().find_map(|line| {
                        let (k, v) = line.split_once(':')?;
                        k.trim()
                            .eq_ignore_ascii_case("sec-websocket-key")
                            .then_some(v.trim().to_string())
                    });
                    let Some(key) = key else {
                        return;
                    };
                    let accept_val = {
                        use base64::Engine;
                        use sha1::{Digest, Sha1};
                        let mut hasher = Sha1::new();
                        hasher.update(key.as_bytes());
                        hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
                        base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
                    };
                    let resp = format!(
                        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept_val}\r\n\r\n"
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    loop {
                        let n = stream.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        if let Some(msg) = parse_ws_text(&buf[..n]) {
                            let mut out = vec![0x81, msg.len() as u8];
                            out.extend_from_slice(msg.as_bytes());
                            let _ = stream.write_all(&out).await;
                        }
                    }
                });
            }
        });
        port
    }

    fn minimal_proxy_config(listen_port: u16, upstream_port: u16) -> AppConfig {
        let api = RouteConfig {
            name: "api".into(),
            r#match: RouteMatch {
                path: "/api".into(),
                host: None,
            },
            upstream: Some("backend".into()),
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        };
        let mut upstreams = HashMap::new();
        upstreams.insert(
            "backend".into(),
            UpstreamConfig::legacy("backend", format!("http://127.0.0.1:{upstream_port}"), 5000),
        );
        AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: format!("127.0.0.1:{listen_port}"),
                server_name: ServerNames::None,
                routes: vec!["api".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![api],
            upstreams,
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        }
    }

    struct WireTunnelTestHooks {
        _gate: tokio::sync::MutexGuard<'static, ()>,
        _static: StaticDispatchTestGuard,
        _proxy: crate::execute_backend::ProxyDispatchTestGuard,
    }

    static WS_TUNNEL_TEST_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn install_cli_hooks() -> WireTunnelTestHooks {
        let gate = WS_TUNNEL_TEST_GATE.lock().await;
        let static_runtime = Arc::new(exyonq_mod_static::StaticRuntime::new());
        let static_service: Arc<dyn StaticDispatchService> = static_runtime.clone();
        crate::execute_backend::clear_global_static_dispatch_for_register_once_test();
        crate::register_static_dispatch_service(static_service).expect("static register");
        exyonq_mod_static::install_kernel_hooks(Arc::clone(&static_runtime));
        let proxy_runtime = Arc::new(ProxyRuntime::new());
        install_kernel_hooks(Arc::clone(&proxy_runtime));
        let proxy_service: Arc<dyn exyonq_module_api::proxy_dispatch::ProxyDispatchService> =
            proxy_runtime.clone();
        crate::execute_backend::clear_global_proxy_dispatch_for_register_once_test();
        crate::register_proxy_dispatch_service(Arc::clone(&proxy_service)).expect("proxy register");
        WireTunnelTestHooks {
            _gate: gate,
            _static: StaticDispatchTestGuard::install(
                static_runtime as Arc<dyn StaticDispatchService>,
            ),
            _proxy: crate::execute_backend::ProxyDispatchTestGuard::install(proxy_service),
        }
    }

    async fn install_hooks() -> WireTunnelTestHooks {
        let gate = WS_TUNNEL_TEST_GATE.lock().await;
        let static_runtime = Arc::new(exyonq_mod_static::StaticRuntime::new());
        exyonq_mod_static::install_kernel_hooks(Arc::clone(&static_runtime));
        let static_service: Arc<dyn StaticDispatchService> = static_runtime.clone();
        crate::execute_backend::clear_global_static_dispatch_for_register_once_test();
        let _ = crate::register_static_dispatch_service(static_service);
        let proxy_runtime = Arc::new(ProxyRuntime::new());
        install_kernel_hooks(Arc::clone(&proxy_runtime));
        let proxy_service: Arc<dyn exyonq_module_api::proxy_dispatch::ProxyDispatchService> =
            proxy_runtime.clone();
        crate::execute_backend::clear_global_proxy_dispatch_for_register_once_test();
        let _ = crate::register_proxy_dispatch_service(Arc::clone(&proxy_service));
        WireTunnelTestHooks {
            _gate: gate,
            _static: StaticDispatchTestGuard::install(
                static_runtime as Arc<dyn StaticDispatchService>,
            ),
            _proxy: crate::execute_backend::ProxyDispatchTestGuard::install(proxy_service),
        }
    }

    async fn ws_echo_through_dispatch(coalesce_request: bool) -> String {
        let _hooks = install_hooks().await;
        let upstream_port = spawn_ws_echo_upstream().await;
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_port = proxy_listener.local_addr().unwrap().port();
        let proxy_client = build_incoming_client();
        let config = minimal_proxy_config(listen_port, upstream_port);
        let state = ServerState::new_with_generation(1, config, proxy_client.clone())
            .await
            .expect("server state");
        // LifecycleState::new() already returns Arc<Self>.
        let ops = crate::lifecycle::LifecycleState::new();

        tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = proxy_listener.accept().await else {
                    continue;
                };
                let shared = Arc::new(std::sync::RwLock::new(Arc::clone(&state)));
                let ctx = WireDispatchContext {
                    shared,
                    state: Arc::clone(&state),
                    proxy_client: proxy_client.clone(),
                    x_forwarded_for: hyper::header::HeaderValue::from_str(&peer.ip().to_string())
                        .unwrap_or_else(|_| hyper::header::HeaderValue::from_static("127.0.0.1")),
                    ops: Arc::clone(&ops),
                    pinned_generation: state.generation,
                };
                tokio::spawn(async move {
                    dispatch_tcp(stream, ctx).await;
                });
            }
        });

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let mut req = format!(
            "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .into_bytes();
        if coalesce_request {
            req.extend(ws_frame_text("ping-kd34"));
        }
        stream.write_all(&req).await.unwrap();

        let mut buf = vec![0u8; 4096];
        let mut resp = Vec::new();
        while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0, "unexpected eof before 101");
            resp.extend_from_slice(&buf[..n]);
        }
        assert!(
            String::from_utf8_lossy(&resp).contains("101"),
            "missing 101: {}",
            String::from_utf8_lossy(&resp)
        );

        if !coalesce_request {
            stream.write_all(&ws_frame_text("ping-kd34")).await.unwrap();
        }

        let body_start = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(resp.len());
        let mut pending = resp[body_start..].to_vec();
        while pending.len() < 2 {
            let n = stream.read(&mut buf).await.unwrap();
            pending.extend_from_slice(&buf[..n]);
        }
        let hdr = &pending[..2];
        let mut rest = pending[2..].to_vec();
        let length = (hdr[1] & 0x7f) as usize;
        while rest.len() < length {
            let n = stream.read(&mut buf).await.unwrap();
            rest.extend_from_slice(&buf[..n]);
        }
        let mut frame = hdr.to_vec();
        frame.extend_from_slice(&rest[..length]);
        parse_ws_text(&frame).expect("echo frame")
    }

    #[tokio::test]
    async fn wire_dispatch_ws_echo_exact_bytes() {
        let echo = ws_echo_through_dispatch(false).await;
        assert_eq!(echo, "ping-kd34");
    }

    #[tokio::test]
    async fn wire_dispatch_ws_echo_coalesced_request() {
        let echo = ws_echo_through_dispatch(true).await;
        assert_eq!(echo, "ping-kd34");
    }

    async fn ws_echo_through_run_on(coalesce_request: bool) -> String {
        let _hooks = install_hooks().await;
        let upstream_port = spawn_ws_echo_upstream().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = minimal_proxy_config(listen_port, upstream_port);
        let listen_addr: std::net::SocketAddr = format!("127.0.0.1:{listen_port}").parse().unwrap();
        tokio::spawn(async move {
            let _ = crate::server::run_on(listen_addr, config).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let mut req = format!(
            "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .into_bytes();
        if coalesce_request {
            req.extend(ws_frame_text("ping-kd34"));
        }
        stream.write_all(&req).await.unwrap();

        let mut buf = vec![0u8; 4096];
        let mut resp = Vec::new();
        while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0);
            resp.extend_from_slice(&buf[..n]);
        }
        assert!(String::from_utf8_lossy(&resp).contains("101"));

        if !coalesce_request {
            stream.write_all(&ws_frame_text("ping-kd34")).await.unwrap();
        }

        let body_start = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(resp.len());
        let mut pending = resp[body_start..].to_vec();
        while pending.len() < 2 {
            let n = stream.read(&mut buf).await.unwrap();
            pending.extend_from_slice(&buf[..n]);
        }
        let hdr = &pending[..2];
        let mut rest = pending[2..].to_vec();
        let length = (hdr[1] & 0x7f) as usize;
        while rest.len() < length {
            let n = stream.read(&mut buf).await.unwrap();
            rest.extend_from_slice(&buf[..n]);
        }
        let mut frame = hdr.to_vec();
        frame.extend_from_slice(&rest[..length]);
        parse_ws_text(&frame).expect("echo frame")
    }

    #[tokio::test]
    async fn run_on_ws_echo_with_cli_registration() {
        let _hooks = install_cli_hooks().await;
        let upstream_port = spawn_ws_echo_upstream().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = minimal_proxy_config(listen_port, upstream_port);
        tokio::spawn(async move {
            let _ = crate::server::run(config).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        stream
            .write_all(format!(
                "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            ).as_bytes())
            .await
            .unwrap();
        let mut buf = vec![0u8; 4096];
        let mut resp = Vec::new();
        while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            resp.extend_from_slice(&buf[..n]);
        }
        stream.write_all(&ws_frame_text("ping-kd34")).await.unwrap();
        let body_start = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(resp.len());
        let mut pending = resp[body_start..].to_vec();
        while pending.len() < 2 {
            let n = stream.read(&mut buf).await.unwrap();
            pending.extend_from_slice(&buf[..n]);
        }
        let hdr = &pending[..2];
        let mut rest = pending[2..].to_vec();
        let length = (hdr[1] & 0x7f) as usize;
        while rest.len() < length {
            let n = stream.read(&mut buf).await.unwrap();
            rest.extend_from_slice(&buf[..n]);
        }
        let mut frame = hdr.to_vec();
        frame.extend_from_slice(&rest[..length]);
        let echo = parse_ws_text(&frame).expect("echo");
        assert_eq!(echo, "ping-kd34");
    }

    #[tokio::test]
    async fn run_on_ws_echo_exact_bytes() {
        let echo = ws_echo_through_run_on(false).await;
        assert_eq!(echo, "ping-kd34");
    }

    #[tokio::test]
    async fn run_on_ws_echo_after_health_get() {
        let _hooks = install_hooks().await;
        let upstream_port = spawn_ws_echo_upstream().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = minimal_proxy_config(listen_port, upstream_port);
        let listen_addr: std::net::SocketAddr = format!("127.0.0.1:{listen_port}").parse().unwrap();
        tokio::spawn(async move {
            let _ = crate::server::run_on(listen_addr, config).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let mut health = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        health
            .write_all(
                format!(
                    "GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut buf = [0u8; 512];
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(2), health.read(&mut buf)).await;
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        stream
            .write_all(format!(
                "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            ).as_bytes())
            .await
            .unwrap();
        let mut buf = vec![0u8; 4096];
        let mut resp = Vec::new();
        while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0, "unexpected eof before ws upgrade response");
            resp.extend_from_slice(&buf[..n]);
        }
        assert!(
            String::from_utf8_lossy(&resp).contains("101"),
            "missing 101: {}",
            String::from_utf8_lossy(&resp)
        );
        stream.write_all(&ws_frame_text("ping-kd34")).await.unwrap();
        let body_start = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(resp.len());
        let mut pending = resp[body_start..].to_vec();
        while pending.len() < 2 {
            let n = stream.read(&mut buf).await.unwrap();
            pending.extend_from_slice(&buf[..n]);
        }
        let hdr = &pending[..2];
        let mut rest = pending[2..].to_vec();
        let length = (hdr[1] & 0x7f) as usize;
        while rest.len() < length {
            let n = stream.read(&mut buf).await.unwrap();
            rest.extend_from_slice(&buf[..n]);
        }
        let mut frame = hdr.to_vec();
        frame.extend_from_slice(&rest[..length]);
        let echo = parse_ws_text(&frame).expect("echo");
        assert_eq!(echo, "ping-kd34");
    }

    #[test]
    fn arch002_wire_inspection_active_predicate() {
        use crate::waf::compute_wire_inspection_active;
        // Pure WAF-off P1: no signatures, no enforce+abuse → skip materialization.
        assert!(!compute_wire_inspection_active(false, false, false));
        assert!(!compute_wire_inspection_active(false, true, false));
        // enabled=false but abuse+enforce still requires wire inspect (pre-ARCH-002 behavior).
        assert!(compute_wire_inspection_active(false, true, true));
        // Monitor/Block signatures active regardless of enforce.
        assert!(compute_wire_inspection_active(true, false, false));
        assert!(compute_wire_inspection_active(true, false, true));
        // Env-resolved Block over IR Disabled is signature_active=true at composition.
        assert!(compute_wire_inspection_active(true, false, true));
    }
}
