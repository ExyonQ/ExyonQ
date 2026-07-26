//! Reload / generation UX helpers (P1.4-WS5) — offline precheck + diff.
//!
//! Live publish remains core PREPARE→COMMIT→RETIRE. This module never publishes.

use crate::output::{sanitize_control_chars, CliExit, OutputFormat, ToolResult};
use exyonq_config_ir::{
    fingerprint as ir_fingerprint, AppConfig, Diagnostic, DiagnosticCode, DiagnosticDocument,
};
use exyonq_config_merge::load_with_includes;
use std::path::Path;

/// Operator-facing change class (P1.4: no DYNAMIC_INPLACE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeClass {
    SupportedGenerationReload,
    RestartRequired,
    Rejected,
    NoEffect,
}

impl ChangeClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SupportedGenerationReload => "SUPPORTED_GENERATION_RELOAD",
            Self::RestartRequired => "RESTART_REQUIRED",
            Self::Rejected => "REJECTED",
            Self::NoEffect => "NO_EFFECT",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub kind: String,
    pub class: ChangeClass,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ReloadPrecheck {
    pub ok: bool,
    pub candidate_ir_fingerprint: Option<String>,
    pub candidate_plan_fingerprint: Option<String>,
    pub diags: Vec<Diagnostic>,
    pub config: Option<AppConfig>,
}

/// Shared precheck: parse → validate → compile RuntimePlan → discard (no publish).
pub fn reload_precheck(path: &Path) -> ReloadPrecheck {
    let mut diags = Vec::new();
    let cfg = match load_app_for_reload(path) {
        Ok(c) => c,
        Err(d) => {
            diags.push(d);
            return ReloadPrecheck {
                ok: false,
                candidate_ir_fingerprint: None,
                candidate_plan_fingerprint: None,
                diags,
                config: None,
            };
        }
    };
    let ir_fp = ir_fingerprint(&cfg).as_str().to_string();
    match exyonq_runtime_plan::compile_runtime_plan_from_ir(0, cfg.clone()) {
        Ok(plan) => {
            let plan_fp = plan.fingerprint.as_str().to_string();
            drop(plan);
            ReloadPrecheck {
                ok: true,
                candidate_ir_fingerprint: Some(ir_fp),
                candidate_plan_fingerprint: Some(plan_fp),
                diags,
                config: Some(cfg),
            }
        }
        Err(e) => {
            diags.push(
                Diagnostic::error(
                    DiagnosticCode::ReloadCompileRejected,
                    sanitize_control_chars(&e.to_string()),
                )
                .with_documentation("docs/config/diagnostic-codes.md#exy-reload-0003"),
            );
            ReloadPrecheck {
                ok: false,
                candidate_ir_fingerprint: Some(ir_fp),
                candidate_plan_fingerprint: None,
                diags,
                config: Some(cfg),
            }
        }
    }
}

fn load_app_for_reload(path: &Path) -> Result<AppConfig, Diagnostic> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "exy" {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            Diagnostic::error(
                DiagnosticCode::ReloadParseRejected,
                format!("read {}: {e}", path.display()),
            )
        })?;
        let toml = exyonq_config_surface::compile_serverfile(
            &raw,
            exyonq_config_surface::CompileOptions::default(),
        )
        .map_err(|e| {
            let mut d = e.to_diagnostic();
            d.code = DiagnosticCode::ReloadParseRejected;
            d
        })?;
        AppConfig::parse_str(&toml).map_err(|e| {
            let mut d = e.to_diagnostic();
            if d.code == DiagnosticCode::UnknownField || d.code == DiagnosticCode::IrValidationError
            {
                d.code = DiagnosticCode::ReloadValidationRejected;
            } else {
                d.code = DiagnosticCode::ReloadParseRejected;
            }
            d
        })
    } else {
        load_with_includes(path).map_err(|e| {
            let mut d = e.to_diagnostic();
            match d.code {
                DiagnosticCode::UnknownField
                | DiagnosticCode::IrValidationError
                | DiagnosticCode::InvalidValue
                | DiagnosticCode::MissingRequiredField => {
                    d.code = DiagnosticCode::ReloadValidationRejected;
                }
                DiagnosticCode::IncludeCycle | DiagnosticCode::IncludeNotFound => {}
                _ => {
                    d.code = DiagnosticCode::ReloadParseRejected;
                }
            }
            d
        })
    }
}

