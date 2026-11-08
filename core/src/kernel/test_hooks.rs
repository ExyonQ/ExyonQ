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
//! Test-only wire hook bootstrap for kernel planner tests.

use std::sync::{Arc, Once};

static INSTALL: Once = Once::new();

/// Process-global kernel hooks and pinned module runtimes are shared mutable
/// state: `install_kernel_hooks` replaces the pinned runtime, so any test that
/// installs can swap the runtime another in-flight `--lib` test resolves
/// through. Serialize one-shot installs and tests that pin their own runtime on
/// a single gate.
pub(crate) static WIRE_HOOKS_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Duration hold for async tests — acquired once at test setup and kept until
/// the returned guard drops so the pinned runtimes stay installed.
pub(crate) async fn wire_hooks_gate() -> tokio::sync::MutexGuard<'static, ()> {
    WIRE_HOOKS_GATE.lock().await
}

/// Duration hold for sync `#[test]`s — `blocking_lock` panics inside an async
/// runtime context, so this is only legal on a plain test thread (before any
/// local `Runtime::new()`/`block_on`).
pub(crate) fn wire_hooks_gate_blocking() -> tokio::sync::MutexGuard<'static, ()> {
    WIRE_HOOKS_GATE.blocking_lock()
}

/// Run a one-shot kernel-hook install under the shared gate. Executes on a
/// plain thread so `blocking_lock` is legal from `#[test]` and `#[tokio::test]`
/// callers alike. The caller must not already hold the gate (self-deadlock).
pub(crate) fn with_wire_hooks_gate(install: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let _gate = WIRE_HOOKS_GATE.blocking_lock();
        install();
    })
    .join()
    .expect("wire hooks install thread");
}

/// Install production wire eligibility hooks (once per process).
///
/// Must use real `install_kernel_hooks` — a broad `GET /api/` stub would win the
/// module-api `OnceLock` and classify WebSocket upgrades as GET-only proxy wire,
/// yielding 502 instead of Hyper upgrade (101) in later tests in the same process.
pub fn ensure_wire_hooks_installed() {
    with_wire_hooks_gate(|| {
        INSTALL.call_once(|| {
            let static_rt = Arc::new(exyonq_mod_static::StaticRuntime::new());
            exyonq_mod_static::install_kernel_hooks(static_rt);
            let proxy_rt = Arc::new(exyonq_mod_proxy::ProxyRuntime::new());
            exyonq_mod_proxy::install_kernel_hooks(proxy_rt);
        });
    });
}

/// Pin a test static runtime that owns `/site` under `root`.
///
/// L-02 accept admission fails closed when no compiled static slot owns the
/// request, so tests asserting static wire admission must publish a real slot.
/// Call only while holding [`WIRE_HOOKS_GATE`].
#[cfg(target_os = "linux")]
pub(crate) fn pin_site_static_runtime(root: &std::path::Path) {
    let site = exyonq_mod_static::StaticRoot::new(root, "/site", None).expect("static root");
    let rt = Arc::new(exyonq_mod_static::StaticRuntime::new());
    rt.bind_roots(1, vec![Arc::new(site)].into_boxed_slice());
    exyonq_mod_static::install_kernel_hooks(rt);
}
