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
//! Composition seam: register sync_accept starter from `exyonq-platform-linux` (D1-safe).
//!
//! **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**
//!
//! One process-start `fn` pointer — not a per-connection callback registry.
//! Hot path remains: platform accept loop → `PlatformConnectionEntry::serve_accepted_connection`.

/// Env gate for the sync-accept listener path (mechanism flag only).
pub fn sync_accept_env_enabled(tls_on_listen: bool) -> bool {
    !tls_on_listen && std::env::var("EXYONQ_SYNC_ACCEPT").ok().as_deref() == Some("1")
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::kernel::PlatformConnectionEntry;
    use crate::server::os_worker_guard::OsWorkerGuard;
    use std::io;
    use std::net::SocketAddr;
    use std::sync::{Arc, OnceLock};

    /// Sync std entry installed by the platform crate at composition root.
    pub type SyncAcceptStartFn =
        fn(SocketAddr, usize, Arc<PlatformConnectionEntry>) -> io::Result<OsWorkerGuard>;

    static SYNC_ACCEPT_START: OnceLock<SyncAcceptStartFn> = OnceLock::new();

    /// Called once from `exyonq-platform-linux` via the CLI composition root.
    pub fn register_sync_accept_start(start: SyncAcceptStartFn) {
        let _ = SYNC_ACCEPT_START.set(start);
    }

    pub(crate) fn start_registered_sync_accept(
        listen: SocketAddr,
        workers: usize,
        entry: Arc<PlatformConnectionEntry>,
    ) -> io::Result<OsWorkerGuard> {
        let start = SYNC_ACCEPT_START.get().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "sync accept starter not registered (link exyonq-platform-linux at composition root)",
            )
        })?;
        start(listen, workers, entry)
    }
}

#[cfg(target_os = "linux")]
pub use linux::register_sync_accept_start;
#[cfg(target_os = "linux")]
pub(crate) use linux::start_registered_sync_accept;
