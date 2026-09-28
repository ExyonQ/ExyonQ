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
    let mut io_failed = false;
    if !result.stdout.is_empty() {
        io_failed |= write_stream_with_trailing_newline(&mut io::stdout(), &result.stdout);
    }
    if !result.stderr.is_empty() {
        io_failed |= write_stream_with_trailing_newline(&mut io::stderr(), &result.stderr);
    }
    if io_failed && result.exit == CliExit::Ok {
        return CliExit::DiagnosticError.into();
    }
    result.exit.into()
}

fn write_stream_with_trailing_newline(stream: &mut impl Write, text: &str) -> bool {
    if stream.write_all(text.as_bytes()).is_err() {
        return true;
    }
    if !text.ends_with('\n') && writeln!(stream).is_err() {
        return true;
    }
    false
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
