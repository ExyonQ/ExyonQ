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

use crate::compile::{compile_waf, CompileError, WafCompileInput};
use crate::snapshot::{strongest_action, CompiledWafSnapshot};
use crate::RULE_ID_BODY_INSPECTION_LIMIT;
use arc_swap::ArcSwap;
use exyonq_waf_api::{
    apply_waf_mode, BodyInspectionState, OnInspectionLimit, WafAction, WafDecision, WafEngine,
    WafEvidence, WafPhase, WafRequest, WafRuleId, WafViolation,
};
use std::sync::{Arc, Mutex};

/// Native engine: holds an atomically swappable immutable snapshot.
///
/// Cap013: [`Self::prepare_reload_from_input`] stages a compiled snapshot; [`Self::commit_prepared`]
/// swaps it into the live `ArcSwap`. Failed prepare / discard leaves the live snapshot
/// unchanged.
#[derive(Debug)]
pub struct NativeWafEngine {
    snapshot: ArcSwap<CompiledWafSnapshot>,
    pending: Mutex<Option<Arc<CompiledWafSnapshot>>>,
}

impl NativeWafEngine {
    pub fn from_snapshot(snapshot: Arc<CompiledWafSnapshot>) -> Self {
        Self {
            snapshot: ArcSwap::from(snapshot),
            pending: Mutex::new(None),
        }
    }

    pub fn compile(input: WafCompileInput) -> Result<Self, CompileError> {
        let snap = Arc::new(compile_waf(input)?);
        Ok(Self::from_snapshot(snap))
    }

    /// Hot-reload: replace the immutable snapshot (immediate swap; prefer Cap013 prepare).
    pub fn reload(&self, snapshot: Arc<CompiledWafSnapshot>) {
        self.snapshot.store(snapshot);
    }

    /// Compile `input` and atomically swap the live snapshot.
    pub fn reload_from_input(&self, input: WafCompileInput) -> Result<(), CompileError> {
        let snap = Arc::new(compile_waf(input)?);
        self.reload(snap);
        Ok(())
    }

    /// Cap013 prepare: compile into a pending slot. Does not swap the live snapshot.
    pub fn prepare_reload_from_input(&self, input: WafCompileInput) -> Result<(), CompileError> {
        let snap = Arc::new(compile_waf(input)?);
        let mut pending = self.pending.lock().expect("waf pending lock");
        *pending = Some(snap);
        Ok(())
    }

    /// Cap013 commit: publish the prepared snapshot (no-op if nothing pending).
    pub fn commit_prepared(&self) {
        let mut pending = self.pending.lock().expect("waf pending lock");
        if let Some(snap) = pending.take() {
            self.snapshot.store(snap);
        }
    }

    /// Cap013 discard: drop a prepared snapshot without swapping.
    pub fn discard_prepared(&self) {
        let mut pending = self.pending.lock().expect("waf pending lock");
        *pending = None;
    }

    pub fn load(&self) -> arc_swap::Guard<Arc<CompiledWafSnapshot>> {
        self.snapshot.load()
    }

    /// Generation-frozen engine for Cap013: inspects a fixed snapshot only.
    pub fn fixed_view(&self) -> FixedWafEngine {
        FixedWafEngine::from_snapshot(Arc::clone(&self.snapshot.load()))
    }

    /// Take a prepared snapshot as a frozen engine without swapping the live ArcSwap.
    pub fn take_pending_fixed(&self) -> Option<FixedWafEngine> {
        let mut pending = self.pending.lock().expect("waf pending lock");
        pending.take().map(FixedWafEngine::from_snapshot)
    }

    /// Peek prepared snapshot as frozen engine (leave pending in place for discard).
    pub fn peek_pending_fixed(&self) -> Option<FixedWafEngine> {
        let pending = self.pending.lock().expect("waf pending lock");
        pending
            .as_ref()
            .map(|snap| FixedWafEngine::from_snapshot(Arc::clone(snap)))
    }
}

/// Immutable, generation-bound WAF engine (no shared ArcSwap).
///
/// Cap013 / Cap015: each `ServerState` holds a [`FixedWafEngine`] so committing a
/// new generation cannot mutate in-flight requests' WAF snapshot.
#[derive(Debug, Clone)]
pub struct FixedWafEngine {
    snapshot: Arc<CompiledWafSnapshot>,
}

impl FixedWafEngine {
    pub fn from_snapshot(snapshot: Arc<CompiledWafSnapshot>) -> Self {
        Self { snapshot }
    }

