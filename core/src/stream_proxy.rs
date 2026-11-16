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
//! NG-20: L4 TCP/UDP forwarding.
//!
//! `[[stream]]` listeners reuse an `[[upstream]]` endpoint set and its
//! weights/priority/admin state. TCP connections and UDP source addresses are
//! pinned to one selected endpoint. UDP sessions are capped per listener and
//! expire after `idle_timeout_ms`; every relay has bounded buffers and a
//! connect timeout inherited from `upstream.timeout_ms`.

use crate::lifecycle::LifecycleState;
use exyonq_config_ir::{
    AdminEndpointState, Endpoint, StreamConfig, StreamProtocol, UpstreamConfig,
};
use exyonq_mod_proxy::selector::{EndpointAdmin, EndpointSelector};
use exyonq_mod_proxy::{EndpointSpec, FailoverMode, PassivePeer, SelectionOutcome};
use std::collections::{HashMap, HashSet};
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, info, warn};

/// Largest UDP datagram buffer. The maximum UDP payload is 65,507 bytes;
/// 65,535 leaves room for a datagram received with IPv6 padding semantics.
const UDP_DATAGRAM_MAX: usize = 65_535;
/// Per-session outbound queue capacity. A slow or stuck session drops excess
/// datagrams instead of growing memory without bound.
const UDP_SESSION_QUEUE: usize = 64;
/// How often a relay checks the idle deadline.
const IDLE_TICK: Duration = Duration::from_millis(100);

/// A compiled L4 listener target table.
struct StreamPool {
    selector: Arc<EndpointSelector>,
    targets: Vec<String>,
    connect_timeout: Duration,
}

impl StreamPool {
    fn build(config: &StreamConfig, upstream: &UpstreamConfig) -> anyhow::Result<Self> {
        let endpoints = upstream.endpoint_set.endpoints();
        if endpoints.is_empty() {
            anyhow::bail!(
                "stream `{}` upstream `{}` has no endpoints",
                config.name,
                config.upstream
            );
        }
        let mut targets = Vec::with_capacity(endpoints.len());
        let mut specs = Vec::with_capacity(endpoints.len());
        for endpoint in endpoints {
            targets.push(endpoint_target(endpoint));
            specs.push(stream_endpoint_spec(endpoint));
        }
        let failover = match upstream.failover_policy {
            exyonq_config_ir::EndpointFailoverPolicy::PriorityBands => FailoverMode::PriorityBands,
            exyonq_config_ir::EndpointFailoverPolicy::None => FailoverMode::None,
        };
        let selector = EndpointSelector::build(1, failover, specs);
        Ok(Self {
            selector: Arc::new(selector),
            targets,
            connect_timeout: Duration::from_millis(upstream.timeout_ms.max(1)),
        })
    }

    fn select(&self, excluded: &HashSet<usize>) -> Option<usize> {
        match self
            .selector
            .select_filtered(|index| !excluded.contains(&index))
        {
            SelectionOutcome::Selected { endpoint_index, .. } => Some(endpoint_index),
            SelectionOutcome::NoEligibleEndpoint => None,
        }
    }

    fn target(&self, index: usize) -> Option<&str> {
        self.targets.get(index).map(String::as_str)
    }

    fn len(&self) -> usize {
        self.targets.len()
    }
}

fn endpoint_target(endpoint: &Endpoint) -> String {
    format!("{}:{}", endpoint.address.as_host_str(), endpoint.port)
}

fn stream_endpoint_spec(endpoint: &Endpoint) -> EndpointSpec {
    let enabled = endpoint.admin_state == AdminEndpointState::Enabled;
    EndpointSpec {
        endpoint_id: endpoint.endpoint_id.as_str().to_string(),
        http_uri: format!(
            "http://{}:{}",
            endpoint.address.as_host_str(),
            endpoint.port
        ),
        weight: endpoint.weight,
        priority: endpoint.priority,
        admin_enabled: enabled,
        // L4 uses the same selector tables as HTTP, but transport failures are
        // handled by trying the next endpoint for this connection. Passive HTTP
        // health state is intentionally inert on this path.
        passive: Arc::new(PassivePeer::disabled()),
        admin: Arc::new(EndpointAdmin::new(enabled, endpoint.weight)),
    }
}

