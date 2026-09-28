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
//! Public contracts for the ExyonQ native WAF.
//!
//! Host-agnostic: no Hyper, Tokio, Pingora, or `exyonq-core`. The HTTP stack
//! builds a borrowed [`WafRequest`] and calls a sync [`WafEngine`].

#![forbid(unsafe_code)]

use std::fmt;
use std::net::IpAddr;

/// Stable rule identifier (e.g. `EXY-XSS-1001`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WafRuleId(pub String);

impl WafRuleId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WafRuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for WafRuleId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for WafRuleId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Inspection phase in the host request/response lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WafPhase {
    Connection,
    RequestHeaders,
    RequestBodyChunk,
    RequestBodyComplete,
    ResponseHeaders,
    ResponseBodyChunk,
}

/// Decision action returned by the engine or host abuse seam.
///
/// `Challenge` / `RateLimit` are reserved for host abuse seams (WAF6).
/// Signature engines / `EXY-*` detectors must emit only `Allow` | `Log` | `Block`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WafAction {
    Allow,
    Log,
    Block,
    Challenge,
    RateLimit,
}

/// Runtime enforcement mode (host / compiled snapshot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WafMode {
    Disabled,
    Monitor,
    Block,
}

/// Policy when the body inspection budget is exhausted.
///
/// Never treat over-limit as a silent allow without recording this choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnInspectionLimit {
    AllowUninspected,
    Block,
    LogAndAllow,
}

/// Body (and related) inspection budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectionLimits {
    pub max_body_bytes: usize,
    pub on_limit: OnInspectionLimit,
}

impl Default for InspectionLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 1_048_576,
            on_limit: OnInspectionLimit::LogAndAllow,
        }
    }
}

/// Per-request mutable state for incremental body inspection.
#[derive(Debug, Clone, Default)]
pub struct BodyInspectionState {
    /// Bytes accepted into the inspection window so far.
    pub inspected_bytes: usize,
    /// True once [`OnInspectionLimit`] has been applied for this request.
    pub limit_applied: bool,
    /// True when inspection stopped before seeing the full body.
    pub truncated: bool,
}

/// Transform applied before matching (recorded as evidence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WafTransform {
    UrlDecode { rounds: u8 },
    Lowercase,
    PathNormalize,
    Other(String),
}

/// Evidence attached to a violation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WafEvidence {
    /// Request field or context (e.g. `uri`, `header:cookie`, `body`).
    pub field: Option<String>,
    /// Optional matched excerpt (host may redact).
    pub matched: Option<String>,
    pub transforms: Vec<WafTransform>,
}

/// A single rule hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WafViolation {
    pub rule_id: WafRuleId,
    pub phase: WafPhase,
    pub message: String,
    pub evidence: WafEvidence,
}

/// Engine output for one inspection call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WafDecision {
    pub action: WafAction,
    pub violations: Vec<WafViolation>,
    /// True when the body (or other stream) was not fully inspected.
    pub inspection_truncated: bool,
    /// Optional `Retry-After` seconds (abuse rate-limit).
    pub retry_after_secs: Option<u32>,
}

impl WafDecision {
    pub fn allow() -> Self {
        Self {
            action: WafAction::Allow,
            violations: Vec::new(),
            inspection_truncated: false,
            retry_after_secs: None,
        }
    }

    pub fn is_blocking(&self) -> bool {
        matches!(
            self.action,
            WafAction::Block | WafAction::Challenge | WafAction::RateLimit
        )
    }

    /// Host abuse seam: over-quota (stable rule id for tests / metrics).
    pub fn abuse_rate_limit(phase: WafPhase) -> Self {
        Self::abuse_rate_limit_with_retry(phase, 1)
    }

    /// Host abuse seam with explicit `Retry-After` seconds.
    pub fn abuse_rate_limit_with_retry(phase: WafPhase, retry_after_secs: u32) -> Self {
        Self {
            action: WafAction::RateLimit,
            violations: vec![WafViolation {
                rule_id: WafRuleId::new("EXY-ABUSE-RL-1001"),
                phase,
                message: "abuse rate limit".into(),
                evidence: WafEvidence {
                    field: Some("client_ip".into()),
                    matched: None,
                    transforms: Vec::new(),
                },
            }],
            inspection_truncated: false,
            retry_after_secs: Some(retry_after_secs.max(1)),
        }
    }

    /// Host abuse seam: challenge required (host issues real PoW HTML).
    pub fn abuse_challenge(phase: WafPhase) -> Self {
        Self {
            action: WafAction::Challenge,
            violations: vec![WafViolation {
                rule_id: WafRuleId::new("EXY-ABUSE-CH-1001"),
                phase,
                message: "abuse challenge required".into(),
                evidence: WafEvidence {
                    field: Some("client_ip".into()),
                    matched: None,
                    transforms: Vec::new(),
                },
            }],
            inspection_truncated: false,
            retry_after_secs: None,
        }
    }
}

/// Optional host-side abuse gate (quota / challenge), installed by composition root.
///
/// Runs after signature inspect on request headers only. Must return only
/// [`WafAction::Challenge`] or [`WafAction::RateLimit`] (or `None` to pass).
pub trait WafAbuseGate: Send + Sync {
    fn check(&self, client_ip: IpAddr, route_id: Option<&str>) -> Option<WafDecision>;
}

