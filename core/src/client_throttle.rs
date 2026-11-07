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
//! OL-03 — OpenLiteSpeed-style per-client throttling.
//!
//! Implements the complete `perClientConnLimit` surface: static/dynamic
//! requests per second, inbound/outbound bandwidth, soft/hard concurrent
//! connection limits with grace/ban, and `blockBadReq`.
//!
//! OpenLiteSpeed counts TCP connections, not requests: HTTP/1.1 keep-alive and
//! HTTP/2 multiplexed streams occupy one slot for the lifetime of the
//! connection. ExyonQ applies the same unit at the connection boundary and
//! closes the connection when a limit is exceeded (it never answers `429` for
//! these limits). The request-rate and bandwidth buckets remain per request.

use bytes::Bytes;
use exyonq_config_ir::ClientThrottleConfig;
use http_body::{Body, Frame};
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use std::collections::HashMap;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::{LazyLock, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

type BodyBox = BoxBody<Bytes, exyonq_module_api::BoxError>;

const MAX_CLIENTS: usize = 10_000;
const RATE_WINDOW: Duration = Duration::from_secs(1);
const BAD_REQUEST_THRESHOLD: u32 = 3;

struct RateWindow {
    start: Instant,
    used: u32,
}

impl RateWindow {
    fn new(now: Instant) -> Self {
        Self {
            start: now,
            used: 0,
        }
    }
}

struct ClientState {
    active: u32,
    ban_until: Option<Instant>,
    /// Incremented when a hard-limit refusal arms a ban. Bodies created before
    /// the refusal abort on their next poll, matching OLS truncating live
    /// responses when the hard connection limit is reached.
    hard_abort_epoch: u64,
    /// First instant the client was observed at or above the soft limit.
    soft_since: Option<Instant>,
    out_tokens: f64,
    out_at: Instant,
    out_started: bool,
    in_tokens: f64,
    in_at: Instant,
    in_started: bool,
    static_window: RateWindow,
    dynamic_window: RateWindow,
    bad_requests: u32,
}

impl ClientState {
    fn new(now: Instant) -> Self {
        Self {
            active: 0,
            ban_until: None,
            hard_abort_epoch: 0,
            soft_since: None,
            out_tokens: 0.0,
            out_at: now,
            out_started: false,
            in_tokens: 0.0,
            in_at: now,
            in_started: false,
            static_window: RateWindow::new(now),
            dynamic_window: RateWindow::new(now),
            bad_requests: 0,
        }
    }
}

static CLIENTS: LazyLock<Mutex<HashMap<IpAddr, ClientState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Why a connection was refused. The caller closes the accepted socket; no
/// HTTP response is produced (OpenLiteSpeed's connection-limit behaviour).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThrottleDecision {
    SoftLimit,
    HardLimit,
    Banned,
}

pub(crate) struct ClientGuard {
    ip: IpAddr,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        if let Ok(mut clients) = CLIENTS.lock() {
            if let Some(state) = clients.get_mut(&self.ip) {
                state.active = state.active.saturating_sub(1);
            }
        }
    }
}

pub(crate) fn hard_abort_epoch(ip: IpAddr) -> u64 {
    let Ok(clients) = CLIENTS.lock() else {
        return 0;
    };
    clients
        .get(&ip)
        .map(|state| state.hard_abort_epoch)
        .unwrap_or(0)
}

fn hard_abort_changed(ip: IpAddr, epoch: u64) -> bool {
    hard_abort_epoch(ip) != epoch
}

fn state_for(
    clients: &mut HashMap<IpAddr, ClientState>,
    ip: IpAddr,
    now: Instant,
) -> &mut ClientState {
    if clients.len() >= MAX_CLIENTS && !clients.contains_key(&ip) {
        if let Some(victim) = clients.keys().next().copied() {
            clients.remove(&victim);
        }
    }
    clients.entry(ip).or_insert_with(|| ClientState::new(now))
}

