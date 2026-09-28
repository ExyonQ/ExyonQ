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
//! SDP-P1 listen ownership seam (default OFF via `EXYONQ_SDP_P1=1`).
//! Linux-first (Netcup qualification). mio itself is portable; handoff uses Linux wire paths.

use crate::lifecycle::{self, LifecycleState};
use crate::reload::{self, SharedServerState};
use crate::server::state::ServerState;
use crate::server::wire_dispatch::{
    dispatch_tcp_inner, dispatch_tcp_prefixed_inner, WireDispatchContext,
};
use arc_swap::ArcSwap;
use bytes::Bytes;
use exyonq_mod_proxy::ProxyClient;
use exyonq_proxy_sdp::{
    build_projection_from_parts, resolve_shard_count, start_sdp_listen, GenerationProjection,
    SdpListenHandle, SharedProjection,
};
use hyper::header::HeaderValue;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use tokio::net::TcpStream as TokioTcpStream;
use tokio::runtime::Handle;
use tracing::info;

static LIVE_PROJECTION: OnceLock<SharedProjection> = OnceLock::new();

pub use exyonq_proxy_sdp::sdp_p1_env_enabled as env_enabled;

pub fn projection_from_state(state: &ServerState) -> Arc<GenerationProjection> {
    build_projection_from_parts(
        state.generation,
        Arc::clone(&state.snapshot),
        state.route_index.clone(),
        Arc::clone(&state.waf),
        state.waf_enforce,
        state.waf_wire_inspection_active,
    )
}

pub fn publish_projection(state: &ServerState) {
    let proj = projection_from_state(state);
    if let Some(slot) = LIVE_PROJECTION.get() {
        slot.store(proj);
    }
}

pub fn ensure_projection_slot(initial: Arc<GenerationProjection>) -> SharedProjection {
    LIVE_PROJECTION
        .get_or_init(|| Arc::new(ArcSwap::from(initial)))
        .clone()
}

/// Start SDP listen; Tokio accept must not bind the same address while this runs.
pub fn start_sdp(
    listen: SocketAddr,
    shared: SharedServerState,
    proxy_client: ProxyClient,
    ops: Arc<LifecycleState>,
    runtime: Handle,
) -> io::Result<SdpListenHandle> {
    let state = reload::read_state(&shared);
    let initial = projection_from_state(&state);
    let projection = ensure_projection_slot(initial);
    publish_projection(&reload::read_state(&shared));

    let shards =
        resolve_shard_count(crate::server::runtime_parallelism::default_runtime_parallelism());
    info!(
        %listen,
        shards,
        "sdp-p1 enabled — specialized dataplane owns accept"
    );

    let handoff = {
        let shared = SharedServerState::clone(&shared);
        let ops = Arc::clone(&ops);
        let runtime = runtime.clone();
        let proxy_client = proxy_client.clone();
        Arc::new(move |stream: std::net::TcpStream, peek: Vec<u8>| {
            let shared = SharedServerState::clone(&shared);
            let ops = Arc::clone(&ops);
            let runtime = runtime.clone();
            let proxy_client = proxy_client.clone();
            let _ = stream.set_nonblocking(true);
            let peer = stream
                .peer_addr()
                .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
            let xff = HeaderValue::from_str(&peer.ip().to_string())
                .unwrap_or_else(|_| HeaderValue::from_static("0.0.0.0"));
            runtime.spawn(async move {
                let Ok(token) = lifecycle::admit_connection(&ops) else {
                    return;
                };
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
                        pinned_generation: reload::active_runtime_generation(),
                    };
                    if peek.is_empty() {
                        dispatch_tcp_inner(stream, ctx).await;
                    } else {
                        dispatch_tcp_prefixed_inner(stream, Bytes::from(peek), Bytes::new(), ctx)
                            .await;
                    }
                })
                .await;
            });
        })
    };

    start_sdp_listen(exyonq_proxy_sdp::SdpStartConfig {
        listen,
        shard_count: shards,
        projection,
        handoff,
    })
}
