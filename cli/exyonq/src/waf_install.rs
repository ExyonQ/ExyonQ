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
    AppConfig, WafAbuseMode, WafConfig, WafDetectorModeIr, WafModeIr, WafOnInspectionLimitIr,
    WafPhaseIr, WafRuleActionIr,
};
use exyonq_core::WafRuntimeBinding;
use exyonq_ratelimit::SyncTokenBucket;
use exyonq_waf::{
    BuiltinDetectors, ChallengeService, DetectorToggle, ExclusionInput, FailPolicy, FixedWafEngine,
    IpFilterInput, MatchTarget, RuleInput, RulesetInput, WafCompileInput,
};
use exyonq_waf_api::{
    InspectionLimits, OnInspectionLimit, WafAbuseGate, WafAction, WafDecision, WafEngine, WafMode,
    WafPhase,
};
use std::net::IpAddr;
use std::sync::Arc;
use tracing::{info, warn};

/// Install native WAF + abuse + challenge before the server starts.
///
/// Fail-closed: when `waf.enabled` and compile fails → `Err` (do not silently nop).
/// Uses generation-frozen [`FixedWafEngine`] bindings (Cap013 / Cap015 WAF-LOGIC-P3-A).
pub fn install_native_waf_runtime(config: &AppConfig) -> anyhow::Result<()> {
    let challenge = ChallengeService::from_env_or_random()
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
        Arc::new(
            FixedWafEngine::compile(input)
                .map_err(|e| anyhow::anyhow!("waf compile failed (fail closed): {e}"))?,
        )
    };
    let abuse = build_abuse_gate_from_config(config)?;
    // ARCH-002: same effective-active predicate as engine compile (+ abuse under enforce).
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

    struct EnvAbuseGate {
        bucket: SyncTokenBucket,
        challenge: bool,
    }

    impl WafAbuseGate for EnvAbuseGate {
        fn check(&self, client_ip: IpAddr, _route_id: Option<&str>) -> Option<WafDecision> {
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
    }

    info!(
        ?mode,
        rps, burst, "waf abuse gate prepared (SyncTokenBucket; env overrides IR when set)"
    );
    Ok(Some(Arc::new(EnvAbuseGate {
        bucket: SyncTokenBucket::new(rps, burst),
        challenge: as_challenge,
    })))
}
