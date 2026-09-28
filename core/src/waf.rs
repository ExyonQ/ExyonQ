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
//! WAF host adapter: core depends only on `exyonq-waf-api`.
//!
//! Composition root installs generation-frozen [`WafRuntimeBinding`] values (engine +
//! abuse + enforce). Cap013 prepare stages a candidate binding; `ServerState` captures
//! it; commit promotes it to current. In-flight generations keep their frozen engines
//! (WAF-LOGIC-P3-A).

#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

use crate::config::AppConfig;
use arc_swap::ArcSwap;
use exyonq_waf_api::{
    BodyInspectionState, ChallengeVerifyResult, EmptyHeaders, HeaderView, NopWafEngine,
    WafAbuseGate, WafAction, WafChallengeHandler, WafDecision, WafEngine, WafPhase, WafRequest,
};
use http::HeaderMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tracing::{debug, info, warn};

/// Generation-coherent WAF runtime (engine snapshot + abuse + enforce).
#[derive(Clone)]
pub struct WafRuntimeBinding {
    pub engine: Arc<dyn WafEngine>,
    pub abuse: Option<Arc<dyn WafAbuseGate>>,
    pub enforce: bool,
    /// ARCH-002: generation-frozen — wire must materialize headers / run inspect when true.
    ///
    /// Matches composition compile predicate (`enabled && resolved_mode != Disabled`)
    /// plus enforce+abuse (so `enabled=false` still runs abuse rejects when enforce).
    /// Never derive solely from IR `mode=Disabled` (EXYONQ_WAF_MODE may activate).
    pub wire_inspection_active: bool,
}

/// ARCH-002: compute generation-frozen wire inspection flag.
///
/// `signature_active` must already include env-resolved mode (`enabled && mode != Disabled`).
#[inline]
pub fn compute_wire_inspection_active(
    signature_active: bool,
    abuse_present: bool,
    enforce: bool,
) -> bool {
    signature_active || (abuse_present && enforce)
}

impl Default for WafRuntimeBinding {
    fn default() -> Self {
        Self {
            engine: Arc::new(NopWafEngine),
            abuse: None,
            enforce: false,
            wire_inspection_active: false,
        }
    }
}

static INSTALLED_WAF: OnceLock<Arc<dyn WafEngine>> = OnceLock::new();
/// Legacy global abuse slot (kept in sync on commit for non-state callers/tests).
static INSTALLED_ABUSE: OnceLock<ArcSwap<Option<Arc<dyn WafAbuseGate>>>> = OnceLock::new();
static INSTALLED_CHALLENGE: OnceLock<Arc<dyn WafChallengeHandler>> = OnceLock::new();
static WAF_PREPARE_HOOK: OnceLock<Arc<dyn Fn(&AppConfig) -> Result<(), String> + Send + Sync>> =
    OnceLock::new();
static WAF_COMMIT_HOOK: OnceLock<Arc<dyn Fn(u64) + Send + Sync>> = OnceLock::new();
static WAF_DISCARD_HOOK: OnceLock<Arc<dyn Fn() + Send + Sync>> = OnceLock::new();
static WAF_ENFORCE: AtomicBool = AtomicBool::new(false);
static CURRENT_BINDING: OnceLock<ArcSwap<WafRuntimeBinding>> = OnceLock::new();
static PENDING_BINDING: OnceLock<Mutex<Option<WafRuntimeBinding>>> = OnceLock::new();

fn abuse_slot() -> &'static ArcSwap<Option<Arc<dyn WafAbuseGate>>> {
    INSTALLED_ABUSE.get_or_init(|| ArcSwap::from_pointee(None))
}

fn current_binding_slot() -> &'static ArcSwap<WafRuntimeBinding> {
    CURRENT_BINDING.get_or_init(|| ArcSwap::from_pointee(WafRuntimeBinding::default()))
}

fn pending_binding_slot() -> &'static Mutex<Option<WafRuntimeBinding>> {
    PENDING_BINDING.get_or_init(|| Mutex::new(None))
}