/// Offline diff vs optional live fingerprint (from `status`). Classifies listener/H3 limits.
pub fn classify_reload_diff(
    path: &Path,
    current_fingerprint: Option<&str>,
    current_generation: Option<u64>,
) -> ToolResult {
    let pre = reload_precheck(path);
    if !pre.ok {
        return finish_reload_tool(pre.diags, CliExit::DiagnosticError, OutputFormat::Human, None);
    }
    let candidate_fp = pre.candidate_plan_fingerprint.clone().unwrap_or_default();
    let mut entries = Vec::new();
    if let Some(cur) = current_fingerprint {
        if cur == candidate_fp {
            entries.push(DiffEntry {
                kind: "fingerprint".into(),
                class: ChangeClass::NoEffect,
                detail: "identical plan fingerprint — IDENTICAL_RELOAD=NO_OP".into(),
            });
        } else {
            entries.push(DiffEntry {
                kind: "fingerprint".into(),
                class: ChangeClass::SupportedGenerationReload,
                detail: format!("live={cur} candidate={candidate_fp}"),
            });
        }
    } else {
        entries.push(DiffEntry {
            kind: "fingerprint".into(),
            class: ChangeClass::SupportedGenerationReload,
            detail: format!("candidate={candidate_fp} (no live fingerprint supplied)"),
        });
    }

    if let Some(cfg) = &pre.config {
        for server in &cfg.servers {
            if server.http3_listen.is_some() {
                entries.push(DiffEntry {
                    kind: "http3_listen".into(),
                    class: ChangeClass::SupportedGenerationReload,
                    detail: "H3_RELOAD_SUPPORT=SAME_LISTENER_CONFIG_ONLY — same-listener config-only may apply via generation reload; address/listener/protocol-enable change → RESTART_REQUIRED (EXY-RELOAD-0005)".into(),
                });
            }
            if server.tls.is_some() {
                entries.push(DiffEntry {
                    kind: "tls".into(),
                    class: ChangeClass::SupportedGenerationReload,
                    detail: "TLS material change supported via generation reload when paths valid".into(),
                });
            }
        }
        entries.push(DiffEntry {
            kind: "routes".into(),
            class: ChangeClass::SupportedGenerationReload,
            detail: format!("route_count={}", cfg.routes.len()),
        });
        entries.push(DiffEntry {
            kind: "upstreams".into(),
            class: ChangeClass::SupportedGenerationReload,
            detail: format!("upstream_count={}", cfg.upstreams.len()),
        });
        if !cfg.pools_fcgi.is_empty() {
            entries.push(DiffEntry {
                kind: "fcgi_pool".into(),
                class: ChangeClass::SupportedGenerationReload,
                detail: format!("pool_count={}", cfg.pools_fcgi.len()),
            });
        }
    }

    let mut out = String::new();
    if let Some(g) = current_generation {
        out.push_str(&format!("current_generation = {g}\n"));
    }
    if let Some(cur) = current_fingerprint {
        out.push_str(&format!("current_fingerprint = {cur}\n"));
    }
    out.push_str(&format!("candidate_fingerprint = {candidate_fp}\n"));
    if let Some(ir) = &pre.candidate_ir_fingerprint {
        out.push_str(&format!("candidate_ir_fingerprint = {ir}\n"));
    }
    out.push_str("IDENTICAL_RELOAD = NO_OP\n");
    out.push_str("DYNAMIC_INPLACE = FORBIDDEN\n");
    out.push_str("H3_RELOAD_SUPPORT = SAME_LISTENER_CONFIG_ONLY\n");
    for e in &entries {
        out.push_str(&format!(
            "change[{}] class={} detail={}\n",
            e.kind,
            e.class.as_str(),
            sanitize_control_chars(&e.detail)
        ));
    }
    ToolResult {
        exit: CliExit::Ok,
        stdout: out,
        stderr: String::new(),
    }
}

pub fn reload_check(path: &Path, format: OutputFormat) -> ToolResult {
    let pre = reload_precheck(path);
    let exit = if pre.ok {
        CliExit::Ok
    } else {
        CliExit::DiagnosticError
    };
    finish_reload_tool(pre.diags, exit, format, if pre.ok {
        Some("reload --check OK (no publish)\n".into())
    } else {
        None
    })
}

fn finish_reload_tool(
    diags: Vec<Diagnostic>,
    exit: CliExit,
    format: OutputFormat,
    ok_note: Option<String>,
) -> ToolResult {
    let diags: Vec<Diagnostic> = diags
        .into_iter()
        .map(|mut d| {
            d.message = sanitize_control_chars(&exyonq_config_ir::redact_secrets(&d.message));
            if let Some(v) = d.received.take() {
                d.received = Some(sanitize_control_chars(&exyonq_config_ir::redact_secrets(&v)));
            }
            if let Some(v) = d.expected.take() {
                d.expected = Some(sanitize_control_chars(&exyonq_config_ir::redact_secrets(&v)));
            }
            if let Some(v) = d.cause.take() {
                d.cause = Some(sanitize_control_chars(&exyonq_config_ir::redact_secrets(&v)));
            }
            if let Some(v) = d.suggestion.take() {
                d.suggestion = Some(sanitize_control_chars(&exyonq_config_ir::redact_secrets(&v)));
            }
            if let Some(v) = d.field.take() {
                d.field = Some(sanitize_control_chars(&v));
            }
            d
        })
        .collect();
    match format {
        OutputFormat::Json => {
            let doc = DiagnosticDocument::new(diags);
            ToolResult {
                exit,
                stdout: doc.to_json_string().unwrap_or_else(|_| "{}".into()),
                stderr: String::new(),
            }
        }
        OutputFormat::Human => {
            let mut human = exyonq_config_ir::render_human(&diags);
            if human.is_empty() {
                if let Some(n) = ok_note {
                    human = n;
                }
            }
            ToolResult {
                exit,
                stdout: String::new(),
                stderr: human,
            }
        }
    }
}

