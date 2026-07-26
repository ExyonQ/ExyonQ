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
//! Module-owned sendfile handle registry (KD2.2).
//!
//! ## Ownership
//! - **Registry** holds `Arc<SendfileAsset>` keyed by opaque id until `take` or `release`.
//! - **File fd** lifetime is `Arc<File>` inside `SendfileAsset`; closed when last `Arc` drops.
//! - **Core** must not hold raw fds from static dispatch — only `SendfileHandle` ids until KD2.3 FSM
//!   calls `StaticRuntime::take_sendfile_handle` exactly once per id.
//!
//! ## Reload / generation
//! - Each issued handle records the runtime `generation` at issue time.
//! - `invalidate_through_generation(g)` drops stale handles so reload cannot serve obsolete assets.

use crate::sendfile::SendfileAsset;
use exyonq_module_api::static_dispatch::SendfileHandle;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct Entry {
    generation: u64,
    asset: Arc<SendfileAsset>,
}

#[derive(Debug)]
pub struct SendfileHandleRegistry {
    next_id: AtomicU64,
    entries: Mutex<HashMap<u64, Entry>>,
}

impl Default for SendfileHandleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SendfileHandleRegistry {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Issue a handle; registry retains ownership until [`Self::take`] or [`Self::release`].
    pub fn issue(&self, generation: u64, asset: Arc<SendfileAsset>) -> SendfileHandle {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.entries
            .lock()
            .expect("sendfile handle registry poisoned")
            .insert(id, Entry { generation, asset });
        SendfileHandle(id)
    }

    /// Transfer ownership to caller (FSM / materializer). Idempotent only once — second take returns None.
    pub fn take(&self, handle: SendfileHandle) -> Option<Arc<SendfileAsset>> {
        self.entries
            .lock()
            .expect("sendfile handle registry poisoned")
            .remove(&handle.0)
            .map(|entry| entry.asset)
    }

    /// Cancel without transferring (error/timeout path). Safe if already taken.
    pub fn release(&self, handle: SendfileHandle) {
        self.entries
            .lock()
            .expect("sendfile handle registry poisoned")
            .remove(&handle.0);
    }

    /// Drop handles from reload generations strictly before `generation`.
    pub fn invalidate_through_generation(&self, generation: u64) {
        self.entries
            .lock()
            .expect("sendfile handle registry poisoned")
            .retain(|_, entry| entry.generation >= generation);
    }

    pub fn active_count(&self) -> usize {
        self.entries
            .lock()
            .expect("sendfile handle registry poisoned")
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    fn sample_asset() -> Arc<SendfileAsset> {
        let dir = tempdir().unwrap();
        let path = dir.path().join("body.bin");
        let mut file = File::create(&path).unwrap();
        file.write_all(b"sendfile-body").unwrap();
        Arc::new(SendfileAsset::open(&path, 13).unwrap())
    }

    #[test]
    fn take_transfers_once_release_is_idempotent() {
        let reg = SendfileHandleRegistry::new();
        let asset = sample_asset();
        let handle = reg.issue(1, Arc::clone(&asset));

        let taken = reg.take(handle).expect("first take");
        assert!(Arc::ptr_eq(&taken, &asset));
        assert!(reg.take(handle).is_none());
        reg.release(handle);
        assert_eq!(reg.active_count(), 0);
    }

    #[test]
    fn release_drops_without_take() {
        let reg = SendfileHandleRegistry::new();
        let handle = reg.issue(1, sample_asset());
        reg.release(handle);
        assert_eq!(reg.active_count(), 0);
        assert!(reg.take(handle).is_none());
    }

    #[test]
    fn invalidate_generation_clears_stale_handles() {
        let reg = SendfileHandleRegistry::new();
        let old = reg.issue(1, sample_asset());
        let _new = reg.issue(2, sample_asset());
        reg.invalidate_through_generation(2);
        assert!(reg.take(old).is_none());
        assert_eq!(reg.active_count(), 1);
    }
}
