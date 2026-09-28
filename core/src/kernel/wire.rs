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
/// Cap031 LA-002: Connection is a comma-separated token list (e.g. `close, Upgrade`).
fn wire_head_is_websocket_upgrade(head: &[u8]) -> bool {
    let h = head.to_ascii_lowercase();
    if !h
        .windows(b"upgrade: websocket".len())
        .any(|w| w == b"upgrade: websocket")
    {
        return false;
    }
    for line in h.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r".as_slice()).unwrap_or(line);
        let Some(rest) = line.strip_prefix(b"connection:") else {
            continue;
        };
        let Ok(val) = std::str::from_utf8(rest) else {
            continue;
        };
        if val
            .split(',')
            .map(str::trim)
            .any(|token| token == "upgrade")
        {
            return true;
        }
    }
    false
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

    // Cap067 wire-cheap: ratelimit/metrics do NOT force Hyper here.
    // Compression uses early AE gate in plan_wire_after_headers / handler
    // (`modules_require_hyper_for_request_head`), not this planner.
    // OpenMetrics scrape stays Hyper via `tokio_accept_required` / path.
    if proxy_wire::might_use_proxy_wire(head) {
        return Ok(WirePlanDecision::Proxy);
    }
    if view.site_static_slot.is_some() && static_wire::might_use_static_wire(head) {
        return Ok(WirePlanDecision::Static);
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
    fn ps1c_planner_static_for_health_wire() {
        ensure_wire_hooks_installed();
        let head = b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n";
        let decision = plan_wire_decision(&bench_view(), head).expect("plan");
        assert_eq!(decision, WirePlanDecision::Static);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ps1c_planner_static_for_ordinary_site_asset_when_sendfile_auto() {
        ensure_wire_hooks_installed();
        let head = b"GET /site/1k.bin HTTP/1.1\r\nHost: x\r\n\r\n";
        let decision = plan_wire_decision(&bench_view(), head).expect("plan");
        // Cap067: ordinary static GET is Wire Static so WAF→sendfile divert can run.
        assert_eq!(decision, WirePlanDecision::Static);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn ps1c_planner_hyper_for_ordinary_site_asset_on_non_linux() {
        ensure_wire_hooks_installed();
        let head = b"GET /site/1k.bin HTTP/1.1\r\nHost: x\r\n\r\n";
        let decision = plan_wire_decision(&bench_view(), head).expect("plan");
        assert_eq!(decision, WirePlanDecision::Hyper);
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
    fn cap031_planner_websocket_connection_close_upgrade_uses_hyper() {
        ensure_wire_hooks_installed();
        // LA-CAP031-002: token list form must not fall through to GET-only proxy wire.
        let head = b"GET /api/ws HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: close, Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        let decision = plan_wire_decision(&bench_view(), head).expect("plan");
        assert_eq!(decision, WirePlanDecision::Hyper);
    }

    #[test]
    fn cap031_planner_websocket_connection_keep_alive_upgrade_no_space_uses_hyper() {
        ensure_wire_hooks_installed();
        let head = b"GET /api/ws HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: keep-alive,Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        let decision = plan_wire_decision(&bench_view(), head).expect("plan");
        assert_eq!(decision, WirePlanDecision::Hyper);
    }

    #[test]
    fn wire_cheap_modules_enabled_still_allows_proxy_and_static() {
        ensure_wire_hooks_installed();
        // modules_enabled=true alone must not force Hyper (Cap067 wire-cheap).
        let view = GenerationView::pinned(1, true, Some(0));
        let site = b"GET /site/index.html HTTP/1.1\r\nHost: x\r\n\r\n";
        let api = b"GET /api/health HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(
            plan_wire_decision(&view, api).expect("plan"),
            WirePlanDecision::Proxy
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            plan_wire_decision(&view, site).expect("plan"),
            WirePlanDecision::Static
        );
        #[cfg(not(target_os = "linux"))]
        assert_eq!(
            plan_wire_decision(&view, site).expect("plan"),
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