fn arm_ban(state: &mut ClientState, now: Instant, ban_seconds: u64) {
    state.soft_since = None;
    if ban_seconds > 0 {
        state.ban_until = Some(now + Duration::from_secs(ban_seconds));
    }
}

/// Acquire one concurrent-connection slot. `Ok(None)` means throttling is off.
///
/// This must be called exactly once per accepted connection, before serving
/// requests on it, and the guard must live until the socket is closed. A
/// rejection means the caller drops/closes the socket without writing a
/// response, matching OpenLiteSpeed's hard/soft connection handling.
pub(crate) fn acquire_connection(
    config: &ClientThrottleConfig,
    ip: IpAddr,
) -> Result<Option<ClientGuard>, ThrottleDecision> {
    if !config.enabled {
        return Ok(None);
    }
    let now = Instant::now();
    let mut clients = CLIENTS.lock().unwrap_or_else(|p| p.into_inner());
    let state = state_for(&mut clients, ip, now);
    if let Some(until) = state.ban_until {
        if until > now {
            return Err(ThrottleDecision::Banned);
        }
        state.ban_until = None;
        state.soft_since = None;
    }
    // OpenLiteSpeed hard limit: the new connection is refused and the IP is
    // marked as a bot for the ban period. Existing connections are not the
    // unit that is rejected here.
    if let Some(limit) = config.max_connections_per_ip {
        if state.active >= limit {
            if config.ban_seconds > 0 {
                state.hard_abort_epoch = state.hard_abort_epoch.wrapping_add(1);
            }
            arm_ban(state, now, config.ban_seconds);
            return Err(ThrottleDecision::HardLimit);
        }
    }
    // Soft limit: accept while inside the configured grace window, then ban
    // and refuse the connection that observes the expired grace. OpenLiteSpeed
    // clears the grace marker from its 10 s timer when the live count has
    // fallen to half the soft limit and the grace window is still open; apply
    // the same condition when the next connection arrives.
    if let Some(soft) = config.soft_connections_per_ip {
        if state.active <= soft / 2
            && state.soft_since.is_some_and(|since| {
                now.duration_since(since) < Duration::from_secs(config.grace_seconds)
            })
        {
            state.soft_since = None;
        }
        if state.active >= soft {
            let since = *state.soft_since.get_or_insert(now);
            if config.ban_seconds > 0
                && now.duration_since(since) >= Duration::from_secs(config.grace_seconds)
            {
                arm_ban(state, now, config.ban_seconds);
                return Err(ThrottleDecision::SoftLimit);
            }
        }
    }
    state.active += 1;
    Ok(Some(ClientGuard { ip }))
}

/// OpenLiteSpeed static/dynamic request quota. The excess waits for the next
/// one-second window instead of being rejected.
pub(crate) fn request_wait(config: &ClientThrottleConfig, ip: IpAddr, dynamic: bool) -> Duration {
    if !config.enabled {
        return Duration::ZERO;
    }
    let rate = if dynamic {
        config.dynamic_requests_per_second
    } else {
        config.static_requests_per_second
    };
    let Some(rate) = rate.filter(|rate| *rate > 0) else {
        return Duration::ZERO;
    };
    let now = Instant::now();
    let mut clients = CLIENTS.lock().unwrap_or_else(|p| p.into_inner());
    let state = state_for(&mut clients, ip, now);
    let window = if dynamic {
        &mut state.dynamic_window
    } else {
        &mut state.static_window
    };
    if now
        .checked_duration_since(window.start)
        .is_some_and(|elapsed| elapsed >= RATE_WINDOW)
    {
        window.start = now;
        window.used = 0;
    }
    let future_wait = window
        .start
        .checked_duration_since(now)
        .unwrap_or(Duration::ZERO);
    if window.used < rate {
        window.used += 1;
        return future_wait;
    }
    let mut next = if window.start > now {
        window.start
    } else {
        window.start + RATE_WINDOW
    };
    while next <= now {
        next += RATE_WINDOW;
    }
    window.start = next;
    window.used = 1;
    next.saturating_duration_since(now) + Duration::from_millis(1)
}

