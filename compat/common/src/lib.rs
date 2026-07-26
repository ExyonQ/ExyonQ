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
//! Shared migration IR and report types (Fase 4).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatLevel {
    Migrated,
    Approximated,
    Manual,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatNote {
    pub level: CompatLevel,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub source: String,
    #[serde(default)]
    pub notes: Vec<CompatNote>,
    #[serde(default)]
    pub unsupported: Vec<String>,
}

impl MigrationReport {
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# ExyonQ Compatibility Report\n\n");
        out.push_str(&format!("- Source: `{}`\n", self.source));
        out.push_str(&format!("- Notes: {}\n", self.notes.len()));
        out.push_str(&format!("- Unsupported: {}\n\n", self.unsupported.len()));

        out.push_str("## Notes\n\n");
        if self.notes.is_empty() {
            out.push_str("- None\n");
        } else {
            for note in &self.notes {
                out.push_str(&format!(
                    "- [{}] {}\n",
                    compat_level_label(note.level),
                    note.message
                ));
            }
        }

        out.push_str("\n## Unsupported\n\n");
        if self.unsupported.is_empty() {
            out.push_str("- None\n");
        } else {
            for item in &self.unsupported {
                out.push_str(&format!("- `{}`\n", item));
            }
        }

        out
    }

    pub fn render_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

fn compat_level_label(level: CompatLevel) -> &'static str {
    match level {
        CompatLevel::Migrated => "migrated",
        CompatLevel::Approximated => "approximated",
        CompatLevel::Manual => "manual",
        CompatLevel::Unsupported => "unsupported",
    }
}
