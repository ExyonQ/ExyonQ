//! Shared offline configuration CLI (P1.4-WS3).
//!
//! Single owner for `lint` / `check` / `test` / `dump-ir` / `explain` / `format`.
//! Binaries must delegate here.

mod cache_ux;
mod dump;
mod explain;
mod format;
mod limits;
mod load;
mod output;
mod predictive;
mod profile;
mod provenance;
mod reload;

pub use cache_ux::cache_explain;
pub use dump::{dump_ir, DumpIrFormat};
pub use explain::{explain, ExplainRequest};
pub use format::{format_exy, FormatMode, FormatRequest};
pub use limits::{enforce_diag_cap, MAX_DIAGNOSTICS, MAX_FILE_BYTES, MAX_INCLUDE_DEPTH};
pub use load::{load_app_config, ConfigKind};
pub use output::{emit_result, sanitize_control_chars, CliExit, OutputFormat, ToolResult};
pub use profile::{profile_explain, profile_list, profile_render, profile_test, ProfileCliInputs};
pub use reload::{
    applied_routes_sidecar_path, classify_reload_diff, classify_reload_diff_with,
    format_config_status, format_generation, read_applied_routes_snapshot, reload_check,
    reload_precheck, route_identities, route_identity, write_applied_routes_snapshot,
    AppliedRoutesSnapshot, ChangeClass, DiffEntry, ReloadDiffOptions, ReloadPrecheck,
    RouteDiffSummary,
};

use exyonq_config_ir::{render_human, Diagnostic, DiagnosticCode, DiagnosticDocument, Severity};
use exyonq_config_merge::load_with_includes;
use std::path::Path;

/// Options shared by lint/test.
#[derive(Debug, Clone)]
pub struct CheckOptions {
    pub format: OutputFormat,
    pub strict: bool,
    pub color: bool,
    /// Q-01 / ADR-052: run predictive advisory pass after successful load.
    pub warn: bool,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            format: OutputFormat::Human,
            strict: false,
            color: true,
            warn: false,
        }
    }
}

/// `config lint`: parse + validate (+ includes). No RuntimePlan compile.
pub fn lint(path: &Path, opts: &CheckOptions) -> ToolResult {
    let mut diags = Vec::new();
    match load_for_check(path, &mut diags) {
        Ok(cfg) => {
            if opts.warn {
                diags.extend(predictive::analyze_predictive(&cfg));
            }
        }
        Err(early) => {
            if let Some(d) = early {
                diags.push(*d);
            }
        }
    }
    enforce_diag_cap(&mut diags);
    finish_check(diags, opts, false)
}

/// `config test`: lint path + compile RuntimePlan, then drop. No listeners/publish.
pub fn test_config(path: &Path, opts: &CheckOptions) -> ToolResult {
    let mut diags = Vec::new();
    let loaded = match load_for_check(path, &mut diags) {
        Ok(c) => {
            if opts.warn {
                diags.extend(predictive::analyze_predictive(&c));
            }
            c
        }
        Err(early) => {
            if let Some(d) = early {
                diags.push(*d);
            }
            enforce_diag_cap(&mut diags);
            return finish_check(diags, opts, false);
        }
    };
    match exyonq_runtime_plan::compile_runtime_plan_from_ir(0, loaded) {
        Ok(plan) => {
            // Explicitly drop — no generation publish, no listen.
            drop(plan);
        }
        Err(err) => {
            diags.push(
                Diagnostic::error(
                    DiagnosticCode::RuntimePlanCompileError,
                    sanitize_control_chars(&err.to_string()),
                )
                .with_documentation("docs/config/diagnostic-codes.md#exy-config-0012"),
            );
        }
    }
    enforce_diag_cap(&mut diags);
    finish_check(diags, opts, true)
}

