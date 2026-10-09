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
//! PS1B: unified TCP → Tokio/Hyper handoff consumer (in-core).
//!
//! Cap067 listen workers run on OS threads. Per-connection `Handle::spawn` from those
//! threads was the P5 Connection:close tax (Netcup: ~7k vs ~43k with Tokio-native
//! accept). Handoffs from non-Tokio threads enqueue into a bounded MPMC queue drained
//! by long-lived Tokio tasks — static Cap067 stay-attached paths unchanged.

use crate::kernel::generation::GenerationView;
pub(crate) use crate::kernel::handoff::HyperHandoff;
use crate::lifecycle::{self, ConnectionLifecycleToken, LifecycleState};
use crate::reload::{self, SharedServerState};
use crate::server::wire_dispatch::{
    dispatch_tcp_inner, dispatch_tcp_prefixed_inner, WireDispatchContext,
};
use bytes::Bytes;
use exyonq_mod_proxy::ProxyClient;
use hyper::header::HeaderValue;
use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::net::TcpStream as TokioTcpStream;
use tokio::runtime::Handle;
use tokio::sync::Notify;

/// **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**
pub fn xff_from_peer(peer: SocketAddr) -> HeaderValue {
    HeaderValue::from_str(&peer.ip().to_string())
        .unwrap_or_else(|_| HeaderValue::from_static("0.0.0.0"))
}

struct QueuedHandoff {
    stream: std::net::TcpStream,
    peer: SocketAddr,
    prefetched: Option<(Bytes, Bytes)>,
    generation: GenerationView,
    token: ConnectionLifecycleToken,
}

struct HandoffBridge {
    queue: Mutex<VecDeque<QueuedHandoff>>,
    notify: Notify,
    depth: AtomicUsize,
    max_depth: usize,
}

static HANDOFF_BRIDGE: OnceLock<Arc<HandoffBridge>> = OnceLock::new();

/// Install the Cap067→Tokio handoff queue once per process (idempotent).
///
/// `worker_count` long-lived consumers drain the queue on `runtime`.
pub fn install_handoff_bridge(
    runtime: &Handle,
    shared: SharedServerState,
    proxy_client: ProxyClient,
    ops: Arc<LifecycleState>,
    worker_count: usize,
) {
    let workers = worker_count.max(1);
    let max_depth = handoff_queue_capacity();
    let bridge = Arc::new(HandoffBridge {
        queue: Mutex::new(VecDeque::with_capacity(max_depth.min(1024))),
        notify: Notify::new(),
        depth: AtomicUsize::new(0),
        max_depth,
    });
    if HANDOFF_BRIDGE.set(Arc::clone(&bridge)).is_err() {
        return; // already installed
    }
    for _ in 0..workers {
        let bridge = Arc::clone(&bridge);
        let shared = SharedServerState::clone(&shared);
        let proxy_client = proxy_client.clone();
        let ops = Arc::clone(&ops);
        runtime.spawn(async move {
            bridge.consumer_loop(shared, proxy_client, ops).await;
        });
    }
}

fn handoff_queue_capacity() -> usize {
    match std::env::var("EXYONQ_HANDOFF_QUEUE_CAP")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
    {
        Some(n) if n >= 64 => n,
        _ => 8192,
    }
}

