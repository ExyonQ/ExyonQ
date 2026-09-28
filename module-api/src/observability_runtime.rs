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
//! KD4.12 — cold-path Prometheus append hook registry (no formatting in core).
//!
//! Ownership (Phase 5 / Cap054):
//! - **Production** appenders live in a process-global vec registered at composition
//!   and are never cleared by tests.
//! - **Test overlay** holds ephemeral appenders for unit tests; clearing / RAII
//!   only touches the overlay so parallel workspace tests cannot wipe production
//!   registrations mid-assertion.

use std::sync::{Mutex, MutexGuard, OnceLock};

pub type PrometheusAppendFn = fn(&mut String);

static PRODUCTION_APPENDERS: Mutex<Vec<PrometheusAppendFn>> = Mutex::new(Vec::new());
static FROZEN_APPENDERS: OnceLock<Box<[PrometheusAppendFn]>> = OnceLock::new();
static TEST_OVERLAY: Mutex<Vec<PrometheusAppendFn>> = Mutex::new(Vec::new());
static TEST_OVERLAY_GATE: Mutex<()> = Mutex::new(());
static RUNTIME_APPEND_REGISTERED: OnceLock<()> = OnceLock::new();

fn lock_vec(m: &Mutex<Vec<PrometheusAppendFn>>) -> MutexGuard<'_, Vec<PrometheusAppendFn>> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Register a cold-path Prometheus text appender (composition root / production).
///
/// After [`freeze_prometheus_appenders`], further production registers are ignored
/// (composition is sealed). Tests must use [`register_test_prometheus_appender`].
pub fn register_prometheus_appender(append: PrometheusAppendFn) {
    if FROZEN_APPENDERS.get().is_some() {
        return;
    }
    lock_vec(&PRODUCTION_APPENDERS).push(append);
}

/// Seal production appenders into an immutable snapshot (optional composition step).
pub fn freeze_prometheus_appenders() {
    let snapshot: Vec<PrometheusAppendFn> = lock_vec(&PRODUCTION_APPENDERS).clone();
    let _ = FROZEN_APPENDERS.set(snapshot.into_boxed_slice());
}

/// Register a test-only appender into the overlay (does not mutate production).
pub fn register_test_prometheus_appender(append: PrometheusAppendFn) {
    lock_vec(&TEST_OVERLAY).push(append);
}

/// Exclusive RAII gate for tests that mutate the overlay.
///
/// Holding the guard serializes overlay-mutating tests only — not the workspace.
pub struct PrometheusAppenderTestGuard {
    _gate: MutexGuard<'static, ()>,
}

impl Drop for PrometheusAppenderTestGuard {
    fn drop(&mut self) {
        lock_vec(&TEST_OVERLAY).clear();
    }
}

/// Begin an exclusive test section that owns the Prometheus test overlay.
pub fn begin_prometheus_appender_test() -> PrometheusAppenderTestGuard {
    let gate = TEST_OVERLAY_GATE.lock().unwrap_or_else(|p| p.into_inner());
    lock_vec(&TEST_OVERLAY).clear();
    PrometheusAppenderTestGuard { _gate: gate }
}

/// Invoke production (+ frozen) appenders and the test overlay.
pub fn append_registered_prometheus(out: &mut String) {
    if let Some(frozen) = FROZEN_APPENDERS.get() {
        for append in frozen.iter() {
            append(out);
        }
    } else {
        for append in lock_vec(&PRODUCTION_APPENDERS).iter() {
            append(out);
        }
    }
    for append in lock_vec(&TEST_OVERLAY).iter() {
        append(out);
    }
}

/// Install the composite runtime append hook exactly once.
pub fn ensure_runtime_prometheus_hook(install: fn(PrometheusAppendFn)) {
    let _ = RUNTIME_APPEND_REGISTERED.get_or_init(|| {
        install(append_registered_prometheus);
    });
}

/// Clear **test overlay only**. Never clears production / frozen appenders.
///
/// Acquires the same exclusive overlay gate as [`begin_prometheus_appender_test`]
/// so this cannot race with overlay registration / append tests.
/// Do **not** call while already holding a [`PrometheusAppenderTestGuard`]
/// (non-reentrant mutex → deadlock). Prefer the guard's `Drop` clear instead.
#[doc(hidden)]
pub fn clear_prometheus_appenders_for_tests() {
    let _gate = TEST_OVERLAY_GATE.lock().unwrap_or_else(|p| p.into_inner());
    lock_vec(&TEST_OVERLAY).clear();
}