/// Install the active binding (startup / tests). Also seeds `ServerState` construction.
pub fn install_waf_runtime_binding(binding: WafRuntimeBinding) {
    let _ = INSTALLED_WAF.set(Arc::clone(&binding.engine));
    match &binding.abuse {
        Some(g) => abuse_slot().store(Arc::new(Some(Arc::clone(g)))),
        None => abuse_slot().store(Arc::new(None)),
    }
    WAF_ENFORCE.store(binding.enforce, Ordering::SeqCst);
    current_binding_slot().store(Arc::new(binding));
    *pending_binding_slot().lock().expect("waf pending binding") = None;
}

/// Stage a candidate binding during Cap013 prepare (does not affect live generations).
pub fn stage_waf_runtime_binding(binding: WafRuntimeBinding) {
    *pending_binding_slot().lock().expect("waf pending binding") = Some(binding);
}

/// Binding captured into a newly built `ServerState` (pending if staged, else current).
pub fn waf_binding_for_new_state() -> WafRuntimeBinding {
    if let Some(pending) = pending_binding_slot()
        .lock()
        .expect("waf pending binding")
        .clone()
    {
        return pending;
    }
    (**current_binding_slot().load()).clone()
}

/// Promote staged binding to current after SharedServerState publish.
pub fn commit_staged_waf_binding(generation: u64) {
    let staged = pending_binding_slot()
        .lock()
        .expect("waf pending binding")
        .take();
    if let Some(binding) = staged {
        match &binding.abuse {
            Some(g) => abuse_slot().store(Arc::new(Some(Arc::clone(g)))),
            None => abuse_slot().store(Arc::new(None)),
        }
        WAF_ENFORCE.store(binding.enforce, Ordering::SeqCst);
        current_binding_slot().store(Arc::new(binding));
        info!(
            target: "exyonq_waf",
            generation,
            "waf runtime binding committed (generation-frozen)"
        );
    }
}

/// Drop staged binding when reload prepare fails.
pub fn discard_staged_waf_binding() {
    *pending_binding_slot().lock().expect("waf pending binding") = None;
}

/// Exact path for PoW verification (host early-dispatch).
pub const WAF_CHALLENGE_VERIFY_PATH: &str = "/.exyonq/waf-challenge";

/// Install the process WAF engine (composition root). Idempotent first-wins.
pub fn install_waf_engine(engine: Arc<dyn WafEngine>) {
    let _ = INSTALLED_WAF.set(engine);
}

/// Install or replace host abuse gate (quota/challenge). Reload-safe via ArcSwap.
pub fn install_waf_abuse_gate(gate: Arc<dyn WafAbuseGate>) {
    abuse_slot().store(Arc::new(Some(gate)));
    info!(target: "exyonq_waf", "waf abuse gate installed");
}

/// Clear abuse gate (abuse mode off after reload commit).
pub fn clear_waf_abuse_gate() {
    abuse_slot().store(Arc::new(None));
    info!(target: "exyonq_waf", "waf abuse gate cleared");
}

/// Install challenge issuer/verifier. Composition root only; first-wins.
pub fn install_waf_challenge_handler(handler: Arc<dyn WafChallengeHandler>) {
    let _ = INSTALLED_CHALLENGE.set(handler);
    info!(target: "exyonq_waf", "waf challenge handler installed");
}

/// Cap013: prepare WAF compile before publish. Failure must not swap live WAF.
pub fn install_waf_prepare_hook(hook: Arc<dyn Fn(&AppConfig) -> Result<(), String> + Send + Sync>) {
    let _ = WAF_PREPARE_HOOK.set(hook);
}

/// Cap013: commit prepared WAF after runtime generation publish.
pub fn install_waf_commit_hook(hook: Arc<dyn Fn(u64) + Send + Sync>) {
    let _ = WAF_COMMIT_HOOK.set(hook);
}

