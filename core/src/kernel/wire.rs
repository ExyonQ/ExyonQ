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
//! Semantic wire planner — transport-agnostic (no stream, fd, or worker state).

use crate::kernel::errors::WirePlanningError;
use crate::kernel::generation::GenerationView;
use exyonq_module_api::{proxy_wire, static_wire};

/// Semantic dispatch decision after request headers are available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WirePlanDecision {
    Static,
    Proxy,
    Hyper,
}

/// Raw-head WebSocket upgrade — must use Hyper (proxy wire is GET-response only).
fn wire_head_is_websocket_upgrade(head: &[u8]) -> bool {
    let h = head.to_ascii_lowercase();
    h.windows(b"upgrade: websocket".len())
        .any(|w| w == b"upgrade: websocket")
        && (h
            .windows(b"connection: upgrade".len())
            .any(|w| w == b"connection: upgrade")
            || h.windows(b"connection: keep-alive, upgrade".len())
                .any(|w| w == b"connection: keep-alive, upgrade"))
}

/// Classify wire path from pinned generation view and header bytes only.
///
/// **STATIC_DIRECT** — monomorphized predicate calls into module-api hooks; no `dyn`, no `Box`.
pub fn plan_wire_decision(
    view: &GenerationView,
    head: &[u8],
) -> Result<WirePlanDecision, WirePlanningError> {
    if head.is_empty() {
        return Err(WirePlanningError);
    }

    // Invariant: upgrades never take the GET-only proxy wire path.
    if wire_head_is_websocket_upgrade(head) {
        return Ok(WirePlanDecision::Hyper);
    }

    if !view.modules_enabled {
        if proxy_wire::might_use_proxy_wire(head) {
            return Ok(WirePlanDecision::Proxy);
        }
        if view.site_static_slot.is_some() && static_wire::might_use_static_wire(head) {
            return Ok(WirePlanDecision::Static);
        }
    }

    Ok(WirePlanDecision::Hyper)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::test_hooks::ensure_wire_hooks_installed;

    fn bench_view() -> GenerationView {
        GenerationView::pinned(1, false, Some(0))
    }

    #[test]
    fn ps1c_planner_rejects_empty_head() {
        ensure_wire_hooks_installed();
        assert!(plan_wire_decision(&bench_view(), b"").is_err());
    }

    #[test]
    fn ps1c_planner_static_for_site_wire() {
        ensure_wire_hooks_installed();
        let head = b"GET /site/1k.bin HTTP/1.1\r\nHost: x\r\n\r\n";
        let decision = plan_wire_decision(&bench_view(), head).expect("plan");
        assert_eq!(decision, WirePlanDecision::Static);
    }

    #[test]
    fn ps1c_planner_proxy_for_api_wire() {
        ensure_wire_hooks_installed();
        let head = b"GET /api/health HTTP/1.1\r\nHost: x\r\n\r\n";
        let decision = plan_wire_decision(&bench_view(), head).expect("plan");
        assert_eq!(decision, WirePlanDecision::Proxy);
    }

    #[test]
    fn ps1c_planner_websocket_upgrade_uses_hyper() {
        ensure_wire_hooks_installed();
        let head = b"GET /api/ws-echo HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        let decision = plan_wire_decision(&bench_view(), head).expect("plan");
        assert_eq!(decision, WirePlanDecision::Hyper);
    }

    #[test]
    fn ps1c_planner_hyper_when_modules_enabled() {
        ensure_wire_hooks_installed();
        let view = GenerationView::pinned(1, true, Some(0));
        let head = b"GET /site/index.html HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(
            plan_wire_decision(&view, head).expect("plan"),
            WirePlanDecision::Hyper
        );
    }

    #[test]
    fn ps1c_planner_preserves_generation_view_identity() {
        ensure_wire_hooks_installed();
        let view = GenerationView::pinned(42, false, None);
        let head = b"GET /unknown HTTP/1.1\r\nHost: x\r\n\r\n";
        let _ = plan_wire_decision(&view, head).expect("plan");
        assert_eq!(view.generation, 42);
    }
}
