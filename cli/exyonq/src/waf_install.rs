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
//! Composition-root WAF install (before `server::run`).
//!
//! Converts IR → [`exyonq_waf::WafCompileInput`], installs Cap013 prepare/commit
//! hooks, abuse gate, and real challenge handler. Core depends only on waf-api.

use exyonq_config_ir::{
    AppConfig, WafAbuseMode, WafCaptchaProvider, WafConfig, WafDetectorModeIr, WafModeIr,
    WafOnInspectionLimitIr, WafPhaseIr, WafRuleActionIr,
};
use exyonq_core::WafRuntimeBinding;
use exyonq_ratelimit::SyncTokenBucket;
use exyonq_waf::{
    BuiltinDetectors, ChallengeProvider, ChallengeService, DetectorToggle, ExclusionInput,
    FailPolicy, FixedWafEngine, IpFilterInput, MatchTarget, RuleInput, RulesetInput,
    WafCompileInput,
};
use exyonq_waf_api::{
    InspectionLimits, OnInspectionLimit, WafAbuseGate, WafAction, WafDecision, WafEngine, WafMode,
    WafPhase,
};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// EXQ-032: bounded per-IP violation table (mirrors the client throttle ceiling).
/// Active bans are never evicted; a new IP that cannot be tracked is still rate
/// limited, so the table cap cannot disable the abuse gate.
const MAX_BAN_TRACKED_CLIENTS: usize = 10_000;

/// Install native WAF + abuse + challenge before the server starts.
///
/// Fail-closed: when `waf.enabled` and compile fails → `Err` (do not silently nop).
/// Uses generation-frozen [`FixedWafEngine`] bindings (Cap013 / Cap015 WAF-LOGIC-P3-A).
pub fn install_native_waf_runtime(config: &AppConfig) -> anyhow::Result<()> {
    let challenge = ChallengeService::from_env_or_random_with_provider(challenge_provider(config))
        .map_err(|e| anyhow::anyhow!("waf challenge key: {e}"))?;
    exyonq_core::install_waf_challenge_handler(Arc::new(challenge));

    let binding = compile_runtime_binding(config)?;
    exyonq_core::install_waf_runtime_binding(binding.clone());
    install_cap013_hooks();
    info!(
        enforce = binding.enforce,
        "waf runtime binding installed (fixed snapshot; Cap013 prepare/commit)"
    );
    Ok(())
}

fn challenge_provider(config: &AppConfig) -> Option<ChallengeProvider> {
    let abuse = &config.waf.abuse;
    let site_key = abuse.site_key.as_deref()?.trim();
    if site_key.is_empty() {
        return None;
    }
    match abuse.provider {
        WafCaptchaProvider::Native => None,
        WafCaptchaProvider::Recaptcha => Some(ChallengeProvider::recaptcha(site_key)),
        WafCaptchaProvider::Hcaptcha => Some(ChallengeProvider::hcaptcha(site_key)),
        WafCaptchaProvider::RecaptchaV3 => Some(ChallengeProvider::recaptcha_v3(site_key)),
    }
}

fn compile_runtime_binding(config: &AppConfig) -> anyhow::Result<WafRuntimeBinding> {
    let (mode, enforce) = resolve_mode_and_enforce(config);
    let engine: Arc<dyn WafEngine> = if !config.waf.enabled || mode == WafMode::Disabled {
        let input = WafCompileInput {
            enabled: false,
            mode: WafMode::Disabled,
            ..Default::default()
        };
        Arc::new(
            FixedWafEngine::compile(input)
                .map_err(|e| anyhow::anyhow!("waf disabled snapshot compile: {e}"))?,
        )
    } else {
        let input = waf_compile_input_from_config(config)
            .map_err(|e| anyhow::anyhow!("waf IR convert: {e}"))?;
        let crs = waf_crs_rulesets_from_ir(&config.waf)
            .map_err(|e| anyhow::anyhow!("waf CRS load: {e}"))?;
        Arc::new(
            FixedWafEngine::compile_with_crs(input, &crs)
                .map_err(|e| anyhow::anyhow!("waf compile failed (fail closed): {e}"))?,
        )
    };
    let abuse = build_abuse_gate_from_config(config)?;
    // ARCH-002: same effective-active predicate as engine compile (+ independent abuse).
    let signature_active = config.waf.enabled && mode != WafMode::Disabled;
    let wire_inspection_active =
        exyonq_core::compute_wire_inspection_active(signature_active, abuse.is_some(), enforce);
    Ok(WafRuntimeBinding {
        engine,
        abuse,
        enforce,
        wire_inspection_active,
    })
}