/// Bind every configured L4 listener, then spawn its accept/receive loop.
///
/// All binds happen before any task is spawned, so a bad listener is a
/// startup/config error instead of a partially running L4 surface.
pub async fn start_streams(
    streams: &[StreamConfig],
    upstreams: &HashMap<String, UpstreamConfig>,
    ops: Arc<LifecycleState>,
) -> anyhow::Result<Vec<std::net::SocketAddr>> {
    let mut bound = Vec::with_capacity(streams.len());
    let mut addresses = Vec::with_capacity(streams.len());

    for config in streams {
        let upstream = upstreams.get(&config.upstream).ok_or_else(|| {
            anyhow::anyhow!(
                "stream `{}` references unknown upstream `{}`",
                config.name,
                config.upstream
            )
        })?;
        let pool = Arc::new(StreamPool::build(config, upstream)?);
        let idle = Duration::from_millis(config.idle_timeout_ms);
        match config.protocol {
            StreamProtocol::Tcp => {
                let listener = TcpListener::bind(&config.listen).await?;
                let addr = listener.local_addr()?;
                addresses.push(addr);
                bound.push(BoundStream::Tcp {
                    name: config.name.clone(),
                    listener,
                    pool,
                    idle,
                });
            }
            StreamProtocol::Udp => {
                let socket = UdpSocket::bind(&config.listen).await?;
                let addr = socket.local_addr()?;
                addresses.push(addr);
                bound.push(BoundStream::Udp {
                    name: config.name.clone(),
                    socket: Arc::new(socket),
                    pool,
                    idle,
                    max_sessions: config.max_sessions,
                });
            }
        }
    }

    for stream in bound {
        match stream {
            BoundStream::Tcp {
                name,
                listener,
                pool,
                idle,
            } => {
                info!(
                    stream = %name,
                    listen = %listener.local_addr().map(|a| a.to_string()).unwrap_or_default(),
                    "L4 TCP stream listening"
                );
                tokio::spawn(run_tcp_listener(
                    name,
                    listener,
                    pool,
                    idle,
                    Arc::clone(&ops),
                ));
            }
            BoundStream::Udp {
                name,
                socket,
                pool,
                idle,
                max_sessions,
            } => {
                info!(
                    stream = %name,
                    listen = %socket.local_addr().map(|a| a.to_string()).unwrap_or_default(),
                    "L4 UDP stream listening"
                );
                tokio::spawn(run_udp_listener(
                    name,
                    socket,
                    pool,
                    idle,
                    max_sessions,
                    Arc::clone(&ops),
                ));
            }
        }
    }

    Ok(addresses)
}

enum BoundStream {
    Tcp {
        name: String,
        listener: TcpListener,
        pool: Arc<StreamPool>,
        idle: Duration,
    },
    Udp {
        name: String,
        socket: Arc<UdpSocket>,
        pool: Arc<StreamPool>,
        idle: Duration,
        max_sessions: usize,
    },
}

async fn run_tcp_listener(
    name: String,
    listener: TcpListener,
    pool: Arc<StreamPool>,
    idle: Duration,
    ops: Arc<LifecycleState>,
) {
    let mut shutdown = ops.shutdown_rx();
    loop {
        let accepted = tokio::select! {
            result = listener.accept() => result,
            changed = shutdown.changed() => {
                if changed.is_err() || ops.shutdown_requested() {
                    break;
                }
                continue;
            }
        };
        let (client, peer) = match accepted {
            Ok(pair) => pair,
            Err(err) => {
                warn!(stream = %name, %err, "L4 TCP accept failed");
                continue;
            }
        };
        let token = match ops.try_enter() {
            Ok(token) => token,
            Err(_) => break,
        };
        let pool = Arc::clone(&pool);
        let ops = Arc::clone(&ops);
        let name = name.clone();
        tokio::spawn(async move {
            relay_tcp(name, client, peer, pool, idle, ops, token).await;
        });
    }
}