/// Cap013: discard prepared WAF when reload aborts after prepare.
pub fn install_waf_discard_hook(hook: Arc<dyn Fn() + Send + Sync>) {
    let _ = WAF_DISCARD_HOOK.set(hook);
}

/// Prepare WAF for a candidate config (Cap013). Active snapshot unchanged on Err.
pub fn prepare_waf_reload(config: &AppConfig) -> Result<(), String> {
    if let Some(hook) = WAF_PREPARE_HOOK.get() {
        hook(config)?;
    }
    Ok(())
}

/// Commit prepared WAF after successful publish.
pub fn commit_waf_reload(generation: u64) {
    if let Some(hook) = WAF_COMMIT_HOOK.get() {
        hook(generation);
    }
}

/// Discard prepared WAF (reload failed between prepare and commit).
pub fn discard_waf_reload() {
    if let Some(hook) = WAF_DISCARD_HOOK.get() {
        hook();
    }
}

/// Enable/disable host-side blocking (403/429). Independent of engine monitor/block mode.
pub fn set_waf_enforce(enabled: bool) {
    WAF_ENFORCE.store(enabled, Ordering::SeqCst);
    info!(target: "exyonq_waf", enforce = enabled, "waf host enforce updated");
}

/// Whether the host will reject on blocking WAF decisions.
pub fn waf_enforce_enabled() -> bool {
    WAF_ENFORCE.load(Ordering::SeqCst)
}

/// Engine used by new [`crate::server::state::ServerState`] instances.
pub fn current_waf_engine() -> Arc<dyn WafEngine> {
    INSTALLED_WAF
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(NopWafEngine))
}

#[allow(dead_code)] // available for Hyper HeaderMap paths
pub(crate) struct HeaderMapView<'a>(pub(crate) &'a HeaderMap);

impl HeaderView for HeaderMapView<'_> {
    fn get(&self, name: &str) -> Option<&[u8]> {
        self.0.get(name).map(|v| v.as_bytes())
    }

    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) {
        for (name, value) in self.0.iter() {
            f(name.as_str().as_bytes(), value.as_bytes());
        }
    }
}

pub(crate) struct PairsHeaderView<'a>(pub(crate) &'a [(String, String)]);

impl HeaderView for PairsHeaderView<'_> {
    fn get(&self, name: &str) -> Option<&[u8]> {
        let want = name.to_ascii_lowercase();
        self.0
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&want))
            .map(|(_, v)| v.as_bytes())
    }

    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) {
        for (n, v) in self.0 {
            f(n.as_bytes(), v.as_bytes());
        }
    }
}

/// Parse a trusted peer IP string (already host-resolved; may be `ip:port` for IPv4).
pub fn parse_client_ip(raw: Option<&str>) -> IpAddr {
    raw.and_then(|s| {
        let first = s.split(',').next().unwrap_or(s).trim();
        IpAddr::from_str(first).ok().or_else(|| {
            let (h, p) = first.rsplit_once(':')?;
            if p.chars().all(|c| c.is_ascii_digit()) {
                IpAddr::from_str(h).ok()
            } else {
                None
            }
        })
    })
    .unwrap_or_else(|| IpAddr::from_str("127.0.0.1").expect("localhost"))
}

fn emit_decision(phase: WafPhase, decision: &WafDecision) {
    if decision.violations.is_empty() && decision.action == WafAction::Allow {
        return;
    }
    for v in &decision.violations {
        warn!(
            target: "exyonq_waf",
            phase = ?phase,
            rule_id = %v.rule_id,
            action = ?decision.action,
            truncated = decision.inspection_truncated,
            field = ?v.evidence.field,
            enforce = waf_enforce_enabled(),
            message = %v.message,
            "waf violation"
        );
    }
    if decision.violations.is_empty() && decision.inspection_truncated {
        debug!(
            target: "exyonq_waf",
            phase = ?phase,
            action = ?decision.action,
            "waf inspection truncated"
        );
    }
}

