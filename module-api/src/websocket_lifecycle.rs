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
//! Cap031: admission ownership for Hyper WebSocket tunnels that outlive the HTTP connection future.
//!
//! Core installs a factory that extends `active_connections` for the tunnel task duration.
//! Proxy module acquires the hold before spawning the bidirectional copy and drops it when the
//! tunnel ends — so drain/shutdown cannot observe zero active work while a tunnel is live.

use std::sync::{Arc, Mutex};

type HoldFactory = Arc<dyn Fn() -> Option<Box<dyn Send>> + Send + Sync>;

// Cap031 LA-005: replaceable (not OnceLock) so each run_on LifecycleState can rebind.
static WS_TUNNEL_HOLD: Mutex<Option<HoldFactory>> = Mutex::new(None);

/// Install/replace the Cap031 tunnel lifecycle factory (core server start).
pub fn install_websocket_tunnel_hold(factory: HoldFactory) {
    if let Ok(mut slot) = WS_TUNNEL_HOLD.lock() {
        *slot = Some(factory);
    }
}

/// Acquire a Send hold that must outlive the upgraded WebSocket tunnel task.
///
/// Returns `None` when core has not installed a factory (unit tests of proxy alone).
pub fn take_websocket_tunnel_hold() -> Option<Box<dyn Send>> {
    WS_TUNNEL_HOLD
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().and_then(|f| f()))
}
