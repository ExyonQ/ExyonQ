//! `.exy` format (P1.4-WS3). TOML formatting is out of scope.

use crate::output::{CliExit, OutputFormat, ToolResult};
use exyonq_config_surface::fmt_serverfile;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatMode {
    Print,
    Write,
    Check,
}

#[derive(Debug, Clone)]
pub struct FormatRequest {
    pub path: PathBuf,
    pub mode: FormatMode,
    pub format: OutputFormat,
}

pub fn format_exy(req: FormatRequest) -> ToolResult {
    if req
        .path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.eq_ignore_ascii_case("exy"))
        != Some(true)
    {
        return ToolResult {
            exit: CliExit::UsageOrTool,
            stdout: String::new(),
            stderr: "config format: only .exy is supported in WS3\n".into(),
        };
    }
    if req.path.is_symlink() && matches!(req.mode, FormatMode::Write) {
        return ToolResult {
            exit: CliExit::UsageOrTool,
            stdout: String::new(),
            stderr: "config format --write: refusing to follow symlink\n".into(),
        };
    }

    let raw = match fs::read_to_string(&req.path) {
        Ok(s) => s,
        Err(e) => {
            return ToolResult {
                exit: CliExit::UsageOrTool,
                stdout: String::new(),
                stderr: format!("read {}: {e}\n", req.path.display()),
            };
        }
    };

    let formatted = match fmt_serverfile(&raw) {
        Ok(s) => s,
        Err(e) => {
            return ToolResult {
                exit: CliExit::DiagnosticError,
                stdout: String::new(),
                stderr: format!("error[EXY-CONFIG-0009]: {e}\n"),
            };
        }
    };

    if let Err(e) = exyonq_config_surface::parse_serverfile(&formatted) {
        return ToolResult {
            exit: CliExit::DiagnosticError,
            stdout: String::new(),
            stderr: format!("error[EXY-CONFIG-0009]: format output failed re-parse: {e}\n"),
        };
    }

    match req.mode {
        FormatMode::Print => match req.format {
            OutputFormat::Human => ToolResult {
                exit: CliExit::Ok,
                stdout: formatted,
                stderr: String::new(),
            },
            OutputFormat::Json => {
                let changed = formatted != raw;
                let escaped = json_escape(&formatted);
                ToolResult {
                    exit: CliExit::Ok,
                    stdout: format!(
                        "{{\n  \"schema_version\": 1,\n  \"formatted\": \"{escaped}\",\n  \"changed\": {changed}\n}}"
                    ),
                    stderr: String::new(),
                }
            }
        },
        FormatMode::Check => {
            let changed = formatted != raw;
            if changed {
                ToolResult {
                    exit: CliExit::DiagnosticError,
                    stdout: String::new(),
                    stderr: format!("would reformat {}\n", req.path.display()),
                }
            } else {
                ToolResult {
                    exit: CliExit::Ok,
                    stdout: String::new(),
                    stderr: "format --check OK\n".into(),
                }
            }
        }
        FormatMode::Write => {
            if let Err(e) = atomic_write(&req.path, formatted.as_bytes()) {
                return ToolResult {
                    exit: CliExit::UsageOrTool,
                    stdout: String::new(),
                    stderr: format!("write {}: {e}\n", req.path.display()),
                };
            }
            ToolResult {
                exit: CliExit::Ok,
                stdout: String::new(),
                stderr: format!("wrote {}\n", req.path.display()),
            }
        }
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".exyonq-format-{}.tmp", nanos);
    let tmp_path = parent.join(tmp_name);
    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    match fs::rename(&tmp_path, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}