async fn relay_tcp(
    name: String,
    client: TcpStream,
    peer: std::net::SocketAddr,
    pool: Arc<StreamPool>,
    idle: Duration,
    ops: Arc<LifecycleState>,
    _token: crate::lifecycle::ConnectionLifecycleToken,
) {
    if ops.shutdown_requested() {
        return;
    }
    let Ok(()) = client.set_nodelay(true) else {
        return;
    };
    let mut excluded = HashSet::new();
    let mut upstream = None;
    for _ in 0..pool.len() {
        let Some(index) = pool.select(&excluded) else {
            break;
        };
        let Some(target) = pool.target(index) else {
            break;
        };
        match connect_tcp(target, pool.connect_timeout).await {
            Ok(stream) => {
                upstream = Some(stream);
                break;
            }
            Err(err) => {
                debug!(stream = %name, %peer, target, %err, "L4 TCP upstream connect failed");
                excluded.insert(index);
            }
        }
    }
    let Some(upstream) = upstream else {
        debug!(stream = %name, %peer, "L4 TCP no reachable upstream");
        return;
    };
    let activity = Arc::new(Activity::new());
    let mut client = ActivityStream::new(client, Arc::clone(&activity));
    let mut upstream = ActivityStream::new(upstream, Arc::clone(&activity));
    let relay = tokio::io::copy_bidirectional(&mut client, &mut upstream);
    tokio::pin!(relay);
    let mut tick = tokio::time::interval(IDLE_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut shutdown = ops.shutdown_rx();
    loop {
        tokio::select! {
            result = &mut relay => {
                if let Err(err) = result {
                    debug!(stream = %name, %peer, %err, "L4 TCP relay ended with error");
                }
                break;
            }
            _ = tick.tick() => {
                if activity.idle_for() >= idle {
                    debug!(stream = %name, %peer, "L4 TCP relay idle timeout");
                    break;
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || ops.shutdown_requested() {
                    break;
                }
            }
        }
    }
}

async fn connect_tcp(target: &str, timeout: Duration) -> io::Result<TcpStream> {
    match tokio::time::timeout(timeout, TcpStream::connect(target)).await {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "L4 TCP connect timed out",
        )),
    }
}

struct UdpSession {
    id: u64,
    tx: mpsc::Sender<Vec<u8>>,
}

struct UdpRelay {
    name: String,
    client: std::net::SocketAddr,
    id: u64,
    listener: Arc<UdpSocket>,
    sessions: Arc<Mutex<HashMap<std::net::SocketAddr, UdpSession>>>,
    pool: Arc<StreamPool>,
    idle: Duration,
    ops: Arc<LifecycleState>,
}

async fn run_udp_listener(
    name: String,
    socket: Arc<UdpSocket>,
    pool: Arc<StreamPool>,
    idle: Duration,
    max_sessions: usize,
    ops: Arc<LifecycleState>,
) {
    let sessions: Arc<Mutex<HashMap<std::net::SocketAddr, UdpSession>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let mut next_id = 0u64;
    let mut shutdown = ops.shutdown_rx();
    let mut buf = vec![0u8; UDP_DATAGRAM_MAX];
    loop {
        let received = tokio::select! {
            result = socket.recv_from(&mut buf) => result,
            changed = shutdown.changed() => {
                if changed.is_err() || ops.shutdown_requested() {
                    break;
                }
                continue;
            }
        };
        let (len, peer) = match received {
            Ok(pair) => pair,
            Err(err) => {
                debug!(stream = %name, %err, "L4 UDP receive failed");
                continue;
            }
        };
        if len == 0 {
            continue;
        }
        let data = buf[..len].to_vec();
        let mut new_session = None;
        let tx = {
            let mut guard = sessions.lock().expect("L4 UDP session map poisoned");
            if let Some(session) = guard.get(&peer) {
                session.tx.clone()
            } else {
                if guard.len() >= max_sessions {
                    warn!(
                        stream = %name,
                        max_sessions,
                        "L4 UDP session cap reached; dropping datagram"
                    );
                    continue;
                }
                let id = next_id;
                next_id = next_id.wrapping_add(1);
                let (tx, rx) = mpsc::channel::<Vec<u8>>(UDP_SESSION_QUEUE);
                guard.insert(peer, UdpSession { id, tx: tx.clone() });
                new_session = Some((id, rx));
                tx
            }
        };
        if let Some((id, rx)) = new_session {
            let token = match ops.try_enter() {
                Ok(token) => token,
                Err(_) => break,
            };
            let relay = UdpRelay {
                name: name.clone(),
                client: peer,
                id,
                listener: Arc::clone(&socket),
                sessions: Arc::clone(&sessions),
                pool: Arc::clone(&pool),
                idle,
                ops: Arc::clone(&ops),
            };
            tokio::spawn(async move {
                relay_udp_session(relay, rx, token).await;
            });
        }
        if tx.try_send(data).is_err() {
            debug!(stream = %name, %peer, "L4 UDP session queue full");
        }
    }
}