fn install_cap013_hooks() {
    exyonq_core::install_waf_prepare_hook(Arc::new(|cfg: &AppConfig| {
        let binding = compile_runtime_binding(cfg).map_err(|e| e.to_string())?;
        exyonq_core::stage_waf_runtime_binding(binding);
        Ok(())
    }));
    exyonq_core::install_waf_commit_hook(Arc::new(|generation| {
        exyonq_core::commit_staged_waf_binding(generation);
    }));
    exyonq_core::install_waf_discard_hook(Arc::new(|| {
        exyonq_core::discard_staged_waf_binding();
    }));
}

/// Load CRS `SecRule` files declared by `[[waf.ruleset]] rules_file`.
pub fn waf_crs_rulesets_from_ir(
    waf: &WafConfig,
) -> Result<Vec<exyonq_waf::CrsRulesetInput>, String> {
    let mut out = Vec::new();
    for ruleset in &waf.rulesets {
        let Some(path) = &ruleset.rules_file else {
            continue;
        };
        let contents =
            std::fs::read_to_string(path).map_err(|e| format!("crs rules file `{path}`: {e}"))?;
        let base = std::path::Path::new(path).parent();
        let mut crs = exyonq_waf::parse_rule_file_with_base(
            &ruleset.id,
            &contents,
            ruleset.inbound_threshold.unwrap_or(5),
            base,
        );
        crs.detection_only |= ruleset.detection_only;
        crs.disabled_ids
            .extend(ruleset.disabled_rule_ids.iter().copied());
        out.push(crs);
    }
    Ok(out)
}

/// Convert product IR to engine compile input (CLI composition root).
pub fn waf_compile_input_from_config(config: &AppConfig) -> Result<WafCompileInput, String> {
    waf_compile_input_from_ir(&config.waf)
}

pub fn waf_compile_input_from_ir(waf: &WafConfig) -> Result<WafCompileInput, String> {
    let mode = env_mode_override().unwrap_or_else(|| map_mode_ir(waf.mode));

    let mut builtins = BuiltinDetectors::default();
    let mut rulesets = Vec::new();
    let mut builtins_from_ruleset = false;

    for rs in &waf.rulesets {
        if let Some(toggles) = &rs.builtins {
            builtins = map_builtins(toggles);
            builtins_from_ruleset = true;
        }
        let mut rules = Vec::new();
        for rule in &rs.rules {
            rules.push(RuleInput {
                id: rule.id.clone(),
                enabled: rule.enabled,
                phases: map_phases(&rule.phases),
                target: map_target(&rule.target)?,
                pattern: rule.pattern.clone(),
                action: map_rule_action(rule.action),
            });
        }
        rulesets.push(RulesetInput {
            id: rs.id.clone(),
            enabled: rs.enabled,
            rules,
        });
    }

    if !builtins_from_ruleset && waf.enabled {
        builtins = BuiltinDetectors::default();
    }
    if !waf.enabled {
        builtins = BuiltinDetectors {
            sql_injection: DetectorToggle::off(),
            xss: DetectorToggle::off(),
            path_traversal: DetectorToggle::off(),
            command_injection: DetectorToggle::off(),
            header_anomaly: DetectorToggle::off(),
            bot_ua: DetectorToggle::off(),
            ip_filter: IpFilterInput::default(),
        };
    }

    let exclusions = waf
        .exclusions
        .iter()
        .map(|ex| ExclusionInput {
            host: ex.host.clone(),
            path_prefix: ex.path_prefix.clone(),
            method: ex.method.clone(),
            rule_ids: ex.rule_ids.clone(),
        })
        .collect();

    Ok(WafCompileInput {
        enabled: waf.enabled && mode != WafMode::Disabled,
        mode,
        limits: InspectionLimits {
            max_body_bytes: waf.max_inspection_body_bytes as usize,
            on_limit: map_on_limit(waf.on_inspection_limit),
        },
        fail_policy: FailPolicy::ClosedForInvalidRules,
        normalize: Default::default(),
        builtins,
        rulesets,
        exclusions,
    })
}

