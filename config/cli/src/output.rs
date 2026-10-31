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