/// Wire body for a host-side WAF reject.
#[derive(Debug, Clone)]
pub enum WafRejectBody {
    Static(&'static [u8]),
    Owned(Vec<u8>),
}

impl WafRejectBody {
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Static(b) => b,
            Self::Owned(b) => b.as_slice(),
        }
    }
}

/// Wire body for a host-side WAF reject (Challenge/RateLimit ≠ signature Block).
pub struct WafReject {
    pub status: http::StatusCode,
    pub body: WafRejectBody,
    pub content_type: &'static str,
    /// If set, add `Retry-After` header (seconds as decimal string).
    pub retry_after_secs: Option<u32>,
    pub set_cookie: Option<String>,
    pub location: Option<String>,
}

/// Result of a host-side WAF check.
pub enum WafHookResult {
    /// Continue the request path.
    Continue,
    /// Enforce: return this response.
    Reject(WafReject),
}

const TEXT_PLAIN: &str = "text/plain; charset=utf-8";
const BLOCK_BODY: &[u8] = b"blocked by waf\n";
const RATE_LIMIT_BODY: &[u8] = b"rate limited\n";

fn finish(
    phase: WafPhase,
    decision: WafDecision,
    client_ip: IpAddr,
    host: Option<&str>,
    path: &str,
    generation: u64,
    enforce: bool,
) -> WafHookResult {
    emit_decision(phase, &decision);
    if enforce && decision.is_blocking() {
        let reject = match decision.action {
            WafAction::RateLimit => WafReject {
                status: http::StatusCode::TOO_MANY_REQUESTS,
                body: WafRejectBody::Static(RATE_LIMIT_BODY),
                content_type: TEXT_PLAIN,
                retry_after_secs: decision.retry_after_secs.or(Some(1)),
                set_cookie: None,
                location: None,
            },
            WafAction::Challenge => {
                if let Some(handler) = INSTALLED_CHALLENGE.get() {
                    let issue = handler.issue(client_ip, host, path, generation);
                    WafReject {
                        status: http::StatusCode::from_u16(issue.status)
                            .unwrap_or(http::StatusCode::FORBIDDEN),
                        body: WafRejectBody::Owned(issue.body),
                        content_type: issue.content_type,
                        retry_after_secs: None,
                        set_cookie: None,
                        location: None,
                    }
                } else {
                    WafReject {
                        status: http::StatusCode::FORBIDDEN,
                        body: WafRejectBody::Static(b"challenge required\n"),
                        content_type: TEXT_PLAIN,
                        retry_after_secs: None,
                        set_cookie: None,
                        location: None,
                    }
                }
            }
            _ => WafReject {
                status: http::StatusCode::FORBIDDEN,
                body: WafRejectBody::Static(BLOCK_BODY),
                content_type: TEXT_PLAIN,
                retry_after_secs: None,
                set_cookie: None,
                location: None,
            },
        };
        return WafHookResult::Reject(reject);
    }
    WafHookResult::Continue
}

fn finish_abuse(
    phase: WafPhase,
    client_ip: IpAddr,
    host: Option<&str>,
    path: &str,
    route_id: Option<&str>,
    cookie_header: Option<&[u8]>,
    generation: u64,
    skip_abuse: bool,
    enforce: bool,
    abuse: Option<&dyn WafAbuseGate>,
) -> WafHookResult {
    if skip_abuse {
        return WafHookResult::Continue;
    }
    if let Some(handler) = INSTALLED_CHALLENGE.get() {
        if handler.grant_allows(client_ip, host, cookie_header, generation) {
            return WafHookResult::Continue;
        }
    }
    let Some(gate) = abuse else {
        return WafHookResult::Continue;
    };
    let Some(decision) = gate.check(client_ip, route_id) else {
        return WafHookResult::Continue;
    };
    debug_assert!(
        matches!(decision.action, WafAction::Challenge | WafAction::RateLimit),
        "abuse gate must return Challenge or RateLimit"
    );
    finish(phase, decision, client_ip, host, path, generation, enforce)
}