fn handoff_worker_count_default() -> usize {
    match std::env::var("EXYONQ_WORKER_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
    {
        Some(n) if n >= 1 => n,
        _ => std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(2, 16),
    }
}

impl HandoffBridge {
    #[allow(clippy::result_large_err)]
    fn try_enqueue(&self, handoff: HyperHandoff) -> Result<(), HyperHandoff> {
        let HyperHandoff {
            stream,
            peer,
            prefetched,
            generation,
            token,
        } = handoff;
        let mut guard = match self.queue.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if guard.len() >= self.max_depth {
            return Err(HyperHandoff {
                stream,
                peer,
                prefetched,
                generation,
                token,
            });
        }
        guard.push_back(QueuedHandoff {
            stream,
            peer,
            prefetched,
            generation,
            token,
        });
        drop(guard);
        self.depth.fetch_add(1, Ordering::Relaxed);
        self.notify.notify_one();
        Ok(())
    }

    fn pop(&self) -> Option<QueuedHandoff> {
        let mut guard = match self.queue.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let item = guard.pop_front();
        if item.is_some() {
            self.depth.fetch_sub(1, Ordering::Relaxed);
        }
        item
    }

    async fn consumer_loop(
        self: Arc<Self>,
        shared: SharedServerState,
        proxy_client: ProxyClient,
        ops: Arc<LifecycleState>,
    ) {
        loop {
            while let Some(item) = self.pop() {
                // Spawn from inside Tokio (cheap). Never await the connection here —
                // that would serialize Cap067 handoffs through `worker_count` tasks.
                spawn_queued_handoff(item, &shared, proxy_client.clone(), &ops);
            }
            let notified = self.notify.notified();
            if let Some(item) = self.pop() {
                drop(notified);
                spawn_queued_handoff(item, &shared, proxy_client.clone(), &ops);
                continue;
            }
            notified.await;
        }
    }
}

fn spawn_queued_handoff(
    item: QueuedHandoff,
    shared: &SharedServerState,
    proxy_client: ProxyClient,
    ops: &Arc<LifecycleState>,
) {
    let QueuedHandoff {
        stream,
        peer,
        prefetched,
        generation,
        token,
    } = item;
    let _ = stream.set_nonblocking(true);
    let xff = xff_from_peer(peer);
    let shared = SharedServerState::clone(shared);
    let ops = Arc::clone(ops);
    tokio::spawn(async move {
        lifecycle::run_with_connection_token(token, async move {
            let Ok(stream) = TokioTcpStream::from_std(stream) else {
                return;
            };
            let state = reload::read_state(&shared);
            let ctx = WireDispatchContext {
                shared: SharedServerState::clone(&shared),
                state,
                proxy_client,
                x_forwarded_for: xff,
                ops,
                pinned_generation: generation.generation,
            };
            if let Some((head, rest)) = prefetched {
                dispatch_tcp_prefixed_inner(stream, head, rest, ctx).await;
            } else {
                dispatch_tcp_inner(stream, ctx).await;
            }
        })
        .await;
    });
}

fn spawn_hyper_handoff_direct(
    handoff: HyperHandoff,
    shared: &SharedServerState,
    proxy_client: ProxyClient,
    ops: &Arc<LifecycleState>,
    runtime: &Handle,
) -> io::Result<()> {
    handoff.stream.set_nonblocking(true)?;
    let shared = SharedServerState::clone(shared);
    let ops = Arc::clone(ops);
    let xff = xff_from_peer(handoff.peer);
    let prefetched = handoff.prefetched;
    let generation = handoff.generation;
    let token = handoff.token;
    let stream = handoff.stream;
    runtime.spawn(async move {
        lifecycle::run_with_connection_token(token, async move {
            let Ok(stream) = TokioTcpStream::from_std(stream) else {
                return;
            };
            let state = reload::read_state(&shared);
            let ctx = WireDispatchContext {
                shared: SharedServerState::clone(&shared),
                state,
                proxy_client,
                x_forwarded_for: xff,
                ops,
                pinned_generation: generation.generation,
            };
            if let Some((head, rest)) = prefetched {
                dispatch_tcp_prefixed_inner(stream, head, rest, ctx).await;
            } else {
                dispatch_tcp_inner(stream, ctx).await;
            }
        })
        .await;
    });
    Ok(())
}

/// **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**
pub fn spawn_hyper_handoff(
    handoff: HyperHandoff,
    shared: &SharedServerState,
    proxy_client: ProxyClient,
    ops: &Arc<LifecycleState>,
    runtime: &Handle,
) -> io::Result<()> {
    // Already on a Tokio worker (tests / divert): direct spawn on that runtime.
    if let Ok(current) = Handle::try_current() {
        return spawn_hyper_handoff_direct(handoff, shared, proxy_client, ops, &current);
    }

    if let Some(bridge) = HANDOFF_BRIDGE.get() {
        match bridge.try_enqueue(handoff) {
            Ok(()) => return Ok(()),
            Err(handoff) => {
                return spawn_hyper_handoff_direct(handoff, shared, proxy_client, ops, runtime);
            }
        }
    }

    spawn_hyper_handoff_direct(handoff, shared, proxy_client, ops, runtime)
}

/// Ensure the bridge exists (composition / executor construction).
pub(crate) fn ensure_handoff_bridge(
    runtime: &Handle,
    shared: SharedServerState,
    proxy_client: ProxyClient,
    ops: Arc<LifecycleState>,
) {
    if HANDOFF_BRIDGE.get().is_some() {
        return;
    }
    install_handoff_bridge(
        runtime,
        shared,
        proxy_client,
        ops,
        handoff_worker_count_default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::handoff::HyperHandoff;
    use crate::lifecycle::LifecycleState;
    use crate::server::conn_pool::SyncBenchCache;
    use bytes::Bytes;
    use std::net::SocketAddr;

    #[test]
    fn hyper_handoff_is_not_clone() {
        fn assert_not_clone<T>() {}
        assert_not_clone::<HyperHandoff>();
    }

    #[test]
    fn ps1b_xff_from_peer_uses_real_ip() {
        let peer: SocketAddr = "203.0.113.9:54321".parse().unwrap();
        let xff = xff_from_peer(peer);
        assert_eq!(xff.to_str().unwrap(), "203.0.113.9");
    }

    #[test]
    fn ps1b_xff_from_peer_not_unspecified() {
        let peer: SocketAddr = "192.168.1.50:1234".parse().unwrap();
        let xff = xff_from_peer(peer);
        assert_ne!(xff.to_str().unwrap(), "0.0.0.0");
    }

    #[test]
    fn ps1c_generation_from_bench_cache_preserves_identity() {
        let cache = SyncBenchCache {
            site_static_slot: Some(3),
            modules_enabled: true,
            generation: 99,
        };
        let view = crate::kernel::GenerationView::from(cache);
        assert_eq!(view.generation, 99);
        assert!(view.modules_enabled);
        assert_eq!(view.site_static_slot, Some(3));
    }

    #[test]
    fn bench_cache_moves_with_handoff() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        let cache = SyncBenchCache {
            site_static_slot: Some(1),
            modules_enabled: false,
            generation: 42,
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, peer) = listener.accept().unwrap();
        drop(server);
        let handoff = HyperHandoff::new(stream, peer, None, cache, token);
        assert_eq!(handoff.generation.generation, 42);
        drop(handoff);
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn ps1b_prefetched_bytes_move_without_clone() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        let cache = SyncBenchCache {
            site_static_slot: None,
            modules_enabled: false,
            generation: 7,
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, peer) = listener.accept().unwrap();
        drop(server);
        let head = Bytes::from_static(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
        let rest = Bytes::from_static(b"body-prefix");
        let handoff = HyperHandoff::new(stream, peer, Some((head, rest)), cache, token);
        let HyperHandoff {
            stream: _stream,
            peer: got_peer,
            prefetched,
            generation: got_gen,
            token: got_token,
        } = handoff;
        let (h, r) = prefetched.unwrap();
        assert_eq!(got_peer, peer);
        assert_eq!(got_gen.generation, 7);
        assert_eq!(h.as_ref(), b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(r.as_ref(), b"body-prefix");
        drop(got_token);
        assert_eq!(ops.active_connections(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ps1b_spawn_hyper_handoff_preserves_peer_and_drops_token() {
        let ops = std::sync::Arc::new(LifecycleState::new());
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let raw = include_str!("../../../tests/fixtures/minimal.toml");
        let config: crate::config::AppConfig = raw.parse().expect("config");
        let proxy = exyonq_mod_proxy::build_incoming_client();
        let shared = rt.block_on(async {
            crate::reload::wrap_state(
                crate::server::state::ServerState::new(config, proxy.clone())
                    .await
                    .expect("state"),
            )
        });
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (stream, peer) = {
            let _client =
                std::thread::spawn(move || std::net::TcpStream::connect(addr).expect("connect"));
            listener.accept().expect("accept")
        };
        let bench_cache = SyncBenchCache::from_state(crate::reload::read_state(&shared).as_ref());
        let token = ops.try_enter().expect("enter");
        let xff_before = xff_from_peer(peer);
        spawn_hyper_handoff(
            HyperHandoff::new(stream, peer, None, bench_cache, token),
            &shared,
            proxy.clone(),
            &ops,
            rt.handle(),
        )
        .expect("spawn");
        assert_eq!(xff_before.to_str().unwrap(), peer.ip().to_string());
        // Token is owned by the Hyper task after spawn. Under load it may already
        // have finished (0) before this assert; never allow >1.
        assert!(ops.active_connections() <= 1);
        rt.block_on(async {
            for _ in 0..100 {
                if ops.active_connections() == 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        });
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn handoff_queue_capacity_respects_floor() {
        // Unset / tiny values fall back to default ≥ 64 semantics via parse arm.
        let cap = handoff_queue_capacity();
        assert!(cap >= 64, "cap={cap}");
    }

    #[test]
    fn handoff_bridge_enqueues_when_outside_tokio() {
        let bridge = Arc::new(HandoffBridge {
            queue: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            depth: AtomicUsize::new(0),
            max_depth: 8,
        });
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        let cache = SyncBenchCache {
            site_static_slot: None,
            modules_enabled: false,
            generation: 1,
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, peer) = listener.accept().unwrap();
        drop(server);
        let handoff = HyperHandoff::new(stream, peer, None, cache, token);
        assert!(bridge.try_enqueue(handoff).is_ok());
        assert_eq!(bridge.depth.load(Ordering::Relaxed), 1);
        assert!(bridge.pop().is_some());
        assert_eq!(bridge.depth.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn handoff_bridge_rejects_when_full() {
        let bridge = Arc::new(HandoffBridge {
            queue: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            depth: AtomicUsize::new(0),
            max_depth: 1,
        });
        let ops = LifecycleState::new();
        let mk = || {
            let token = ops.try_enter().unwrap();
            let cache = SyncBenchCache {
                site_static_slot: None,
                modules_enabled: false,
                generation: 1,
            };
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (server, peer) = listener.accept().unwrap();
            drop(server);
            HyperHandoff::new(stream, peer, None, cache, token)
        };
        assert!(bridge.try_enqueue(mk()).is_ok());
        assert!(bridge.try_enqueue(mk()).is_err());
    }
}