fn resolve_mode_and_enforce(config: &AppConfig) -> (WafMode, bool) {
    let ir_mode = map_mode_ir(config.waf.mode);
    let mode = env_mode_override().unwrap_or(ir_mode);
    let enforce = resolve_enforce_with_env(mode == WafMode::Block, mode);
    (mode, enforce)
}

fn resolve_enforce_with_env(default_enforce: bool, mode: WafMode) -> bool {
    match std::env::var("EXYONQ_WAF_ENFORCE") {
        Ok(v) if !v.is_empty() => {
            let requested = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
            if requested && mode != WafMode::Block {
                warn!(?mode, "EXYONQ_WAF_ENFORCE ignored unless WAF mode is block");
                false
            } else {
                requested && mode == WafMode::Block
            }
        }
        _ => default_enforce,
    }
}

fn env_mode_override() -> Option<WafMode> {
    let v = std::env::var("EXYONQ_WAF_MODE").ok()?;
    if v.is_empty() {
        return None;
    }
    Some(match v.to_ascii_lowercase().as_str() {
        "block" => WafMode::Block,
        "disabled" => WafMode::Disabled,
        "monitor" => WafMode::Monitor,
        other => {
            warn!(mode = other, "unknown EXYONQ_WAF_MODE; using IR");
            return None;
        }
    })
}

fn map_mode_ir(mode: WafModeIr) -> WafMode {
    match mode {
        WafModeIr::Monitor => WafMode::Monitor,
        WafModeIr::Block => WafMode::Block,
        WafModeIr::Disabled => WafMode::Disabled,
    }
}

fn map_on_limit(v: WafOnInspectionLimitIr) -> OnInspectionLimit {
    match v {
        WafOnInspectionLimitIr::AllowUninspected => OnInspectionLimit::AllowUninspected,
        WafOnInspectionLimitIr::Block => OnInspectionLimit::Block,
        WafOnInspectionLimitIr::LogAndAllow => OnInspectionLimit::LogAndAllow,
    }
}

fn map_rule_action(a: WafRuleActionIr) -> WafAction {
    match a {
        WafRuleActionIr::Allow => WafAction::Allow,
        WafRuleActionIr::Log => WafAction::Log,
        WafRuleActionIr::Block => WafAction::Block,
    }
}

fn map_phases(phases: &[WafPhaseIr]) -> Vec<WafPhase> {
    phases
        .iter()
        .map(|p| match p {
            WafPhaseIr::Connection => WafPhase::Connection,
            WafPhaseIr::RequestHeaders => WafPhase::RequestHeaders,
            WafPhaseIr::RequestBodyChunk => WafPhase::RequestBodyChunk,
            WafPhaseIr::RequestBodyComplete => WafPhase::RequestBodyComplete,
            WafPhaseIr::ResponseHeaders => WafPhase::ResponseHeaders,
            WafPhaseIr::ResponseBodyChunk => WafPhase::ResponseBodyChunk,
        })
        .collect()
}

fn map_target(target: &str) -> Result<MatchTarget, String> {
    let t = target.trim().to_ascii_lowercase();
    if t == "path" || t == "uri" {
        return Ok(MatchTarget::Path);
    }
    if t == "query" || t == "args" {
        return Ok(MatchTarget::Query);
    }
    if t == "body" {
        return Ok(MatchTarget::Body);
    }
    if let Some(name) = t.strip_prefix("header:") {
        return Ok(MatchTarget::Header(name.to_string()));
    }
    if let Some(name) = t.strip_prefix("hdr:") {
        return Ok(MatchTarget::Header(name.to_string()));
    }
    Err(format!("unsupported waf rule target `{target}`"))
}

fn map_detector(mode: Option<WafDetectorModeIr>, default: DetectorToggle) -> DetectorToggle {
    match mode {
        None => default,
        Some(WafDetectorModeIr::Off) => DetectorToggle::off(),
        Some(WafDetectorModeIr::Log) => DetectorToggle::log(),
        Some(WafDetectorModeIr::Block) => DetectorToggle::block(),
    }
}