/// H1 / H2 / H4 / H5: header-phase inspection.
///
/// Precedence: signature inspect → Block (enforce) → else abuse (grant may skip) → Challenge/RateLimit.
pub(crate) fn inspect_request_headers(
    waf: &dyn WafEngine,
    method: &str,
    host: Option<&str>,
    path: &str,
    query: Option<&str>,
    headers: &dyn HeaderView,
    client_ip: IpAddr,
    route_id: Option<&str>,
    generation: u64,
    skip_abuse: bool,
    enforce: bool,
    abuse: Option<&dyn WafAbuseGate>,
) -> WafHookResult {
    let req = WafRequest {
        method,
        host,
        path,
        query,
        headers,
        client_ip,
        route_id,
    };
    let decision = waf.inspect(WafPhase::RequestHeaders, &req);
    match finish(
        WafPhase::RequestHeaders,
        decision,
        client_ip,
        host,
        path,
        generation,
        enforce,
    ) {
        reject @ WafHookResult::Reject(_) => reject,
        WafHookResult::Continue => {
            let cookie = headers.get("cookie");
            finish_abuse(
                WafPhase::RequestHeaders,
                client_ip,
                host,
                path,
                route_id,
                cookie,
                generation,
                skip_abuse,
                enforce,
                abuse,
            )
        }
    }
}

/// H3: body chunk / complete — does **not** run abuse gate.
pub(crate) fn inspect_request_body(
    waf: &dyn WafEngine,
    method: &str,
    host: Option<&str>,
    path: &str,
    query: Option<&str>,
    headers: &dyn HeaderView,
    client_ip: IpAddr,
    route_id: Option<&str>,
    body: &[u8],
    generation: u64,
    enforce: bool,
) -> WafHookResult {
    let req = WafRequest {
        method,
        host,
        path,
        query,
        headers,
        client_ip,
        route_id,
    };
    let mut state = BodyInspectionState::default();
    let decision = waf.inspect_body_chunk(&req, body, true, &mut state);
    finish(
        WafPhase::RequestBodyComplete,
        decision,
        client_ip,
        host,
        path,
        generation,
        enforce,
    )
}

/// Verify POST `/.exyonq/waf-challenge` — signature WAF still applies; abuse skipped.
pub(crate) fn handle_waf_challenge_verify(
    waf: &dyn WafEngine,
    method: &str,
    host: Option<&str>,
    headers: &dyn HeaderView,
    client_ip: IpAddr,
    body: &[u8],
    generation: u64,
    enforce: bool,
) -> WafReject {
    // Signature inspect first — Block still applies.
    let req = WafRequest {
        method,
        host,
        path: WAF_CHALLENGE_VERIFY_PATH,
        query: None,
        headers,
        client_ip,
        route_id: None,
    };
    let decision = waf.inspect(WafPhase::RequestHeaders, &req);
    if enforce && decision.action == WafAction::Block {
        emit_decision(WafPhase::RequestHeaders, &decision);
        return WafReject {
            status: http::StatusCode::FORBIDDEN,
            body: WafRejectBody::Static(BLOCK_BODY),
            content_type: TEXT_PLAIN,
            retry_after_secs: None,
            set_cookie: None,
            location: None,
        };
    }
    emit_decision(WafPhase::RequestHeaders, &decision);

    let Some(handler) = INSTALLED_CHALLENGE.get() else {
        return WafReject {
            status: http::StatusCode::FORBIDDEN,
            body: WafRejectBody::Static(b"challenge unavailable\n"),
            content_type: TEXT_PLAIN,
            retry_after_secs: None,
            set_cookie: None,
            location: None,
        };
    };
    match handler.verify_post(client_ip, host, body, generation) {
        ChallengeVerifyResult::Ok {
            status,
            content_type,
            body,
            set_cookie,
            location,
        } => WafReject {
            status: http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::OK),
            body: WafRejectBody::Owned(body),
            content_type,
            retry_after_secs: None,
            set_cookie: Some(set_cookie),
            location,
        },
        ChallengeVerifyResult::Fail {
            status,
            content_type,
            body,
        } => WafReject {
            status: http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::FORBIDDEN),
            body: WafRejectBody::Owned(body),
            content_type,
            retry_after_secs: None,
            set_cookie: None,
            location: None,
        },
    }
}

