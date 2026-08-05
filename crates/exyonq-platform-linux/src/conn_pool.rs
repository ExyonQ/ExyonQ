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
//! OS thread pool for sync_accept (PS3A-PM1). No bench-cache / policy state.

use exyonq_core::kernel::{AcceptedConnection, PlatformConnectionEntry};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// Pool job: owned connection + opaque platform entry (no SharedServerState / ProxyClient).
pub struct ConnJob {
    pub conn: AcceptedConnection,
    pub entry: Arc<PlatformConnectionEntry>,
}

pub struct ConnectionPool {
    tx: SyncSender<ConnJob>,
    _handles: Vec<JoinHandle<()>>,
}

fn pool_size() -> usize {
    std::env::var("EXYONQ_CONN_POOL_SIZE")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(64)
        .clamp(1, 512)
}

fn queue_capacity() -> usize {
    std::env::var("EXYONQ_CONN_POOL_QUEUE")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(256)
        .clamp(16, 4096)
}

impl ConnectionPool {
    pub fn spawn<F>(handler: Arc<F>) -> Self
    where
        F: Fn(ConnJob) + Send + Sync + 'static,
    {
        let (tx, rx) = mpsc::sync_channel(queue_capacity());
        let rx = Arc::new(Mutex::new(rx));
        let size = pool_size();
        let mut handles = Vec::with_capacity(size);
        for _ in 0..size {
            let rx = Arc::clone(&rx);
            let handler = Arc::clone(&handler);
            handles.push(thread::spawn(move || loop {
                let job = match rx.lock() {
                    Ok(guard) => match guard.recv() {
                        Ok(job) => job,
                        Err(_) => break,
                    },
                    Err(poison) => {
                        drop(poison.into_inner());
                        break;
                    }
                };
                handler(job);
            }));
        }
        Self {
            tx,
            _handles: handles,
        }
    }

    pub fn sender(&self) -> SyncSender<ConnJob> {
        self.tx.clone()
    }
}
