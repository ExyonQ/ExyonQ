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

pub(crate) use crate::kernel::handoff::HyperHandoff;
use crate::lifecycle::{self, LifecycleState};
use crate::reload::{self, SharedServerState};
use crate::server::wire_dispatch::{
    dispatch_tcp_inner, dispatch_tcp_prefixed_inner, WireDispatchContext,
};
use exyonq_mod_proxy::ProxyClient;
use hyper::header::HeaderValue;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream as TokioTcpStream;
use tokio::runtime::Handle;

/// **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**
pub fn xff_from_peer(peer: SocketAddr) -> HeaderValue {
    HeaderValue::from_str(&peer.ip().to_string())
        .unwrap_or_else(|_| HeaderValue::from_static("0.0.0.0"))
}

/// **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**
pub fn spawn_hyper_handoff(
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
            proxy,
            &ops,
            rt.handle(),
        )
        .expect("spawn");
        assert_eq!(xff_before.to_str().unwrap(), peer.ip().to_string());
        assert_eq!(ops.active_connections(), 1);
        rt.block_on(async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        });
        assert_eq!(ops.active_connections(), 0);
    }
}