fn map_builtins(t: &exyonq_config_ir::WafBuiltinToggles) -> BuiltinDetectors {
    let defaults = BuiltinDetectors::default();
    BuiltinDetectors {
        sql_injection: map_detector(t.sql_injection, defaults.sql_injection),
        xss: map_detector(t.xss, defaults.xss),
        path_traversal: map_detector(t.path_traversal, defaults.path_traversal),
        command_injection: map_detector(t.command_injection, defaults.command_injection),
        header_anomaly: map_detector(t.header_anomaly, defaults.header_anomaly),
        bot_ua: map_detector(t.bot_ua, defaults.bot_ua),
        ip_filter: t
            .ip_filter
            .as_ref()
            .map(|ip| IpFilterInput {
                enabled: ip.enabled,
                whitelist: ip.whitelist.clone(),
                blacklist: ip.blacklist.clone(),
            })
            .unwrap_or_default(),
    }
}

#[derive(Debug)]
struct ViolationEntry {
    window_start: Instant,
    count: u32,
    ban_until: Option<Instant>,
    last_seen: Instant,
}

/// EXQ-032: bounded per-IP counter of enforced WAF violations.
#[derive(Debug)]
struct ViolationTracker {
    window: Duration,
    threshold: u32,
    ban: Duration,
    clients: Mutex<HashMap<IpAddr, ViolationEntry>>,
}

