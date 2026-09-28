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
//! OS-thread worker guard returned by registered Linux platform starters (PS3A-PM1).
//!
//! **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**

use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Join/shutdown bundle for accept workers started outside `exyonq-core`.
pub struct OsWorkerGuard {
    shutdown: Arc<AtomicBool>,
    /// Cap041: force worker exit even with in-map connections (after grace wait).
    abort: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
    listeners: Vec<TcpListener>,
    /// Shutdown-only retain (e.g. `Arc<ConnectionPool>`). Not used on the accept hot path.
    retain: Vec<Arc<dyn Send + Sync>>,
}

impl OsWorkerGuard {
    pub fn new(
        shutdown: Arc<AtomicBool>,
        handles: Vec<JoinHandle<()>>,
        listeners: Vec<TcpListener>,
    ) -> Self {
        Self::new_with_abort(
            shutdown,
            Arc::new(AtomicBool::new(false)),
            handles,
            listeners,
        )
    }

    pub fn new_with_abort(
        shutdown: Arc<AtomicBool>,
        abort: Arc<AtomicBool>,
        handles: Vec<JoinHandle<()>>,
        listeners: Vec<TcpListener>,
    ) -> Self {
        Self {
            shutdown,
            abort,
            handles,
            listeners,
            retain: Vec::new(),
        }
    }

    pub fn retain_arc<T: Send + Sync + 'static>(&mut self, value: Arc<T>) {
        self.retain.push(value);
    }

    /// Cap041: stop new accepts; workers keep serving in-map connections until empty or [`Self::stop`].
    pub fn signal_stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Force-abort remaining in-map work (if any) and join worker threads.
    pub fn stop(mut self) {
        self.abort.store(true, Ordering::SeqCst);
        self.shutdown.store(true, Ordering::SeqCst);
        self.listeners.clear();
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
        self.retain.clear();
    }
}
