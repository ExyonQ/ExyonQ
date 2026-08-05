use exyonq_config_ir::redact_secrets;
use std::io::{self, Write};
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CliExit {
    Ok = 0,
    DiagnosticError = 1,
    UsageOrTool = 2,
}

impl From<CliExit> for ExitCode {
    fn from(value: CliExit) -> Self {
        ExitCode::from(value as u8)
    }
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub exit: CliExit,
    pub stdout: String,
    pub stderr: String,
}

pub fn emit_result(result: &ToolResult) -> ExitCode {
    if !result.stdout.is_empty() {
        let _ = io::stdout().write_all(result.stdout.as_bytes());
        if !result.stdout.ends_with('\n') {
            let _ = writeln!(io::stdout());
        }
    }
    if !result.stderr.is_empty() {
        let _ = io::stderr().write_all(result.stderr.as_bytes());
        if !result.stderr.ends_with('\n') {
            let _ = writeln!(io::stderr());
        }
    }
    result.exit.into()
}

/// Strip ASCII control chars (except tab/newline) from human diagnostic text.
pub fn sanitize_control_chars(input: &str) -> String {
    let redacted = redact_secrets(input);
    redacted
        .chars()
        .map(|c| {
            if c == '\n' || c == '\t' || !c.is_control() {
                c
            } else {
                ' '
            }
        })
        .collect()
}