#[allow(dead_code)]
pub(crate) fn empty_headers() -> EmptyHeaders {
    EmptyHeaders
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_waf_api::{WafEvidence, WafRuleId, WafViolation};

    struct BlockingEngine;

    impl WafEngine for BlockingEngine {
        fn inspect(&self, phase: WafPhase, _req: &WafRequest<'_>) -> WafDecision {
            WafDecision {
                action: WafAction::Block,
                violations: vec![WafViolation {
                    rule_id: WafRuleId::new("EXY-TEST-BLOCK"),
                    phase,
                    message: "test".into(),
                    evidence: WafEvidence::default(),
                }],
                inspection_truncated: false,
                retry_after_secs: None,
            }
        }

        fn inspect_body_chunk(
            &self,
            req: &WafRequest<'_>,
            _chunk: &[u8],
            _end: bool,
            _state: &mut BodyInspectionState,
        ) -> WafDecision {
            self.inspect(WafPhase::RequestBodyComplete, req)
        }
    }

    /// Serializes tests that mutate process-global host WAF state (`WAF_ENFORCE` / abuse gate).
    static WAF_HOST_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn enforce_off_continues_on_block() {
        let _guard = WAF_HOST_TEST_LOCK.lock().expect("waf host test lock");
        let headers = EmptyHeaders;
        let r = inspect_request_headers(
            &BlockingEngine,
            "GET",
            None,
            "/",
            None,
            &headers,
            IpAddr::from_str("127.0.0.1").unwrap(),
            None,
            1,
            false,
            false,
            None,
        );
        assert!(matches!(r, WafHookResult::Continue));
    }

    #[test]
    fn enforce_on_rejects_block() {
        let _guard = WAF_HOST_TEST_LOCK.lock().expect("waf host test lock");
        let headers = EmptyHeaders;
        let r = inspect_request_headers(
            &BlockingEngine,
            "GET",
            None,
            "/",
            None,
            &headers,
            IpAddr::from_str("127.0.0.1").unwrap(),
            None,
            1,
            false,
            true,
            None,
        );
        let WafHookResult::Reject(reject) = r else {
            panic!("expected Reject");
        };
        assert_eq!(reject.status, http::StatusCode::FORBIDDEN);
        assert_eq!(reject.content_type, "text/plain; charset=utf-8");
        assert!(std::str::from_utf8(reject.body.as_slice())
            .unwrap()
            .contains("blocked by waf"));
        assert!(reject.retry_after_secs.is_none());
    }

    #[test]
    fn finish_abuse_rate_limit_is_429_with_retry_after() {
        let _guard = WAF_HOST_TEST_LOCK.lock().expect("waf host test lock");
        let r = finish(
            WafPhase::RequestHeaders,
            WafDecision::abuse_rate_limit(WafPhase::RequestHeaders),
            IpAddr::from_str("127.0.0.1").unwrap(),
            None,
            "/",
            1,
            true,
        );
        let WafHookResult::Reject(reject) = r else {
            panic!("expected Reject");
        };
        assert_eq!(reject.status, http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(reject.retry_after_secs, Some(1));
        assert_eq!(reject.content_type, "text/plain; charset=utf-8");
        assert!(std::str::from_utf8(reject.body.as_slice())
            .unwrap()
            .contains("rate limited"));
    }

    #[test]
    fn finish_abuse_challenge_without_handler_is_plain_403() {
        let _guard = WAF_HOST_TEST_LOCK.lock().expect("waf host test lock");
        let r = finish(
            WafPhase::RequestHeaders,
            WafDecision::abuse_challenge(WafPhase::RequestHeaders),
            IpAddr::from_str("127.0.0.1").unwrap(),
            None,
            "/",
            1,
            true,
        );
        let WafHookResult::Reject(reject) = r else {
            panic!("expected Reject");
        };
        assert_eq!(reject.status, http::StatusCode::FORBIDDEN);
        assert!(reject.retry_after_secs.is_none());
        // Without installed handler: plain text fallback (not static HTML stub page).
        assert!(!std::str::from_utf8(reject.body.as_slice())
            .unwrap()
            .contains("blocked by waf"));
    }

    /// Shared OnceLock gate for abuse path tests (mode 0 = pass).
    struct ConfigurableAbuseGate;
    static ABUSE_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    static ABUSE_CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    impl WafAbuseGate for ConfigurableAbuseGate {
        fn check(&self, _client_ip: IpAddr, _route_id: Option<&str>) -> Option<WafDecision> {
            ABUSE_CHECKS.fetch_add(1, Ordering::SeqCst);
            match ABUSE_MODE.load(Ordering::SeqCst) {
                1 => Some(WafDecision::abuse_rate_limit(WafPhase::RequestHeaders)),
                2 => Some(WafDecision::abuse_challenge(WafPhase::RequestHeaders)),
                _ => None,
            }
        }
    }

    #[test]
    fn headers_abuse_rate_limit_rejects_429() {
        let _guard = WAF_HOST_TEST_LOCK.lock().expect("waf host test lock");
        ABUSE_MODE.store(1, Ordering::SeqCst);
        let headers = EmptyHeaders;
        let gate = ConfigurableAbuseGate;
        let r = inspect_request_headers(
            &NopWafEngine,
            "GET",
            None,
            "/",
            None,
            &headers,
            IpAddr::from_str("203.0.113.9").unwrap(),
            None,
            1,
            false,
            true,
            Some(&gate),
        );
        ABUSE_MODE.store(0, Ordering::SeqCst);
        let WafHookResult::Reject(reject) = r else {
            panic!("expected Reject");
        };
        assert_eq!(reject.status, http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(reject.retry_after_secs, Some(1));
    }

    #[test]
    fn body_inspect_does_not_run_abuse_gate() {
        let _guard = WAF_HOST_TEST_LOCK.lock().expect("waf host test lock");
        ABUSE_MODE.store(1, Ordering::SeqCst);
        ABUSE_CHECKS.store(0, Ordering::SeqCst);
        let headers = EmptyHeaders;
        let r = inspect_request_body(
            &NopWafEngine,
            "POST",
            None,
            "/",
            None,
            &headers,
            IpAddr::from_str("127.0.0.1").unwrap(),
            None,
            b"ok",
            1,
            true,
        );
        ABUSE_MODE.store(0, Ordering::SeqCst);
        assert!(matches!(r, WafHookResult::Continue));
        assert_eq!(ABUSE_CHECKS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn skip_abuse_flag_bypasses_gate() {
        let _guard = WAF_HOST_TEST_LOCK.lock().expect("waf host test lock");
        ABUSE_MODE.store(1, Ordering::SeqCst);
        ABUSE_CHECKS.store(0, Ordering::SeqCst);
        let headers = EmptyHeaders;
        let gate = ConfigurableAbuseGate;
        let r = inspect_request_headers(
            &NopWafEngine,
            "POST",
            None,
            WAF_CHALLENGE_VERIFY_PATH,
            None,
            &headers,
            IpAddr::from_str("127.0.0.1").unwrap(),
            None,
            1,
            true,
            true,
            Some(&gate),
        );
        ABUSE_MODE.store(0, Ordering::SeqCst);
        assert!(matches!(r, WafHookResult::Continue));
        assert_eq!(ABUSE_CHECKS.load(Ordering::SeqCst), 0);
    }
}
