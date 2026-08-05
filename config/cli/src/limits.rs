//! Resource limits for offline config CLI (not hot path).

use exyonq_config_ir::{Diagnostic, DiagnosticCode};

pub const MAX_FILE_BYTES: u64 = exyonq_config_merge::MAX_INCLUDE_FILE_BYTES;
pub const MAX_INCLUDE_DEPTH: usize = exyonq_config_merge::MAX_INCLUDE_DEPTH;
pub const MAX_DIAGNOSTICS: usize = 256;

pub fn enforce_diag_cap(diags: &mut Vec<Diagnostic>) {
    if diags.len() > MAX_DIAGNOSTICS {
        diags.truncate(MAX_DIAGNOSTICS);
        diags.push(Diagnostic::error(
            DiagnosticCode::IrValidationError,
            format!("diagnostic cap reached ({MAX_DIAGNOSTICS}); further findings omitted"),
        ));
    }
}
