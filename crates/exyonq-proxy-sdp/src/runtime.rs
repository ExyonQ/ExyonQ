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

use crate::obs::DataplaneCounters;
use crate::projection::SharedProjection;
use crate::shard::{self, ShardArgs};
use arc_swap::ArcSwap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// `EXYONQ_SDP_P1=1` enables specialized dataplane listen ownership (default OFF).
pub fn sdp_p1_env_enabled() -> bool {
    static ENABLED: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);
    let cached = ENABLED.load(Ordering::Relaxed);
    if cached >= 0 {
        return cached != 0;
    }
    let v = std::env::var("EXYONQ_SDP_P1").ok().as_deref() == Some("1");
    let encoded: i8 = if v { 1 } else { 0 };
    let _ = ENABLED.compare_exchange(-1, encoded, Ordering::Relaxed, Ordering::Relaxed);
    ENABLED.load(Ordering::Relaxed) != 0
}

/// Shard count: `EXYONQ_SDP_SHARDS` or derived from available parallelism (no magic global 4).
pub fn resolve_shard_count(hint_cpus: usize) -> usize {
    if let Ok(raw) = std::env::var("EXYONQ_SDP_SHARDS") {
        if let Ok(n) = raw.trim().parse::<usize>() {
            return n.clamp(1, 128);
        }
    }
    let cpus = hint_cpus.max(1);
    // Prefer ≤ CPUs; on 8-CPU EP-C host, 4 matched best — derive as cpus/2 when cpus≥4.
    if cpus >= 4 {
        (cpus / 2).clamp(1, 128)
    } else {
        cpus.clamp(1, 128)
    }
}

pub type HandoffFn = Arc<dyn Fn(std::net::TcpStream, Vec<u8>) + Send + Sync>;

pub struct SdpStartConfig {
    pub listen: SocketAddr,
    pub shard_count: usize,
    pub projection: SharedProjection,
    pub handoff: HandoffFn,
}

pub struct SdpListenHandle {
    stop: Arc<AtomicBool>,
    joins: Vec<JoinHandle<()>>,
    pub counters: Arc<DataplaneCounters>,
}

impl SdpListenHandle {
    pub fn signal_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn join(self) {
        self.signal_stop();
        for j in self.joins {
            let _ = j.join();
        }
    }

    pub fn counters_snapshot(&self) -> crate::obs::PathIdentity {
        self.counters.snapshot()
    }
}

/// Start SO_REUSEPORT mio shards. Caller must not also Tokio-accept the same bind.
pub fn start_sdp_listen(cfg: SdpStartConfig) -> io::Result<SdpListenHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(DataplaneCounters::default());
    let mut joins = Vec::with_capacity(cfg.shard_count);
    for shard_id in 0..cfg.shard_count {
        let stop = Arc::clone(&stop);
        let projection = Arc::clone(&cfg.projection);
        let counters = Arc::clone(&counters);
        let handoff = Arc::clone(&cfg.handoff);
        let listen = cfg.listen;
        let join = std::thread::Builder::new()
            .name(format!("exyonq-sdp-{shard_id}"))
            .spawn(move || {
                let args = ShardArgs {
                    shard_id,
                    listen,
                    projection,
                    counters,
                    stop,
                    handoff,
                };
                if let Err(e) = shard::run_shard(args) {
                    tracing::error!(shard_id, error = %e, "sdp shard exited");
                }
            })?;
        joins.push(join);
    }
    tracing::info!(
        shards = cfg.shard_count,
        %cfg.listen,
        "sdp-p1 listen started (Tokio accept must be skipped for this bind)"
    );
    Ok(SdpListenHandle {
        stop,
        joins,
        counters,
    })
}

/// Helper: empty shared projection placeholder (tests).
pub fn empty_shared_projection_for_tests(
    proj: Arc<crate::projection::GenerationProjection>,
) -> SharedProjection {
    Arc::new(ArcSwap::from(proj))
}
