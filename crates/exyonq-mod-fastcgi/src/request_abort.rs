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
//! Per-request abort gate for orphaned `spawn_blocking` work after await timeout.
//!
//! Tokio does not cancel `spawn_blocking` when the JoinHandle is dropped. The
//! HTTP response is already 504, but the blocking task may still finish I/O.
//! This gate ensures a late completion never checkins a socket into the pool
//! (discard only). The abort mutex is held across the checkin decision so a
//! timeout cannot race a successful idle push. Bounded by Semaphore +
//! ConnPool max_connections — not unbounded threads.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

/// Shared abort state for one delegated FastCGI request.
#[derive(Debug, Default)]
pub struct AbortGate {
    /// `true` once the await-side timeout (or cancel) has fired.
    aborted: Mutex<bool>,
}

impl AbortGate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            aborted: Mutex::new(false),
        })
    }

    pub fn signal_abort(&self) {
        if let Ok(mut g) = self.aborted.lock() {
            *g = true;
        }
    }

    /// Runs `f` only if not aborted; holds the abort lock for the duration of `f`
    /// so timeout cannot flip the flag between the check and idle push.
    pub fn under_checkin_lock<R>(&self, f: impl FnOnce(bool) -> R) -> R {
        match self.aborted.lock() {
            Ok(g) => f(*g),
            Err(poisoned) => f(*poisoned.into_inner()),
        }
    }

    pub fn is_aborted(&self) -> bool {
        match self.aborted.lock() {
            Ok(g) => *g,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

thread_local! {
    static CURRENT_GATE: RefCell<Option<Arc<AbortGate>>> = const { RefCell::new(None) };
}

/// Installs `gate` for the current blocking worker thread. Drop clears it.
pub struct AbortScope {
    _gate: Arc<AbortGate>,
}

impl AbortScope {
    pub fn enter(gate: Arc<AbortGate>) -> Self {
        CURRENT_GATE.with(|slot| {
            *slot.borrow_mut() = Some(Arc::clone(&gate));
        });
        Self { _gate: gate }
    }
}

impl Drop for AbortScope {
    fn drop(&mut self) {
        CURRENT_GATE.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

/// True when the await-side timeout has fired for this request.
pub fn is_aborted() -> bool {
    CURRENT_GATE.with(|slot| slot.borrow().as_ref().is_some_and(|g| g.is_aborted()))
}

/// Invoke `f(aborted)` under the current request abort lock (or `aborted=false` if none).
pub fn with_checkin_gate<R>(f: impl FnOnce(bool) -> R) -> R {
    CURRENT_GATE.with(|slot| match slot.borrow().as_ref() {
        Some(gate) => gate.under_checkin_lock(f),
        None => f(false),
    })
}