/// Format a status-like document from control-plane fields (no secrets).
pub fn format_config_status(
    generation: u64,
    fingerprint: &str,
    source_config: Option<&Path>,
    draining: bool,
    reload_in_progress: bool,
    format: OutputFormat,
) -> ToolResult {
    let src = source_config
        .map(|p| sanitize_control_chars(&p.display().to_string()))
        .unwrap_or_else(|| "(unknown)".into());
    match format {
        OutputFormat::Human => ToolResult {
            exit: CliExit::Ok,
            stdout: format!(
                "current_generation = {generation}\nfingerprint = {fingerprint}\nsource_config = {src}\ndraining = {draining}\nreload_in_progress = {reload_in_progress}\nH3_RELOAD_SUPPORT = SAME_LISTENER_CONFIG_ONLY\nIDENTICAL_RELOAD = NO_OP\n"
            ),
            stderr: String::new(),
        },
        OutputFormat::Json => ToolResult {
            exit: CliExit::Ok,
            stdout: format!(
                "{{\n  \"schema_version\": 1,\n  \"current_generation\": {generation},\n  \"fingerprint\": \"{}\",\n  \"source_config\": \"{}\",\n  \"draining\": {draining},\n  \"reload_in_progress\": {reload_in_progress},\n  \"h3_reload_support\": \"SAME_LISTENER_CONFIG_ONLY\",\n  \"identical_reload\": \"NO_OP\"\n}}",
                json_escape(fingerprint),
                json_escape(&src)
            ),
            stderr: String::new(),
        },
    }
}

pub fn format_generation(generation: u64, fingerprint: &str, format: OutputFormat) -> ToolResult {
    match format {
        OutputFormat::Human => ToolResult {
            exit: CliExit::Ok,
            stdout: format!("generation = {generation}\nfingerprint = {fingerprint}\n"),
            stderr: String::new(),
        },
        OutputFormat::Json => ToolResult {
            exit: CliExit::Ok,
            stdout: format!(
                "{{\n  \"schema_version\": 1,\n  \"generation\": {generation},\n  \"fingerprint\": \"{}\"\n}}",
                json_escape(fingerprint)
            ),
            stderr: String::new(),
        },
    }
}

fn json_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            '\n' => "\\n".to_string(),
            c if c.is_control() => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn minimal() -> &'static str {
        r#"
config_version = 1
[[server]]
listen = "127.0.0.1:18080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#
    }

    #[test]
    fn precheck_ok_no_side_effects() {
        let dir = std::env::temp_dir().join(format!(
            "exyonq-ws5-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("ok.toml");
        fs::write(&p, minimal()).unwrap();
        let pre = reload_precheck(&p);
        assert!(pre.ok);
        assert!(pre.candidate_plan_fingerprint.is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn identical_diff_no_effect() {
        let dir = std::env::temp_dir().join(format!(
            "exyonq-ws5-id-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("ok.toml");
        fs::write(&p, minimal()).unwrap();
        let pre = reload_precheck(&p);
        let fp = pre.candidate_plan_fingerprint.unwrap();
        let r = classify_reload_diff(&p, Some(&fp), Some(3));
        assert_eq!(r.exit, CliExit::Ok);
        assert!(r.stdout.contains("NO_EFFECT") || r.stdout.contains("IDENTICAL_RELOAD"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn check_redacts_secretish_unknown_field() {
        let dir = std::env::temp_dir().join(format!(
            "exyonq-ws5-redact-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.toml");
        fs::write(
            &p,
            "config_version = 1\npassword=supersecret\n[[server]]\nlisten=\"127.0.0.1:1\"\nroutes=[\"r\"]\n[[route]]\nname=\"r\"\nmatch={path=\"/\"}\nroot=\"/t\"\n",
        )
        .unwrap();
        let r = reload_check(&p, OutputFormat::Human);
        assert_eq!(r.exit, CliExit::DiagnosticError);
        assert!(!r.stderr.contains("supersecret"));
        let _ = fs::remove_dir_all(dir);
    }
}
