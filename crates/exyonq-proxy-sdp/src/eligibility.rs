/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/// Pre-side-effect eligibility using MSG_PEEK (no ambiguous specialized consume).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdpEligibility {
    /// SDP-P1 may own this connection for request processing.
    Specialized,
    /// Hand off to legacy Hyper/Cap067 before specialized parse advances.
    HandoffLegacy,
    /// Close without handoff (malformed prefix).
    RejectClose,
}

/// Peek-based eligibility: GET /api/ without Upgrade / TE / duplicate CL signals.
pub fn peek_sdp_eligible(peek: &[u8]) -> SdpEligibility {
    if peek.is_empty() {
        return SdpEligibility::HandoffLegacy;
    }
    // Need at least request-line start.
    if peek.len() < 8 {
        return SdpEligibility::HandoffLegacy;
    }
    if !(peek.starts_with(b"GET /api/") || peek.starts_with(b"GET /api ")) {
        return SdpEligibility::HandoffLegacy;
    }
    let lower = peek.to_ascii_lowercase();
    if lower
        .windows(b"upgrade: websocket".len())
        .any(|w| w == b"upgrade: websocket")
    {
        return SdpEligibility::HandoffLegacy;
    }
    if lower
        .windows(b"transfer-encoding:".len())
        .any(|w| w == b"transfer-encoding:")
    {
        // Policy: TE on request → not SDP; handoff only if we have not consumed —
        // peek-only, so handoff is safe. Legacy may also reject.
        return SdpEligibility::HandoffLegacy;
    }
    let cl = b"content-length:";
    let cl_count = lower.windows(cl.len()).filter(|w| *w == cl).count();
    if cl_count > 1 {
        return SdpEligibility::RejectClose;
    }
    SdpEligibility::Specialized
}
