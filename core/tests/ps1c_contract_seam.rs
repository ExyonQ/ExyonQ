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

//! PS1C contract seam — portable integration tests.

use exyonq_core::kernel::{
    plan_wire_decision, GenerationView, PlatformConnectionAdmission, WirePlanDecision,
};
use exyonq_core::lifecycle::LifecycleState;
use std::sync::{Arc, Once};

static HOOKS: Once = Once::new();

fn ensure_wire_hooks() {
    HOOKS.call_once(|| {
        exyonq_mod_static::install_kernel_hooks(Arc::new(exyonq_mod_static::StaticRuntime::new()));
        exyonq_mod_proxy::install_kernel_hooks(Arc::new(exyonq_mod_proxy::ProxyRuntime::new()));
    });
}

#[test]
fn ps1c_lifecycle_seam_admits_once_via_raii() {
    let ops = LifecycleState::new();
    let token = PlatformConnectionAdmission::try_admit(&ops).expect("admit");
    assert_eq!(ops.active_connections(), 1);
    drop(token);
    assert_eq!(ops.active_connections(), 0);
}

#[test]
fn ps1c_planner_api_is_transport_agnostic() {
    ensure_wire_hooks();
    let view = GenerationView::pinned(1, false, Some(0));
    let head = b"GET /site/index.html HTTP/1.1\r\nHost: x\r\n\r\n";
    // API accepts only GenerationView + bytes — no stream/fd parameters.
    assert!(plan_wire_decision(&view, head).is_ok());
}

#[test]
fn ps1c_generation_identity_stable_across_planner_call() {
    ensure_wire_hooks();
    let view = GenerationView::pinned(77, false, Some(1));
    let head = b"GET /api/x HTTP/1.1\r\nHost: x\r\n\r\n";
    let _ = plan_wire_decision(&view, head).expect("plan");
    assert_eq!(view.generation, 77);
}

#[test]
fn ps1c_planner_parity_proxy_decision() {
    ensure_wire_hooks();
    let view = GenerationView::pinned(1, false, Some(0));
    let head = b"GET /api/health HTTP/1.1\r\nHost: x\r\n\r\n";
    assert_eq!(
        plan_wire_decision(&view, head).expect("plan"),
        WirePlanDecision::Proxy
    );
}