    pub fn compile(input: WafCompileInput) -> Result<Self, CompileError> {
        Ok(Self::from_snapshot(Arc::new(compile_waf(input)?)))
    }

    pub fn snapshot(&self) -> &Arc<CompiledWafSnapshot> {
        &self.snapshot
    }

    pub fn mode(&self) -> exyonq_waf_api::WafMode {
        self.snapshot.mode()
    }
}

fn decide_from_hits(
    snap: &CompiledWafSnapshot,
    hits: Vec<(WafViolation, WafAction)>,
    inspection_truncated: bool,
) -> WafDecision {
    if hits.is_empty() {
        return WafDecision {
            action: WafAction::Allow,
            violations: Vec::new(),
            inspection_truncated,
            retry_after_secs: None,
        };
    }
    let action = strongest_action(hits.iter().map(|(_, a)| *a));
    let violations = hits.into_iter().map(|(v, _)| v).collect();
    let raw = WafDecision {
        action,
        violations,
        inspection_truncated,
        retry_after_secs: None,
    };
    let mut out = apply_waf_mode(snap.mode(), raw);
    out.inspection_truncated = inspection_truncated;
    out
}

fn inspect_headers_with_snap(
    snap: &CompiledWafSnapshot,
    phase: WafPhase,
    req: &WafRequest<'_>,
) -> WafDecision {
    if !snap.enabled() || snap.mode() == exyonq_waf_api::WafMode::Disabled {
        return WafDecision::allow();
    }
    let hits = snap.evaluate_phase(phase, req, None);
    decide_from_hits(snap, hits, false)
}

fn inspect_body_with_snap(
    snap: &CompiledWafSnapshot,
    req: &WafRequest<'_>,
    chunk: &[u8],
    end_of_stream: bool,
    state: &mut BodyInspectionState,
) -> WafDecision {
    if !snap.enabled() || snap.mode() == exyonq_waf_api::WafMode::Disabled {
        return WafDecision::allow();
    }

    let limits = snap.limits();

    if state.limit_applied {
        return WafDecision {
            action: WafAction::Allow,
            violations: Vec::new(),
            inspection_truncated: true,
            retry_after_secs: None,
        };
    }

    let remaining = limits.max_body_bytes.saturating_sub(state.inspected_bytes);

    if chunk.len() > remaining {
        let inspectable = &chunk[..remaining];
        state.inspected_bytes = limits.max_body_bytes;
        state.truncated = true;
        state.limit_applied = true;

        let mut hits = if !inspectable.is_empty() {
            snap.evaluate_phase(WafPhase::RequestBodyChunk, req, Some(inspectable))
        } else {
            Vec::new()
        };

        match limits.on_limit {
            OnInspectionLimit::AllowUninspected => {
                let mut d = decide_from_hits(snap, hits, true);
                d.inspection_truncated = true;
                return d;
            }
            OnInspectionLimit::Block => {
                hits.push((
                    limit_violation(
                        WafPhase::RequestBodyChunk,
                        "body inspection limit exceeded; blocking",
                    ),
                    WafAction::Block,
                ));
                return decide_from_hits(snap, hits, true);
            }
            OnInspectionLimit::LogAndAllow => {
                hits.push((
                    limit_violation(
                        WafPhase::RequestBodyChunk,
                        "body inspection limit exceeded; logging and allowing",
                    ),
                    WafAction::Log,
                ));
                return decide_from_hits(snap, hits, true);
            }
        }
    }

    state.inspected_bytes = state.inspected_bytes.saturating_add(chunk.len());

    let mut hits = snap.evaluate_phase(WafPhase::RequestBodyChunk, req, Some(chunk));
    if end_of_stream {
        hits.extend(snap.evaluate_phase(WafPhase::RequestBodyComplete, req, Some(chunk)));
    }
    decide_from_hits(snap, hits, state.truncated)
}

impl WafEngine for NativeWafEngine {
    fn inspect(&self, phase: WafPhase, req: &WafRequest<'_>) -> WafDecision {
        inspect_headers_with_snap(&self.snapshot.load(), phase, req)
    }

    fn inspect_body_chunk(
        &self,
        req: &WafRequest<'_>,
        chunk: &[u8],
        end_of_stream: bool,
        state: &mut BodyInspectionState,
    ) -> WafDecision {
        inspect_body_with_snap(&self.snapshot.load(), req, chunk, end_of_stream, state)
    }
}