impl ViolationTracker {
    fn new(window: Duration, threshold: u32, ban: Duration) -> Self {
        Self {
            window,
            threshold,
            ban,
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// Remaining ban for `client_ip`, if one is active at `now`.
    fn banned_for(&self, client_ip: IpAddr, now: Instant) -> Option<Duration> {
        let mut clients = self
            .clients
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = clients.get_mut(&client_ip)?;
        match entry.ban_until {
            Some(until) if until > now => Some(until - now),
            Some(_) => {
                // Ban elapsed: start a clean window for this IP.
                entry.ban_until = None;
                entry.count = 0;
                entry.window_start = now;
                None
            }
            None => None,
        }
    }

    fn record(&self, client_ip: IpAddr, now: Instant) {
        let mut clients = self
            .clients
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = clients.get_mut(&client_ip) {
            Self::note(entry, now, self.window, self.threshold, self.ban);
            return;
        }
        if clients.len() >= MAX_BAN_TRACKED_CLIENTS && !Self::evict_one(&mut clients, now) {
            // Every tracked row has an active ban: keep the bans and skip
            // counting this new IP instead of evicting a protected one.
            return;
        }
        let mut entry = ViolationEntry {
            window_start: now,
            count: 0,
            ban_until: None,
            last_seen: now,
        };
        Self::note(&mut entry, now, self.window, self.threshold, self.ban);
        clients.insert(client_ip, entry);
    }

    fn note(
        entry: &mut ViolationEntry,
        now: Instant,
        window: Duration,
        threshold: u32,
        ban: Duration,
    ) {
        entry.last_seen = now;
        if entry.ban_until.is_some_and(|until| until > now) {
            return;
        }
        entry.ban_until = None;
        if now.saturating_duration_since(entry.window_start) >= window {
            entry.window_start = now;
            entry.count = 0;
        }
        entry.count = entry.count.saturating_add(1);
        if entry.count >= threshold {
            entry.count = 0;
            entry.window_start = now;
            entry.ban_until = Some(now + ban);
        }
    }

    /// Evict the oldest row without an active ban. Returns false when every row
    /// is currently banned.
    fn evict_one(clients: &mut HashMap<IpAddr, ViolationEntry>, now: Instant) -> bool {
        let victim = clients
            .iter()
            .filter(|(_, entry)| entry.ban_until.is_none_or(|until| until <= now))
            .min_by_key(|(_, entry)| entry.last_seen)
            .map(|(ip, _)| *ip);
        match victim {
            Some(ip) => clients.remove(&ip).is_some(),
            None => false,
        }
    }
}

struct EnvAbuseGate {
    bucket: SyncTokenBucket,
    challenge: bool,
    violations: Option<ViolationTracker>,
}

impl EnvAbuseGate {
    fn check_at(&self, client_ip: IpAddr, now: Instant) -> Option<WafDecision> {
        if let Some(decision) = self.ban_decision(client_ip, now) {
            return Some(decision);
        }
        let key = client_ip.to_string();
        if self.bucket.allow_key(&key) {
            return None;
        }
        if self.challenge {
            Some(WafDecision::abuse_challenge(WafPhase::RequestHeaders))
        } else {
            let retry = self.bucket.retry_after_secs(&key).max(1) as u32;
            Some(WafDecision::abuse_rate_limit_with_retry(
                WafPhase::RequestHeaders,
                retry,
            ))
        }
    }

    fn ban_decision(&self, client_ip: IpAddr, now: Instant) -> Option<WafDecision> {
        let remaining = self.violations.as_ref()?.banned_for(client_ip, now)?;
        let retry = remaining.as_secs().max(1).min(u64::from(u32::MAX)) as u32;
        Some(WafDecision::abuse_ban(WafPhase::RequestHeaders, retry))
    }

    fn record_violation_at(&self, client_ip: IpAddr, now: Instant) {
        if let Some(tracker) = &self.violations {
            tracker.record(client_ip, now);
        }
    }
}

impl WafAbuseGate for EnvAbuseGate {
    fn check(&self, client_ip: IpAddr, _route_id: Option<&str>) -> Option<WafDecision> {
        self.check_at(client_ip, Instant::now())
    }

    fn check_ban(&self, client_ip: IpAddr) -> Option<WafDecision> {
        self.ban_decision(client_ip, Instant::now())
    }

    fn record_violation(&self, client_ip: IpAddr) {
        self.record_violation_at(client_ip, Instant::now());
    }
}

/// Build abuse gate from IR (+ env overrides). `None` means abuse off.
fn build_abuse_gate_from_config(
    config: &AppConfig,
) -> anyhow::Result<Option<Arc<dyn WafAbuseGate>>> {
    let ir = &config.waf.abuse;
    let mode_env = std::env::var("EXYONQ_WAF_ABUSE").unwrap_or_default();
    let mode = if !mode_env.is_empty() {
        match mode_env.to_ascii_lowercase().as_str() {
            "off" | "0" | "false" => WafAbuseMode::Off,
            "challenge" | "ch" => WafAbuseMode::Challenge,
            "rate_limit" | "ratelimit" | "rl" | "on" | "1" | "true" => WafAbuseMode::RateLimit,
            other => {
                return Err(anyhow::anyhow!(
                    "unknown EXYONQ_WAF_ABUSE={other}; expected off|rate_limit|challenge (fail closed)"
                ));
            }
        }
    } else {
        ir.mode
    };

    if mode == WafAbuseMode::Off {
        return Ok(None);
    }

    let rps: u32 = std::env::var("EXYONQ_WAF_ABUSE_RPS")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .unwrap_or(ir.requests_per_second);
    let burst: u32 = std::env::var("EXYONQ_WAF_ABUSE_BURST")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .unwrap_or(ir.burst);

    if rps < 1 || burst < 1 {
        return Err(anyhow::anyhow!(
            "waf.abuse rps/burst must be >= 1 when mode is not off"
        ));
    }

    let as_challenge = mode == WafAbuseMode::Challenge;

    // EXQ-032: violation counting/bans are opt-in (`violation_threshold = 0`
    // disables them) so existing abuse configurations keep their behaviour.
    let violations = if ir.violation_threshold > 0 {
        if ir.violation_window_seconds == 0 || ir.ban_seconds == 0 {
            return Err(anyhow::anyhow!(
                "waf.abuse.violation_window_seconds and ban_seconds must be >= 1"
            ));
        }
        Some(ViolationTracker::new(
            Duration::from_secs(ir.violation_window_seconds),
            ir.violation_threshold,
            Duration::from_secs(ir.ban_seconds),
        ))
    } else {
        None
    };

    info!(
        ?mode,
        rps,
        burst,
        violation_threshold = ir.violation_threshold,
        violation_window_seconds = ir.violation_window_seconds,
        ban_seconds = ir.ban_seconds,
        "waf abuse gate prepared (SyncTokenBucket; env overrides IR when set)"
    );
    Ok(Some(Arc::new(EnvAbuseGate {
        bucket: SyncTokenBucket::new(rps, burst),
        challenge: as_challenge,
        violations,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker(threshold: u32) -> ViolationTracker {
        ViolationTracker::new(Duration::from_secs(60), threshold, Duration::from_secs(300))
    }

    fn ip(raw: &str) -> IpAddr {
        raw.parse().expect("test ip")
    }

    fn generated(index: u32) -> IpAddr {
        IpAddr::V4(std::net::Ipv4Addr::from(
            (u32::from(10u8) << 24) | (index + 2),
        ))
    }

    #[test]
    fn twenty_violations_ban_only_that_ip() {
        let tracker = tracker(20);
        let t0 = Instant::now();
        let bad = ip("203.0.113.20");
        let good = ip("203.0.113.21");

        for i in 0..19 {
            tracker.record(bad, t0 + Duration::from_millis(i));
            assert_eq!(
                tracker.banned_for(bad, t0 + Duration::from_secs(1)),
                None,
                "ban must not trigger before the threshold"
            );
        }
        tracker.record(bad, t0 + Duration::from_secs(2));

        assert!(tracker
            .banned_for(bad, t0 + Duration::from_secs(2))
            .is_some());
        assert_eq!(tracker.banned_for(good, t0 + Duration::from_secs(2)), None);
    }

    #[test]
    fn ban_expires_and_does_not_extend() {
        let tracker = tracker(2);
        let t0 = Instant::now();
        let client = ip("198.51.100.7");

        tracker.record(client, t0);
        tracker.record(client, t0 + Duration::from_secs(1));
        assert_eq!(
            tracker.banned_for(client, t0 + Duration::from_secs(1)),
            Some(Duration::from_secs(300))
        );

        // A violating request during the ban must not extend it.
        tracker.record(client, t0 + Duration::from_secs(10));
        assert_eq!(
            tracker.banned_for(client, t0 + Duration::from_secs(301)),
            None
        );
    }

    #[test]
    fn window_expiry_resets_the_count() {
        let tracker = tracker(20);
        let t0 = Instant::now();
        let client = ip("192.0.2.50");

        for i in 0..10 {
            tracker.record(client, t0 + Duration::from_secs(i));
        }
        for i in 0..10 {
            tracker.record(client, t0 + Duration::from_secs(60 + i));
        }
        assert_eq!(
            tracker.banned_for(client, t0 + Duration::from_secs(70)),
            None
        );
    }

    #[test]
    fn violation_table_is_bounded() {
        let tracker = tracker(20);
        let t0 = Instant::now();
        let oldest = ip("10.0.0.1");
        tracker.record(oldest, t0 - Duration::from_secs(1));
        for i in 0..(MAX_BAN_TRACKED_CLIENTS as u32 - 1) {
            tracker.record(generated(i), t0 + Duration::from_millis(u64::from(i)));
        }
        tracker.record(ip("10.9.9.9"), t0 + Duration::from_secs(1));

        let clients = tracker.clients.lock().expect("tracker lock");
        assert_eq!(clients.len(), MAX_BAN_TRACKED_CLIENTS);
        assert!(
            !clients.contains_key(&oldest),
            "oldest unbanned row must be evicted"
        );
    }

    #[test]
    fn active_bans_survive_a_full_table() {
        let tracker = tracker(1);
        let t0 = Instant::now();
        let banned = ip("10.0.0.1");
        tracker.record(banned, t0);
        for i in 0..(MAX_BAN_TRACKED_CLIENTS as u32) {
            tracker.record(generated(i), t0 + Duration::from_millis(u64::from(i)));
        }

        let clients = tracker.clients.lock().expect("tracker lock");
        assert!(clients.len() <= MAX_BAN_TRACKED_CLIENTS);
        assert!(
            clients
                .get(&banned)
                .is_some_and(|entry| entry.ban_until.is_some()),
            "an active ban must never be evicted"
        );
    }

    #[test]
    fn gate_returns_ban_decision_with_retry_after() {
        let gate = EnvAbuseGate {
            bucket: SyncTokenBucket::new(1_000, 1_000),
            challenge: false,
            violations: Some(tracker(20)),
        };
        let t0 = Instant::now();
        let client = ip("203.0.113.30");
        for i in 0..20 {
            gate.record_violation_at(client, t0 + Duration::from_millis(i));
        }
        let banned_at = t0 + Duration::from_millis(19);
        let decision = gate.check_at(client, banned_at).expect("banned");
        assert_eq!(decision.action, WafAction::RateLimit);
        assert_eq!(
            decision.violations[0].rule_id.as_str(),
            exyonq_waf_api::RULE_ID_ABUSE_BAN
        );
        assert_eq!(decision.retry_after_secs, Some(300));
        assert!(gate.check_at(ip("203.0.113.31"), banned_at).is_none());
    }
}
