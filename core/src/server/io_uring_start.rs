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
//! Composition seam: register io_uring starter from `exyonq-platform-linux` (D1-safe).
//!
//! **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**

/// Env gate for the io_uring listener path (mechanism flag only).
pub fn io_uring_env_enabled(tls_on_listen: bool) -> bool {
    !tls_on_listen && std::env::var("EXYONQ_IO_URING").ok().as_deref() == Some("1")
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::kernel::PlatformConnectionEntry;
    use crate::server::os_worker_guard::OsWorkerGuard;
    use std::io;
    use std::net::SocketAddr;
    use std::sync::{Arc, OnceLock};

    pub type IoUringStartFn =
        fn(SocketAddr, usize, Arc<PlatformConnectionEntry>) -> io::Result<OsWorkerGuard>;

    static IO_URING_START: OnceLock<IoUringStartFn> = OnceLock::new();

    pub fn register_io_uring_start(start: IoUringStartFn) {
        let _ = IO_URING_START.set(start);
    }

    pub(crate) fn start_registered_io_uring(
        listen: SocketAddr,
        workers: usize,
        entry: Arc<PlatformConnectionEntry>,
    ) -> io::Result<OsWorkerGuard> {
        let start = IO_URING_START.get().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "io_uring starter not registered (link exyonq-platform-linux at composition root)",
            )
        })?;
        start(listen, workers, entry)
    }
}

#[cfg(target_os = "linux")]
pub use linux::register_io_uring_start;
#[cfg(target_os = "linux")]
pub(crate) use linux::start_registered_io_uring;