fn token_wait(ip: IpAddr, bytes: usize, rate: u64, inbound: bool, now: Instant) -> Duration {
    let mut clients = CLIENTS.lock().unwrap_or_else(|p| p.into_inner());
    let state = state_for(&mut clients, ip, now);
    let (tokens, at) = if inbound {
        (&mut state.in_tokens, &mut state.in_at)
    } else {
        (&mut state.out_tokens, &mut state.out_at)
    };
    let started = if inbound {
        &mut state.in_started
    } else {
        &mut state.out_started
    };
    if !*started {
        // OpenLiteSpeed's bucket starts with one second of capacity, so the
        // first frame is sent immediately instead of being held for a full
        // refill interval.
        *tokens = rate as f64;
        *started = true;
    } else {
        let elapsed = now
            .checked_duration_since(*at)
            .unwrap_or(Duration::ZERO)
            .as_secs_f64();
        *tokens = (*tokens + elapsed * rate as f64).min(rate as f64);
    }
    *at = now;
    *tokens -= bytes as f64;
    if *tokens >= 0.0 {
        Duration::ZERO
    } else {
        let wait = -*tokens / rate as f64;
        *tokens = 0.0;
        Duration::from_secs_f64(wait.min(60.0))
    }
}

/// Total delay for an already-buffered HTTP/3 request body.
pub(crate) fn inbound_wait(config: &ClientThrottleConfig, ip: IpAddr, bytes: usize) -> Duration {
    let Some(rate) = config.input_bandwidth_bytes_per_second else {
        return Duration::ZERO;
    };
    if !config.enabled || bytes == 0 {
        return Duration::ZERO;
    }
    token_wait(ip, bytes, rate, true, Instant::now())
}

/// Record a protocol-level malformed request. Three consecutive bad requests
/// arm the same ban used by the soft connection limit.
pub(crate) fn note_bad_request(config: &ClientThrottleConfig, ip: IpAddr) {
    if !config.enabled || !config.block_bad_req || config.ban_seconds == 0 {
        return;
    }
    let now = Instant::now();
    let mut clients = CLIENTS.lock().unwrap_or_else(|p| p.into_inner());
    let state = state_for(&mut clients, ip, now);
    if state.ban_until.is_some_and(|until| until > now) {
        return;
    }
    state.bad_requests = state.bad_requests.saturating_add(1);
    if state.bad_requests >= BAD_REQUEST_THRESHOLD {
        state.bad_requests = 0;
        state.ban_until = Some(now + Duration::from_secs(config.ban_seconds));
        state.soft_since = None;
    }
}

/// Split one data frame at the currently available token budget. The returned
/// head is safe to emit now; the tail (if any) must wait one refill interval.
fn token_chunk(
    ip: IpAddr,
    data: Bytes,
    rate: u64,
    inbound: bool,
    now: Instant,
) -> (Bytes, Option<Bytes>, Duration) {
    let mut clients = CLIENTS.lock().unwrap_or_else(|p| p.into_inner());
    let state = state_for(&mut clients, ip, now);
    let (tokens, at) = if inbound {
        (&mut state.in_tokens, &mut state.in_at)
    } else {
        (&mut state.out_tokens, &mut state.out_at)
    };
    let started = if inbound {
        &mut state.in_started
    } else {
        &mut state.out_started
    };
    if !*started {
        // OpenLiteSpeed starts with one second of capacity, so the first
        // frame is sent immediately instead of being held for a full refill.
        *tokens = rate as f64;
        *started = true;
    } else {
        let elapsed = now
            .checked_duration_since(*at)
            .unwrap_or(Duration::ZERO)
            .as_secs_f64();
        *tokens = (*tokens + elapsed * rate as f64).min(rate as f64);
    }
    *at = now;
    let available = (*tokens).floor().clamp(0.0, data.len() as f64) as usize;
    *tokens -= available as f64;
    if available >= data.len() {
        return (data, None, Duration::ZERO);
    }
    let head = data.slice(..available);
    let tail = data.slice(available..);
    // One second refills the bucket to its one-second capacity. The exact
    // refill is recomputed when the tail is polled.
    (head, Some(tail), RATE_WINDOW)
}