async fn relay_udp_session(
    relay: UdpRelay,
    mut rx: mpsc::Receiver<Vec<u8>>,
    _token: crate::lifecycle::ConnectionLifecycleToken,
) {
    let UdpRelay {
        name,
        client,
        id,
        listener,
        sessions,
        pool,
        idle,
        ops,
    } = relay;
    let Some((upstream, _endpoint)) = open_udp_upstream(&pool, &mut HashSet::new()).await else {
        remove_udp_session(&sessions, client, id);
        return;
    };
    let mut shutdown = ops.shutdown_rx();
    let mut buf = vec![0u8; UDP_DATAGRAM_MAX];
    let mut last = Instant::now();
    loop {
        let deadline = last + idle;
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            changed = shutdown.changed() => {
                if changed.is_err() || ops.shutdown_requested() {
                    break;
                }
            }
            maybe = rx.recv() => {
                let Some(data) = maybe else { break };
                if let Err(err) = upstream.send(&data).await {
                    debug!(stream = %name, %client, %err, "L4 UDP upstream send failed");
                    break;
                }
                last = Instant::now();
            }
            received = upstream.recv(&mut buf) => {
                match received {
                    Ok(n) if n > 0 => {
                        if let Err(err) = listener.send_to(&buf[..n], client).await {
                            debug!(stream = %name, %client, %err, "L4 UDP client send failed");
                            break;
                        }
                        last = Instant::now();
                    }
                    Ok(_) => {}
                    Err(err) => {
                        debug!(stream = %name, %client, %err, "L4 UDP upstream receive failed");
                        break;
                    }
                }
            }
        }
    }
    remove_udp_session(&sessions, client, id);
}

fn remove_udp_session(
    sessions: &Mutex<HashMap<std::net::SocketAddr, UdpSession>>,
    client: std::net::SocketAddr,
    id: u64,
) {
    let mut guard = sessions.lock().expect("L4 UDP session map poisoned");
    if guard.get(&client).is_some_and(|session| session.id == id) {
        guard.remove(&client);
    }
}

async fn open_udp_upstream(
    pool: &StreamPool,
    excluded: &mut HashSet<usize>,
) -> Option<(UdpSocket, usize)> {
    for _ in 0..pool.len() {
        let index = pool.select(excluded)?;
        let target = pool.target(index)?;
        if let Ok(socket) = bind_udp_target(target, pool.connect_timeout).await {
            return Some((socket, index));
        }
        excluded.insert(index);
    }
    None
}

async fn bind_udp_target(target: &str, timeout: Duration) -> io::Result<UdpSocket> {
    let addrs = tokio::time::timeout(timeout, tokio::net::lookup_host(target))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "L4 UDP lookup timed out"))??;
    let mut last = io::Error::new(io::ErrorKind::AddrNotAvailable, "no UDP address resolved");
    for addr in addrs {
        let bind = if addr.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind).await?;
        match socket.connect(addr).await {
            Ok(()) => return Ok(socket),
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// Last-activity timestamp shared by both directions of a TCP relay.
struct Activity {
    start: Instant,
    last_ms: AtomicU64,
}

impl Activity {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }

    fn touch(&self) {
        let elapsed = self.start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        self.last_ms.store(elapsed, Ordering::Relaxed);
    }

    fn idle_for(&self) -> Duration {
        let elapsed = self.start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let last = self.last_ms.load(Ordering::Relaxed);
        Duration::from_millis(elapsed.saturating_sub(last))
    }
}

/// `AsyncRead`/`AsyncWrite` adapter that records successful data movement.
struct ActivityStream<S> {
    inner: S,
    activity: Arc<Activity>,
}

impl<S> ActivityStream<S> {
    fn new(inner: S, activity: Arc<Activity>) -> Self {
        Self { inner, activity }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ActivityStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.activity.touch();
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ActivityStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.activity.touch();
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