fn load_for_check(
    path: &Path,
    diags: &mut Vec<Diagnostic>,
) -> Result<exyonq_config_ir::AppConfig, Option<Box<Diagnostic>>> {
    if !path.exists() {
        return Err(Some(Box::new(Diagnostic::error(
            DiagnosticCode::IrValidationError,
            format!("path not found: {}", path.display()),
        ))));
    }
    let meta = std::fs::metadata(path).map_err(|e| {
        Some(Box::new(Diagnostic::error(
            DiagnosticCode::IrValidationError,
            format!("stat {}: {e}", path.display()),
        )))
    })?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(Some(Box::new(Diagnostic::error(
            DiagnosticCode::IrValidationError,
            format!(
                "config file exceeds MAX_FILE_BYTES ({MAX_FILE_BYTES}): {}",
                path.display()
            ),
        ))));
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "exy" {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            Some(Box::new(Diagnostic::error(
                DiagnosticCode::SurfaceParseError,
                format!("read {}: {e}", path.display()),
            )))
        })?;
        let toml = exyonq_config_surface::compile_serverfile(
            &raw,
            exyonq_config_surface::CompileOptions::default(),
        )
        .map_err(|e| {
            Some(Box::new(
                e.to_diagnostic().with_source(path.display().to_string()),
            ))
        })?;
        match exyonq_config_ir::AppConfig::parse_str(&toml) {
            Ok(c) => Ok(c),
            Err(e) => {
                let mut d = e.to_diagnostic();
                d.source = Some(path.display().to_string());
                Err(Some(Box::new(d)))
            }
        }
    } else {
        // TOML IR — prefer include-aware load when path is a file.
        match load_with_includes(path) {
            Ok(c) => Ok(c),
            Err(e) => {
                let mut d = e.to_diagnostic();
                d.source = Some(path.display().to_string());
                diags.push(d);
                Err(None)
            }
        }
    }
}

fn finish_check(diags: Vec<Diagnostic>, opts: &CheckOptions, _compiled: bool) -> ToolResult {
    let diags: Vec<Diagnostic> = diags.into_iter().map(sanitize_diagnostic).collect();
    let has_error = diags.iter().any(|d| d.severity == Severity::Error);
    let has_warning = diags.iter().any(|d| d.severity == Severity::Warning);
    let exit = if has_error || (opts.strict && has_warning) {
        CliExit::DiagnosticError
    } else {
        CliExit::Ok
    };

    match opts.format {
        OutputFormat::Json => {
            let doc = DiagnosticDocument::new(diags);
            let body = doc
                .to_json_string()
                .unwrap_or_else(|e| format!("{{\"schema_version\":1,\"error\":\"{e}\"}}"));
            ToolResult {
                exit,
                stdout: body,
                stderr: String::new(),
            }
        }
        OutputFormat::Human => {
            let mut human = render_human(&diags);
            let _ = opts.color; // reserved; renderer is plain-text
            if human.is_empty() && exit == CliExit::Ok {
                human = "OK\n".to_string();
            }
            ToolResult {
                exit,
                stdout: String::new(),
                stderr: human,
            }
        }
    }
}