struct ThrottledBody<B> {
    inner: B,
    _guard: Option<ClientGuard>,
    ip: IpAddr,
    hard_epoch: u64,
    rate: Option<u64>,
    inbound: bool,
    held: Option<Bytes>,
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl<B> Body for ThrottledBody<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<exyonq_module_api::BoxError>,
{
    type Data = Bytes;
    type Error = exyonq_module_api::BoxError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if let Some(sleep) = self.sleep.as_mut() {
            if sleep.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            self.sleep = None;
        }
        if hard_abort_changed(self.ip, self.hard_epoch) {
            return Poll::Ready(Some(Err(Box::new(std::io::Error::other(
                "per-client hard connection limit exceeded",
            )))));
        }
        if let Some(tail) = self.held.take() {
            let Some(rate) = self.rate else {
                return Poll::Ready(Some(Ok(Frame::data(tail))));
            };
            let (head, more, wait) = token_chunk(self.ip, tail, rate, self.inbound, Instant::now());
            if let Some(more) = more {
                self.held = Some(more);
                self.sleep = Some(Box::pin(tokio::time::sleep(wait)));
            }
            if !head.is_empty() {
                return Poll::Ready(Some(Ok(Frame::data(head))));
            }
            return Poll::Pending;
        }
        match Pin::new(&mut self.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                let data = match frame.into_data() {
                    Ok(data) => data,
                    Err(frame) => return Poll::Ready(Some(Ok(frame))),
                };
                if data.is_empty() {
                    return Poll::Ready(Some(Ok(Frame::data(data))));
                }
                let Some(rate) = self.rate else {
                    return Poll::Ready(Some(Ok(Frame::data(data))));
                };
                let (head, more, wait) =
                    token_chunk(self.ip, data, rate, self.inbound, Instant::now());
                if let Some(more) = more {
                    self.held = Some(more);
                    self.sleep = Some(Box::pin(tokio::time::sleep(wait)));
                }
                if !head.is_empty() {
                    return Poll::Ready(Some(Ok(Frame::data(head))));
                }
                Poll::Pending
            }
            Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err.into()))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream() && self.held.is_none()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

pub(crate) fn wrap_body(
    body: BodyBox,
    guard: Option<ClientGuard>,
    ip: IpAddr,
    config: &ClientThrottleConfig,
) -> BodyBox {
    if !config.enabled {
        return body;
    }
    BoxBody::new(ThrottledBody {
        inner: body,
        _guard: guard,
        ip,
        hard_epoch: hard_abort_epoch(ip),
        rate: config.bandwidth_bytes_per_second,
        inbound: false,
        held: None,
        sleep: None,
    })
}