/// Issued challenge page (host maps to HTTP response).
#[derive(Debug, Clone)]
pub struct ChallengeIssue {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// Result of verifying a PoW challenge POST.
#[derive(Debug, Clone)]
pub enum ChallengeVerifyResult {
    /// Proof accepted — set grant cookie and respond.
    Ok {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
        set_cookie: String,
        location: Option<String>,
    },
    /// Verification failed — host returns 403.
    Fail {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
    },
}

/// Host-installed challenge issuer/verifier (composition root; concrete in `exyonq-waf`).
pub trait WafChallengeHandler: Send + Sync {
    fn issue(
        &self,
        client_ip: IpAddr,
        host: Option<&str>,
        path: &str,
        generation: u64,
    ) -> ChallengeIssue;

    fn verify_post(
        &self,
        client_ip: IpAddr,
        host: Option<&str>,
        body: &[u8],
        generation: u64,
    ) -> ChallengeVerifyResult;

    fn grant_allows(
        &self,
        client_ip: IpAddr,
        host: Option<&str>,
        cookie_header: Option<&[u8]>,
        generation: u64,
    ) -> bool;
}

/// Borrowed header map view — implement for Hyper/`http` maps in the host.
pub trait HeaderView: Send + Sync {
    /// Case-insensitive name lookup; value as raw bytes.
    fn get(&self, name: &str) -> Option<&[u8]>;

    /// Visit each header once as `(name_bytes, value_bytes)`.
    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8]));
}

/// Empty header view for tests and connection-phase calls.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyHeaders;

impl HeaderView for EmptyHeaders {
    fn get(&self, _name: &str) -> Option<&[u8]> {
        None
    }

    fn for_each(&self, _f: &mut dyn FnMut(&[u8], &[u8])) {}
}

/// Borrowed request view for sync inspection.
pub struct WafRequest<'a> {
    pub method: &'a str,
    pub host: Option<&'a str>,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub headers: &'a dyn HeaderView,
    /// Already resolved by ExyonQ — do not re-parse untrusted client XFF here.
    pub client_ip: IpAddr,
    pub route_id: Option<&'a str>,
}

/// Sync WAF engine. Implementations live in `exyonq-waf`; core depends only on this trait.
pub trait WafEngine: Send + Sync {
    fn inspect(&self, phase: WafPhase, req: &WafRequest<'_>) -> WafDecision;

    fn inspect_body_chunk(
        &self,
        req: &WafRequest<'_>,
        chunk: &[u8],
        end_of_stream: bool,
        state: &mut BodyInspectionState,
    ) -> WafDecision;
}

/// No-op engine: always [`WafAction::Allow`]. Useful before a concrete engine is wired.
#[derive(Debug, Default, Clone, Copy)]
pub struct NopWafEngine;

impl WafEngine for NopWafEngine {
    fn inspect(&self, _phase: WafPhase, _req: &WafRequest<'_>) -> WafDecision {
        WafDecision::allow()
    }

    fn inspect_body_chunk(
        &self,
        _req: &WafRequest<'_>,
        _chunk: &[u8],
        _end_of_stream: bool,
        _state: &mut BodyInspectionState,
    ) -> WafDecision {
        WafDecision::allow()
    }
}

/// Apply [`WafMode`] to a raw engine decision (host-side policy).
///
/// - [`WafMode::Disabled`] → allow, drop violations
/// - [`WafMode::Monitor`] → never block/challenge/rate-limit; force [`WafAction::Log`] if any hit
/// - [`WafMode::Block`] → pass through
pub fn apply_waf_mode(mode: WafMode, mut decision: WafDecision) -> WafDecision {
    match mode {
        WafMode::Disabled => WafDecision::allow(),
        WafMode::Monitor => {
            if decision.action != WafAction::Allow || !decision.violations.is_empty() {
                decision.action = WafAction::Log;
            }
            decision
        }
        WafMode::Block => decision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn sample_req<'a>(headers: &'a dyn HeaderView) -> WafRequest<'a> {
        WafRequest {
            method: "GET",
            host: Some("example.com"),
            path: "/",
            query: None,
            headers,
            client_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            route_id: None,
        }
    }

    #[test]
    fn nop_engine_allows() {
        let engine = NopWafEngine;
        let headers = EmptyHeaders;
        let req = sample_req(&headers);
        let d = engine.inspect(WafPhase::RequestHeaders, &req);
        assert_eq!(d.action, WafAction::Allow);
        assert!(d.violations.is_empty());
    }

    #[test]
    fn monitor_mode_downgrades_block_to_log() {
        let decision = WafDecision {
            action: WafAction::Block,
            violations: vec![WafViolation {
                rule_id: WafRuleId::new("EXY-TEST-1"),
                phase: WafPhase::RequestHeaders,
                message: "test".into(),
                evidence: WafEvidence::default(),
            }],
            inspection_truncated: false,
            retry_after_secs: None,
        };
        let out = apply_waf_mode(WafMode::Monitor, decision);
        assert_eq!(out.action, WafAction::Log);
        assert_eq!(out.violations.len(), 1);
    }

    #[test]
    fn disabled_mode_clears_decision() {
        let decision = WafDecision {
            action: WafAction::Block,
            violations: vec![WafViolation {
                rule_id: WafRuleId::new("EXY-TEST-1"),
                phase: WafPhase::RequestHeaders,
                message: "test".into(),
                evidence: WafEvidence::default(),
            }],
            inspection_truncated: false,
            retry_after_secs: None,
        };
        let out = apply_waf_mode(WafMode::Disabled, decision);
        assert_eq!(out.action, WafAction::Allow);
        assert!(out.violations.is_empty());
    }
}
