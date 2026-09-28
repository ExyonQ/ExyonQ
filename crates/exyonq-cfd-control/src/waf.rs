//! Control-plane WAF projection: product `compile_waf` SSOT → CFDWAF01 bytes.
//!
//! Body-target rules → NOT_SUPPORTED_IN_PHASE3 (hard error).

use exyonq_cfd_gen::{
    CfdBuiltinToggles, CfdIpFilter, CfdToggle, CfdWafExclusion, CfdWafProjection, CfdWafRule,
    CompositeProjection, GenError, Generation, RouteTable, WafProjError,
};
use exyonq_waf::{
    compile_waf, BuiltinDetectors, DetectorToggle, ExclusionInput, IpFilterInput, MatchTarget,
    RuleInput, RulesetInput, WafCompileInput,
};
use exyonq_waf_api::{WafAction, WafMode, WafPhase};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WafProjectError {
    #[error("compile: {0}")]
    Compile(#[from] exyonq_waf::CompileError),
    #[error("projection: {0}")]
    Proj(#[from] WafProjError),
    #[error("body-target rule {0} is NOT_SUPPORTED_IN_PHASE3")]
    BodyDeferred(String),
    #[error("gen: {0}")]
    Gen(#[from] GenError),
}

/// Convert product compile input → wire projection (rejects body targets).
pub fn projection_from_compile_input(
    input: &WafCompileInput,
) -> Result<CfdWafProjection, WafProjectError> {
    // Validate with product compiler first (SSOT).
    let _ = compile_waf(input.clone())?;

    for rs in &input.rulesets {
        for r in &rs.rules {
            if matches!(r.target, MatchTarget::Body) {
                return Err(WafProjectError::BodyDeferred(r.id.clone()));
            }
        }
    }

    let mode = match input.mode {
        WafMode::Disabled => 0,
        WafMode::Monitor => 1,
        WafMode::Block => 2,
    };
    let builtins = CfdBuiltinToggles {
        sql_injection: toggle(&input.builtins.sql_injection),
        xss: toggle(&input.builtins.xss),
        path_traversal: toggle(&input.builtins.path_traversal),
        command_injection: toggle(&input.builtins.command_injection),
        header_anomaly: toggle(&input.builtins.header_anomaly),
        bot_ua: toggle(&input.builtins.bot_ua),
    };
    let ip_filter = CfdIpFilter {
        enabled: input.builtins.ip_filter.enabled,
        whitelist: input.builtins.ip_filter.whitelist.clone(),
        blacklist: input.builtins.ip_filter.blacklist.clone(),
    };
    let mut rules = Vec::new();
    for rs in &input.rulesets {
        if !rs.enabled {
            continue;
        }
        for r in &rs.rules {
            let (target_kind, header_name) = match &r.target {
                MatchTarget::Path => (0u8, String::new()),
                MatchTarget::Query => (1, String::new()),
                MatchTarget::Header(n) => (2, n.clone()),
                MatchTarget::Body => return Err(WafProjectError::BodyDeferred(r.id.clone())),
            };
            let mut phases_mask = 0u8;
            if r.phases.contains(&WafPhase::RequestHeaders) {
                phases_mask |= 1;
            }
            // Phase3 only encodes RequestHeaders; other phases ignored on wire.
            rules.push(CfdWafRule {
                id: r.id.clone(),
                enabled: r.enabled,
                phases_mask,
                target_kind,
                header_name,
                pattern: r.pattern.clone(),
                action: action_u8(r.action)?,
            });
        }
    }
    let exclusions = input
        .exclusions
        .iter()
        .map(|ex| CfdWafExclusion {
            host: ex.host.clone(),
            path_prefix: ex.path_prefix.clone(),
            method: ex.method.clone(),
            rule_ids: ex.rule_ids.clone(),
        })
        .collect();

    Ok(CfdWafProjection {
        enabled: input.enabled,
        mode,
        builtins,
        ip_filter,
        rules,
        exclusions,
    })
}

/// Rebuild product compile input from wire projection (dataplane / control).
pub fn compile_input_from_projection(
    p: &CfdWafProjection,
) -> Result<WafCompileInput, WafProjectError> {
    let mode = match p.mode {
        0 => WafMode::Disabled,
        1 => WafMode::Monitor,
        2 => WafMode::Block,
        _ => {
            return Err(WafProjectError::Proj(WafProjError::InvalidEnum));
        }
    };
    let mut rules = Vec::new();
    for r in &p.rules {
        if r.target_kind == 3 {
            return Err(WafProjectError::BodyDeferred(r.id.clone()));
        }
        let target = match r.target_kind {
            0 => MatchTarget::Path,
            1 => MatchTarget::Query,
            2 => MatchTarget::Header(r.header_name.clone()),
            _ => return Err(WafProjectError::Proj(WafProjError::InvalidEnum)),
        };
        let mut phases = Vec::new();
        if r.phases_mask & 1 != 0 {
            phases.push(WafPhase::RequestHeaders);
        }
        if phases.is_empty() {
            phases.push(WafPhase::RequestHeaders);
        }
        rules.push(RuleInput {
            id: r.id.clone(),
            enabled: r.enabled,
            phases,
            target,
            pattern: r.pattern.clone(),
            action: action_from_u8(r.action)?,
        });
    }
    let exclusions = p
        .exclusions
        .iter()
        .map(|ex| ExclusionInput {
            host: ex.host.clone(),
            path_prefix: ex.path_prefix.clone(),
            method: ex.method.clone(),
            rule_ids: ex.rule_ids.clone(),
        })
        .collect();

    let input = WafCompileInput {
        enabled: p.enabled,
        mode,
        builtins: BuiltinDetectors {
            sql_injection: toggle_from(&p.builtins.sql_injection)?,
            xss: toggle_from(&p.builtins.xss)?,
            path_traversal: toggle_from(&p.builtins.path_traversal)?,
            command_injection: toggle_from(&p.builtins.command_injection)?,
            header_anomaly: toggle_from(&p.builtins.header_anomaly)?,
            bot_ua: toggle_from(&p.builtins.bot_ua)?,
            ip_filter: IpFilterInput {
                enabled: p.ip_filter.enabled,
                whitelist: p.ip_filter.whitelist.clone(),
                blacklist: p.ip_filter.blacklist.clone(),
            },
        },
        rulesets: vec![RulesetInput {
            id: "cfd-phase3".into(),
            enabled: true,
            rules,
        }],
        exclusions,
        ..WafCompileInput::default()
    };
    // Re-validate SSOT.
    let _ = compile_waf(input.clone())?;
    Ok(input)
}

fn toggle(t: &DetectorToggle) -> CfdToggle {
    CfdToggle {
        enabled: t.enabled,
        action: match t.action {
            WafAction::Allow => 0,
            WafAction::Log => 1,
            _ => 2,
        },
    }
}

fn toggle_from(t: &CfdToggle) -> Result<DetectorToggle, WafProjectError> {
    Ok(DetectorToggle {
        enabled: t.enabled,
        action: action_from_u8(t.action)?,
    })
}

fn action_u8(a: WafAction) -> Result<u8, WafProjectError> {
    match a {
        WafAction::Allow => Ok(0),
        WafAction::Log => Ok(1),
        WafAction::Block => Ok(2),
        WafAction::Challenge | WafAction::RateLimit => {
            Err(WafProjectError::Proj(WafProjError::InvalidEnum))
        }
    }
}

fn action_from_u8(a: u8) -> Result<WafAction, WafProjectError> {
    match a {
        0 => Ok(WafAction::Allow),
        1 => Ok(WafAction::Log),
        2 => Ok(WafAction::Block),
        _ => Err(WafProjectError::Proj(WafProjError::InvalidEnum)),
    }
}

/// Representative nontrivial Phase-3 ruleset (Host/path/header/IP + builtins).
/// Allows normal `/api/` GET; denies known attack markers.
pub fn representative_phase3_waf_input() -> WafCompileInput {
    WafCompileInput {
        enabled: true,
        mode: WafMode::Block,
        builtins: BuiltinDetectors {
            sql_injection: DetectorToggle::block(),
            xss: DetectorToggle::block(),
            path_traversal: DetectorToggle::block(),
            command_injection: DetectorToggle::block(),
            header_anomaly: DetectorToggle::log(),
            bot_ua: DetectorToggle::log(),
            ip_filter: IpFilterInput {
                enabled: true,
                whitelist: vec![],
                // Test / adversarial deny IP (documentation); production peers rarely match.
                blacklist: vec!["203.0.113.99/32".into()],
            },
        },
        rulesets: vec![RulesetInput {
            id: "cfd-phase3-rep".into(),
            enabled: true,
            rules: vec![
                RuleInput {
                    id: "CFD-PATH-DENY-ADMIN".into(),
                    enabled: true,
                    phases: vec![WafPhase::RequestHeaders],
                    target: MatchTarget::Path,
                    pattern: "/admin/secret".into(),
                    action: WafAction::Block,
                },
                RuleInput {
                    id: "CFD-HDR-DENY-ATTACK".into(),
                    enabled: true,
                    phases: vec![WafPhase::RequestHeaders],
                    target: MatchTarget::Header("x-attack".into()),
                    pattern: "evil".into(),
                    action: WafAction::Block,
                },
                RuleInput {
                    id: "CFD-HOST-DENY".into(),
                    enabled: true,
                    phases: vec![WafPhase::RequestHeaders],
                    target: MatchTarget::Header("host".into()),
                    pattern: "blocked.example".into(),
                    action: WafAction::Block,
                },
            ],
        }],
        exclusions: vec![],
        ..WafCompileInput::default()
    }
}

pub fn publish_routes_and_waf(
    generation_id: u64,
    table: &RouteTable,
    waf: Option<&WafCompileInput>,
) -> Result<Generation, WafProjectError> {
    let waf_proj = match waf {
        Some(input) => Some(projection_from_compile_input(input)?),
        None => None,
    };
    let composite = CompositeProjection {
        routes: table.clone(),
        waf: waf_proj,
    };
    Ok(Generation::from_composite(generation_id, &composite)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representative_compiles_and_roundtrips() {
        let input = representative_phase3_waf_input();
        let proj = projection_from_compile_input(&input).unwrap();
        let back = compile_input_from_projection(&proj).unwrap();
        assert!(back.enabled);
        assert_eq!(back.mode, WafMode::Block);
        assert!(!back.rulesets[0].rules.is_empty());
    }
}
