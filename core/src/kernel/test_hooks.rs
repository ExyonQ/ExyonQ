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

/// Install production wire eligibility hooks (once per process).
///
/// Must use real `install_kernel_hooks` — a broad `GET /api/` stub would win the
/// module-api `OnceLock` and classify WebSocket upgrades as GET-only proxy wire,
/// yielding 502 instead of Hyper upgrade (101) in later tests in the same process.
pub fn ensure_wire_hooks_installed() {
    INSTALL.call_once(|| {
        let static_rt = Arc::new(exyonq_mod_static::StaticRuntime::new());
        exyonq_mod_static::install_kernel_hooks(static_rt);
        let proxy_rt = Arc::new(exyonq_mod_proxy::ProxyRuntime::new());
        exyonq_mod_proxy::install_kernel_hooks(proxy_rt);
    });
}