/// Wrap an inbound request body. The guard keeps the concurrent slot until the
/// body (including a failed read) is dropped.
pub(crate) fn wrap_request_body<B>(
    body: B,
    guard: Option<ClientGuard>,
    ip: IpAddr,
    config: &ClientThrottleConfig,
) -> BodyBox
where
    B: Body<Data = Bytes> + Unpin + Send + Sync + 'static,
    B::Error: Into<exyonq_module_api::BoxError> + 'static,
{
    if !config.enabled {
        return BoxBody::new(body.map_err(Into::into));
    }
    BoxBody::new(ThrottledBody {
        inner: body,
        _guard: guard,
        ip,
        hard_epoch: hard_abort_epoch(ip),
        rate: config.input_bandwidth_bytes_per_second,
        inbound: true,
        held: None,
        sleep: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ClientThrottleConfig {
        ClientThrottleConfig {
            enabled: true,
            soft_connections_per_ip: None,
            max_connections_per_ip: Some(1),
            bandwidth_bytes_per_second: None,
            input_bandwidth_bytes_per_second: None,
            static_requests_per_second: None,
            dynamic_requests_per_second: None,
            block_bad_req: false,
            ban_seconds: 0,
            grace_seconds: 0,
        }
    }

    fn soft_config() -> ClientThrottleConfig {
        ClientThrottleConfig {
            enabled: true,
            soft_connections_per_ip: Some(1),
            max_connections_per_ip: Some(2),
            bandwidth_bytes_per_second: None,
            input_bandwidth_bytes_per_second: None,
            static_requests_per_second: None,
            dynamic_requests_per_second: None,
            block_bad_req: false,
            ban_seconds: 1,
            grace_seconds: 0,
        }
    }

    #[test]
    fn concurrent_connection_slot_is_released_on_drop() {
        let ip = IpAddr::from([192, 0, 2, 10]);
        let first = acquire_connection(&config(), ip)
            .expect("first")
            .expect("guard");
        assert!(matches!(
            acquire_connection(&config(), ip),
            Err(ThrottleDecision::HardLimit)
        ));
        // A request on the already-admitted connection must not consume a
        // second slot. This is the unit-of-count regression: only the TCP
        // connection owns the guard.
        assert!(request_wait(&config(), ip, false).is_zero());
        drop(first);
        assert!(acquire_connection(&config(), ip)
            .expect("after drop")
            .is_some());
    }

    #[test]
    fn soft_limit_ban_is_armed_after_grace() {
        let ip = IpAddr::from([192, 0, 2, 11]);
        let mut cfg = soft_config();
        cfg.grace_seconds = 0;
        let first = acquire_connection(&cfg, ip).expect("first").expect("guard");
        assert!(matches!(
            acquire_connection(&cfg, ip),
            Err(ThrottleDecision::SoftLimit)
        ));
        drop(first);
        std::thread::sleep(Duration::from_millis(1_100));
        assert!(acquire_connection(&soft_config(), ip)
            .expect("after ban")
            .is_some());
    }

    #[test]
    fn hard_limit_rejects_before_soft_grace() {
        let ip = IpAddr::from([192, 0, 2, 12]);
        let mut cfg = soft_config();
        cfg.ban_seconds = 0;
        cfg.grace_seconds = 60;
        let _first = acquire_connection(&cfg, ip).expect("first").expect("guard");
        let _second = acquire_connection(&cfg, ip)
            .expect("second")
            .expect("guard");
        assert!(matches!(
            acquire_connection(&cfg, ip),
            Err(ThrottleDecision::HardLimit)
        ));
    }

    #[test]
    fn soft_above_hard_is_hard_limited_first() {
        let ip = IpAddr::from([192, 0, 2, 17]);
        let mut cfg = soft_config();
        cfg.soft_connections_per_ip = Some(3);
        cfg.max_connections_per_ip = Some(1);
        cfg.ban_seconds = 60;
        let _first = acquire_connection(&cfg, ip).expect("first").expect("guard");
        assert!(matches!(
            acquire_connection(&cfg, ip),
            Err(ThrottleDecision::HardLimit)
        ));
    }

    #[test]
    fn below_half_soft_limit_resets_grace_on_next_connection() {
        let ip = IpAddr::from([192, 0, 2, 13]);
        let mut cfg = soft_config();
        cfg.soft_connections_per_ip = Some(2);
        cfg.max_connections_per_ip = Some(3);
        cfg.grace_seconds = 60;
        let first = acquire_connection(&cfg, ip).expect("first").expect("guard");
        let second = acquire_connection(&cfg, ip)
            .expect("second")
            .expect("guard");
        drop(second);
        // active == 1, which is <= soft/2 and inside the grace window; OLS's
        // timer clears overLimitTime under the same condition.
        let fresh = acquire_connection(&cfg, ip)
            .expect("fresh grace")
            .expect("guard");
        drop(fresh);
        drop(first);
    }

    #[test]
    fn static_and_dynamic_request_windows_are_separate() {
        let ip = IpAddr::from([192, 0, 2, 14]);
        let mut cfg = config();
        cfg.static_requests_per_second = Some(1);
        cfg.dynamic_requests_per_second = Some(1);
        assert!(request_wait(&cfg, ip, false).is_zero());
        assert!(!request_wait(&cfg, ip, false).is_zero());
        assert!(request_wait(&cfg, ip, true).is_zero());
        assert!(!request_wait(&cfg, ip, true).is_zero());
    }

    #[test]
    fn hard_limit_aborts_bodies_admitted_before_the_refusal() {
        let ip = IpAddr::from([192, 0, 2, 20]);
        let mut cfg = soft_config();
        cfg.grace_seconds = 60;
        let before = hard_abort_epoch(ip);
        let _first = acquire_connection(&cfg, ip).expect("first").expect("guard");
        let _second = acquire_connection(&cfg, ip)
            .expect("second")
            .expect("guard");
        assert!(matches!(
            acquire_connection(&cfg, ip),
            Err(ThrottleDecision::HardLimit)
        ));
        assert!(hard_abort_changed(ip, before));
    }

    #[test]
    fn frame_is_split_at_the_initial_burst() {
        let ip = IpAddr::from([192, 0, 2, 19]);
        let first_at = Instant::now();
        let data = Bytes::from(vec![b'x'; 100]);
        let (head, tail, wait) = token_chunk(ip, data, 40, false, first_at);
        assert_eq!(head.len(), 40, "first frame must use the initial burst");
        assert_eq!(tail.map(|b| b.len()), Some(60));
        assert_eq!(wait, RATE_WINDOW);
        // After one refill interval the remainder is emitted, not held as one
        // oversized frame behind a full-body delay.
        let (head, tail, wait) = token_chunk(
            ip,
            Bytes::from(vec![b'y'; 40]),
            40,
            false,
            first_at + RATE_WINDOW,
        );
        assert_eq!(head.len(), 40);
        assert!(tail.is_none());
        assert!(wait.is_zero());
    }

    #[test]
    fn input_bandwidth_starts_with_one_second_of_tokens() {
        let ip = IpAddr::from([192, 0, 2, 15]);
        let mut cfg = config();
        cfg.input_bandwidth_bytes_per_second = Some(1024);
        // The first 1 KiB is the initial burst and is not held back.
        assert!(inbound_wait(&cfg, ip, 1024).is_zero());
        // A request far beyond the burst must wait even if scheduler noise
        // refills a few tokens between the two calls. A two-byte probe was
        // flaky for exactly that reason.
        assert!(!inbound_wait(&cfg, ip, 1_048_576).is_zero());
        assert!(inbound_wait(&cfg, ip, 0).is_zero());
    }

    #[test]
    fn malformed_ban_closes_the_next_connection() {
        let ip = IpAddr::from([192, 0, 2, 18]);
        let mut cfg = config();
        cfg.block_bad_req = true;
        cfg.ban_seconds = 1;
        let guard = acquire_connection(&cfg, ip).expect("first").expect("guard");
        note_bad_request(&cfg, ip);
        note_bad_request(&cfg, ip);
        note_bad_request(&cfg, ip);
        drop(guard);
        assert!(matches!(
            acquire_connection(&cfg, ip),
            Err(ThrottleDecision::Banned)
        ));
    }

    #[test]
    fn block_bad_req_bans_after_three_bad_requests() {
        let ip = IpAddr::from([192, 0, 2, 16]);
        let mut cfg = config();
        cfg.block_bad_req = true;
        cfg.ban_seconds = 1;
        note_bad_request(&cfg, ip);
        note_bad_request(&cfg, ip);
        assert!(acquire_connection(&cfg, ip)
            .expect("still allowed")
            .is_some());
        note_bad_request(&cfg, ip);
        assert!(matches!(
            acquire_connection(&cfg, ip),
            Err(ThrottleDecision::Banned)
        ));
    }
}