impl WafEngine for FixedWafEngine {
    fn inspect(&self, phase: WafPhase, req: &WafRequest<'_>) -> WafDecision {
        inspect_headers_with_snap(&self.snapshot, phase, req)
    }

    fn inspect_body_chunk(
        &self,
        req: &WafRequest<'_>,
        chunk: &[u8],
        end_of_stream: bool,
        state: &mut BodyInspectionState,
    ) -> WafDecision {
        inspect_body_with_snap(&self.snapshot, req, chunk, end_of_stream, state)
    }
}

fn limit_violation(phase: WafPhase, message: &str) -> WafViolation {
    WafViolation {
        rule_id: WafRuleId::new(RULE_ID_BODY_INSPECTION_LIMIT),
        phase,
        message: message.into(),
        evidence: WafEvidence {
            field: Some("body".into()),
            matched: None,
            transforms: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{
        ExclusionInput, FailPolicy, MatchTarget, RuleInput, RulesetInput, WafCompileInput,
    };
    use crate::snapshot::MapHeaders;
    use exyonq_waf_api::{EmptyHeaders, InspectionLimits, WafMode, WafPhase};
    use std::net::{IpAddr, Ipv4Addr};

    fn base_input() -> WafCompileInput {
        WafCompileInput {
            enabled: true,
            mode: WafMode::Block,
            limits: InspectionLimits {
                max_body_bytes: 16,
                on_limit: OnInspectionLimit::LogAndAllow,
            },
            fail_policy: FailPolicy::ClosedForInvalidRules,
            normalize: crate::NormalizeBudget::default(),
            builtins: Default::default(),
            rulesets: vec![RulesetInput {
                id: "exyonq-core".into(),
                enabled: true,
                rules: vec![RuleInput {
                    id: "EXY-TEST-PATH-1".into(),
                    enabled: true,
                    phases: vec![WafPhase::RequestHeaders],
                    target: MatchTarget::Path,
                    pattern: "../".into(),
                    action: WafAction::Block,
                }],
            }],
            exclusions: Vec::new(),
        }
        .builtins_off()
    }

    fn req<'a>(
        path: &'a str,
        query: Option<&'a str>,
        headers: &'a dyn exyonq_waf_api::HeaderView,
    ) -> WafRequest<'a> {
        WafRequest {
            method: "GET",
            host: Some("example.com"),
            path,
            query,
            headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        }
    }

    #[test]
    fn allow_clean_path() {
        let engine = NativeWafEngine::compile(base_input()).unwrap();
        let headers = EmptyHeaders;
        let d = engine.inspect(WafPhase::RequestHeaders, &req("/api/ok", None, &headers));
        assert_eq!(d.action, WafAction::Allow);
        assert!(d.violations.is_empty());
    }

    #[test]
    fn block_mode_blocks_match() {
        let engine = NativeWafEngine::compile(base_input()).unwrap();
        let headers = EmptyHeaders;
        let d = engine.inspect(
            WafPhase::RequestHeaders,
            &req("/api/../secret", None, &headers),
        );
        assert_eq!(d.action, WafAction::Block);
        assert_eq!(d.violations[0].rule_id.as_str(), "EXY-TEST-PATH-1");
    }

    #[test]
    fn monitor_mode_logs_instead_of_block() {
        let mut input = base_input();
        input.mode = WafMode::Monitor;
        let engine = NativeWafEngine::compile(input).unwrap();
        let headers = EmptyHeaders;
        let d = engine.inspect(
            WafPhase::RequestHeaders,
            &req("/api/../secret", None, &headers),
        );
        assert_eq!(d.action, WafAction::Log);
        assert!(!d.violations.is_empty());
    }

    #[test]
    fn disabled_snapshot_allows() {
        let mut input = base_input();
        input.enabled = false;
        let engine = NativeWafEngine::compile(input).unwrap();
        let headers = EmptyHeaders;
        let d = engine.inspect(
            WafPhase::RequestHeaders,
            &req("/api/../secret", None, &headers),
        );
        assert_eq!(d.action, WafAction::Allow);
    }

    #[test]
    fn exclusion_suppresses_rule() {
        let mut input = base_input();
        input.exclusions.push(ExclusionInput {
            host: Some("example.com".into()),
            path_prefix: Some("/api".into()),
            method: Some("GET".into()),
            rule_ids: vec!["EXY-TEST-PATH-1".into()],
        });
        let engine = NativeWafEngine::compile(input).unwrap();
        let headers = EmptyHeaders;
        let d = engine.inspect(
            WafPhase::RequestHeaders,
            &req("/api/../secret", None, &headers),
        );
        assert_eq!(d.action, WafAction::Allow);
    }

    #[test]
    fn on_limit_block() {
        let mut input = base_input();
        input.limits = InspectionLimits {
            max_body_bytes: 4,
            on_limit: OnInspectionLimit::Block,
        };
        input.rulesets[0].rules.clear();
        let engine = NativeWafEngine::compile(input).unwrap();
        let headers = EmptyHeaders;
        let r = req("/upload", None, &headers);
        let mut state = BodyInspectionState::default();
        let d = engine.inspect_body_chunk(&r, b"12345", false, &mut state);
        assert!(state.limit_applied);
        assert!(state.truncated);
        assert_eq!(d.action, WafAction::Block);
        assert_eq!(
            d.violations[0].rule_id.as_str(),
            RULE_ID_BODY_INSPECTION_LIMIT
        );
        assert!(d.inspection_truncated);
    }

    #[test]
    fn on_limit_log_and_allow() {
        let mut input = base_input();
        input.limits = InspectionLimits {
            max_body_bytes: 4,
            on_limit: OnInspectionLimit::LogAndAllow,
        };
        input.rulesets[0].rules.clear();
        let engine = NativeWafEngine::compile(input).unwrap();
        let headers = EmptyHeaders;
        let r = req("/upload", None, &headers);
        let mut state = BodyInspectionState::default();
        let d = engine.inspect_body_chunk(&r, b"12345", false, &mut state);
        assert_eq!(d.action, WafAction::Log);
        assert!(d.inspection_truncated);
    }

    #[test]
    fn on_limit_allow_uninspected_no_silent_without_flag() {
        let mut input = base_input();
        input.limits = InspectionLimits {
            max_body_bytes: 4,
            on_limit: OnInspectionLimit::AllowUninspected,
        };
        input.rulesets[0].rules.clear();
        let engine = NativeWafEngine::compile(input).unwrap();
        let headers = EmptyHeaders;
        let r = req("/upload", None, &headers);
        let mut state = BodyInspectionState::default();
        let d = engine.inspect_body_chunk(&r, b"12345", false, &mut state);
        assert_eq!(d.action, WafAction::Allow);
        assert!(d.inspection_truncated);
        assert!(state.limit_applied);
    }

    #[test]
    fn body_rule_matches_within_budget() {
        let mut input = base_input();
        input.limits.max_body_bytes = 1024;
        input.rulesets[0].rules = vec![RuleInput {
            id: "EXY-TEST-BODY-1".into(),
            enabled: true,
            phases: vec![WafPhase::RequestBodyChunk],
            target: MatchTarget::Body,
            pattern: "EVIL".into(),
            action: WafAction::Block,
        }];
        let engine = NativeWafEngine::compile(input).unwrap();
        let headers = EmptyHeaders;
        let r = req("/upload", None, &headers);
        let mut state = BodyInspectionState::default();
        let d = engine.inspect_body_chunk(&r, b"xxEVILyy", true, &mut state);
        assert_eq!(d.action, WafAction::Block);
        assert_eq!(d.violations[0].rule_id.as_str(), "EXY-TEST-BODY-1");
    }

    #[test]
    fn compile_rejects_empty_rule_id() {
        let mut input = base_input();
        input.rulesets[0].rules[0].id = "  ".into();
        let err = NativeWafEngine::compile(input).unwrap_err();
        assert_eq!(err, CompileError::EmptyRuleId);
    }

    #[test]
    fn reload_swaps_snapshot() {
        let engine = NativeWafEngine::compile(base_input()).unwrap();
        let headers = EmptyHeaders;
        assert_eq!(
            engine
                .inspect(WafPhase::RequestHeaders, &req("/api/../x", None, &headers))
                .action,
            WafAction::Block
        );

        let mut next = base_input();
        next.mode = WafMode::Monitor;
        let snap = Arc::new(compile_waf(next).unwrap());
        engine.reload(snap);
        assert_eq!(
            engine
                .inspect(WafPhase::RequestHeaders, &req("/api/../x", None, &headers))
                .action,
            WafAction::Log
        );
    }

    #[test]
    fn fixed_engine_unaffected_by_native_reload() {
        let engine = NativeWafEngine::compile(base_input()).unwrap();
        let frozen = engine.fixed_view();
        let headers = EmptyHeaders;
        assert_eq!(
            frozen
                .inspect(WafPhase::RequestHeaders, &req("/x/../y", None, &headers))
                .action,
            WafAction::Block
        );
        // Swap live NativeWafEngine to disabled — Fixed view must keep blocking.
        let mut off = base_input();
        off.enabled = false;
        off.mode = WafMode::Disabled;
        engine.reload_from_input(off).unwrap();
        assert_eq!(
            engine
                .inspect(WafPhase::RequestHeaders, &req("/x/../y", None, &headers))
                .action,
            WafAction::Allow
        );
        assert_eq!(
            frozen
                .inspect(WafPhase::RequestHeaders, &req("/x/../y", None, &headers))
                .action,
            WafAction::Block
        );
    }

    #[test]
    fn prepare_commit_swaps_only_on_commit() {
        let engine = NativeWafEngine::compile(base_input()).unwrap();
        let headers = EmptyHeaders;
        let mut next = base_input();
        next.mode = WafMode::Monitor;
        engine.prepare_reload_from_input(next).unwrap();
        assert_eq!(
            engine
                .inspect(WafPhase::RequestHeaders, &req("/api/../x", None, &headers))
                .action,
            WafAction::Block,
            "live snapshot unchanged before commit"
        );
        engine.commit_prepared();
        assert_eq!(
            engine
                .inspect(WafPhase::RequestHeaders, &req("/api/../x", None, &headers))
                .action,
            WafAction::Log
        );
    }

    #[test]
    fn discard_prepared_keeps_live() {
        let engine = NativeWafEngine::compile(base_input()).unwrap();
        let headers = EmptyHeaders;
        let mut next = base_input();
        next.mode = WafMode::Monitor;
        engine.prepare_reload_from_input(next).unwrap();
        engine.discard_prepared();
        assert_eq!(
            engine
                .inspect(WafPhase::RequestHeaders, &req("/api/../x", None, &headers))
                .action,
            WafAction::Block
        );
    }

    #[test]
    fn compile_rejects_abuse_actions_on_signature_rules() {
        let mut input = base_input();
        input.rulesets[0].rules[0].action = WafAction::RateLimit;
        let err = NativeWafEngine::compile(input).unwrap_err();
        assert!(matches!(err, CompileError::AbuseActionOnSignatureRule(_)));

        let mut input = base_input();
        input.rulesets[0].rules[0].action = WafAction::Challenge;
        let err = NativeWafEngine::compile(input).unwrap_err();
        assert!(matches!(err, CompileError::AbuseActionOnSignatureRule(_)));
    }

    #[test]
    fn header_matcher() {
        let mut input = base_input();
        input.rulesets[0].rules = vec![RuleInput {
            id: "EXY-TEST-HDR-1".into(),
            enabled: true,
            phases: vec![WafPhase::RequestHeaders],
            target: MatchTarget::Header("x-evil".into()),
            pattern: "yes".into(),
            action: WafAction::Block,
        }];
        let engine = NativeWafEngine::compile(input).unwrap();
        let headers = MapHeaders::from_pairs(&[("X-Evil", "yes-please")]);
        let d = engine.inspect(WafPhase::RequestHeaders, &req("/", None, &headers));
        assert_eq!(d.action, WafAction::Block);
    }

    /// WAF-001: LiteralMatcher usa `from_utf8(...).ok()` → non-UTF8 = no match (fail-open).
    /// Detectors usan lossy; el matcher literal no debe dejar pasar el needle en bytes inválidos.
    #[test]
    fn literal_header_matcher_must_not_fail_open_on_non_utf8() {
        let mut input = base_input();
        input.rulesets[0].rules = vec![RuleInput {
            id: "EXY-TEST-HDR-UTF8".into(),
            enabled: true,
            phases: vec![WafPhase::RequestHeaders],
            target: MatchTarget::Header("user-agent".into()),
            pattern: "evilbot".into(),
            action: WafAction::Block,
        }];
        let engine = NativeWafEngine::compile(input).unwrap();
        let headers = MapHeaders {
            entries: vec![("user-agent".into(), b"Mozilla/\xffevilbot".to_vec())],
        };
        let d = engine.inspect(WafPhase::RequestHeaders, &req("/", None, &headers));
        assert_eq!(
            d.action,
            WafAction::Block,
            "WAF-001: LiteralMatcher no debe fail-open en header non-UTF8 que contiene el needle en bytes"
        );
    }
}
