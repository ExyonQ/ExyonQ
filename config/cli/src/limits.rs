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
