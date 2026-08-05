//! Source locations and compile reports.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLoc {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileReport {
    pub executed: usize,
    pub parsed_only: usize,
    pub unknown: usize,
    pub errors: Vec<String>,
    pub diagnostics: Vec<String>,
}

impl CompileReport {
    pub fn push_diag(&mut self, msg: impl Into<String>) {
        self.diagnostics.push(msg.into());
    }
}