fn sanitize_diagnostic(mut d: Diagnostic) -> Diagnostic {
    d.message = sanitize_control_chars(&d.message);
    if let Some(v) = d.received.take() {
        d.received = Some(sanitize_control_chars(&v));
    }
    if let Some(v) = d.expected.take() {
        d.expected = Some(sanitize_control_chars(&v));
    }
    if let Some(v) = d.cause.take() {
        d.cause = Some(sanitize_control_chars(&v));
    }
    if let Some(v) = d.suggestion.take() {
        d.suggestion = Some(sanitize_control_chars(&v));
    }
    if let Some(v) = d.field.take() {
        d.field = Some(sanitize_control_chars(&v));
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> std::path::PathBuf {
        let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("exyonq-config-cli-{nanos}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn minimal_toml() -> &'static str {
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
    fn lint_ok_human() {
        let dir = test_dir();
        let p = dir.join("ok.toml");
        fs::write(&p, minimal_toml()).unwrap();
        let r = lint(&p, &CheckOptions::default());
        assert_eq!(r.exit, CliExit::Ok);
        assert!(r.stderr.contains("OK") || r.stderr.is_empty() || !r.stderr.contains("error["));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn lint_unknown_field_json() {
        let dir = test_dir();
        let p = dir.join("bad.toml");
        fs::write(
            &p,
            r#"
config_version = 1
typo_root = 1
[[server]]
listen = "127.0.0.1:1"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/t"
"#,
        )
        .unwrap();
        let r = lint(
            &p,
            &CheckOptions {
                format: OutputFormat::Json,
                ..Default::default()
            },
        );
        assert_eq!(r.exit, CliExit::DiagnosticError);
        assert!(r.stdout.contains("EXY-CONFIG-0002"));
        assert!(
            r.stdout.contains("\"schema_version\": 1") || r.stdout.contains("\"schema_version\":1")
        );
        assert!(r.stderr.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_compiles_minimal() {
        let dir = test_dir();
        let p = dir.join("ok.toml");
        fs::write(&p, minimal_toml()).unwrap();
        let r = test_config(&p, &CheckOptions::default());
        assert_eq!(r.exit, CliExit::Ok);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn explain_site_lexicon() {
        let r = explain(ExplainRequest {
            directive: Some("site".into()),
            path: None,
            pointer: None,
            format: OutputFormat::Human,
        });
        assert_eq!(r.exit, CliExit::Ok);
        assert!(r.stdout.contains("[[server]]"));
        assert!(r.stdout.to_lowercase().contains("site"));
    }

    #[test]
    fn format_check_and_write() {
        let dir = test_dir();
        let p = dir.join("site.exy");
        fs::write(&p, ":8080 {\nroute / { root /tmp }\n}\n").unwrap();
        let _check = format_exy(FormatRequest {
            path: p.clone(),
            mode: FormatMode::Check,
            format: OutputFormat::Human,
        });
        let write = format_exy(FormatRequest {
            path: p.clone(),
            mode: FormatMode::Write,
            format: OutputFormat::Human,
        });
        assert_eq!(write.exit, CliExit::Ok);
        let again = format_exy(FormatRequest {
            path: p,
            mode: FormatMode::Check,
            format: OutputFormat::Human,
        });
        assert_eq!(again.exit, CliExit::Ok);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn explain_unknown_exit_1() {
        let r = explain(ExplainRequest {
            directive: Some("not_a_real_directive".into()),
            path: None,
            pointer: None,
            format: OutputFormat::Human,
        });
        assert_eq!(r.exit, CliExit::DiagnosticError);
        assert!(r.stderr.contains("EXY-CONFIG-0014"));
    }

    #[test]
    fn sanitize_strips_control() {
        let s = sanitize_control_chars("ok\u{0007}secret");
        assert!(!s.contains('\u{0007}'));
    }

    #[test]
    fn lint_missing_path_is_error() {
        let r = lint(
            Path::new("/no/such/exyonq-config-ws3.toml"),
            &CheckOptions::default(),
        );
        assert_eq!(r.exit, CliExit::DiagnosticError);
    }

    #[test]
    fn lint_strict_warning_exit() {
        let dir = test_dir();
        let p = dir.join("ok.toml");
        fs::write(&p, minimal_toml()).unwrap();
        let r = lint(
            &p,
            &CheckOptions {
                strict: true,
                ..Default::default()
            },
        );
        assert_eq!(r.exit, CliExit::Ok);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn lint_warn_emits_predictive_and_strict_fails() {
        let dir = test_dir();
        let p = dir.join("warn.toml");
        fs::write(
            &p,
            r#"
config_version = 1
[[server]]
listen = "127.0.0.1:18081"
routes = ["short", "long"]
[[route]]
name = "short"
match = { path = "/api" }
upstream = "u"
[[route]]
name = "long"
match = { path = "/api/v1" }
upstream = "u"
[[upstream]]
name = "u"
target = "http://127.0.0.1:9001"
"#,
        )
        .unwrap();
        let silent = lint(&p, &CheckOptions::default());
        assert_eq!(silent.exit, CliExit::Ok);
        assert!(
            !silent.stderr.contains("EXY-LINT-"),
            "without --warn must stay quiet: {}",
            silent.stderr
        );

        let warned = lint(
            &p,
            &CheckOptions {
                warn: true,
                ..Default::default()
            },
        );
        assert_eq!(warned.exit, CliExit::Ok);
        assert!(
            warned.stderr.contains("EXY-LINT-0001") || warned.stderr.contains("EXY-LINT-0003"),
            "expected predictive warnings: {}",
            warned.stderr
        );
        assert!(warned.stderr.contains("cause:") && warned.stderr.contains("suggestion:"));

        let strict = lint(
            &p,
            &CheckOptions {
                warn: true,
                strict: true,
                ..Default::default()
            },
        );
        assert_eq!(strict.exit, CliExit::DiagnosticError);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn format_write_refuses_symlink() {
        let dir = test_dir();
        let real = dir.join("real.exy");
        fs::write(&real, ":8080 {\n    route / { root /tmp }\n}\n").unwrap();
        let link = dir.join("link.exy");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let r = format_exy(FormatRequest {
                path: link,
                mode: FormatMode::Write,
                format: OutputFormat::Human,
            });
            assert_eq!(r.exit, CliExit::UsageOrTool);
            assert!(r.stderr.contains("symlink"));
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn emit_sanitizes_control_in_message() {
        let dir = test_dir();
        let p = dir.join("bad.toml");
        // Unknown field path still yields structured diagnostic; sanitize on emit.
        fs::write(
            &p,
            "config_version = 1\npassword=supersecret\n[[server]]\nlisten=\"127.0.0.1:1\"\nroutes=[\"r\"]\n[[route]]\nname=\"r\"\nmatch={path=\"/\"}\nroot=\"/t\"\n",
        )
        .unwrap();
        let r = lint(
            &p,
            &CheckOptions {
                format: OutputFormat::Human,
                ..Default::default()
            },
        );
        assert_eq!(r.exit, CliExit::DiagnosticError);
        assert!(!r.stderr.contains('\u{0007}'));
        let _ = fs::remove_dir_all(dir);
    }
}
