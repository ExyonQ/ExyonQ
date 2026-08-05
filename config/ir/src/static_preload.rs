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
//! Static tree-preload memory limits (`[static.preload]`).
//!
//! Authority: STATIC_PRELOAD_MEMORY_POLICY_DECISION.
//! Zero on any knob disables the whole preload build (not unlimited).

use serde::{Deserialize, Serialize};

/// Default per-file preload cap (32 MiB). Independent of H3 materialization budget.
pub const DEFAULT_PRELOAD_MAX_FILE_BYTES: u64 = 33_554_432;
/// Default total logical preload budget (256 MiB).
pub const DEFAULT_PRELOAD_TOTAL_BYTES: u64 = 268_435_456;
/// Default max accepted preload entries.
pub const DEFAULT_PRELOAD_MAX_ENTRIES: u64 = 4096;

/// Validation ceiling for `max_file_bytes` (256 MiB).
pub const PRELOAD_MAX_FILE_BYTES_CEILING: u64 = 268_435_456;
/// Validation ceiling for `max_total_bytes` (2 GiB).
pub const PRELOAD_MAX_TOTAL_BYTES_CEILING: u64 = 2_147_483_648;
/// Validation ceiling for `max_entries`.
pub const PRELOAD_MAX_ENTRIES_CEILING: u64 = 1_000_000;

/// Top-level `[static]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StaticSectionConfig {
    #[serde(default)]
    pub preload: StaticPreloadConfig,
}

/// `[static.preload]` tree-preload budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticPreloadConfig {
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
    #[serde(default = "default_max_total_bytes")]
    pub max_total_bytes: u64,
    #[serde(default = "default_max_entries")]
    pub max_entries: u64,
}

impl Default for StaticPreloadConfig {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_PRELOAD_MAX_FILE_BYTES,
            max_total_bytes: DEFAULT_PRELOAD_TOTAL_BYTES,
            max_entries: DEFAULT_PRELOAD_MAX_ENTRIES,
        }
    }
}

impl StaticPreloadConfig {
    /// Any zero disables the entire preload build (policy ZERO_MEANS_DISABLED).
    pub fn is_disabled(&self) -> bool {
        self.max_file_bytes == 0 || self.max_total_bytes == 0 || self.max_entries == 0
    }
}

fn default_max_file_bytes() -> u64 {
    DEFAULT_PRELOAD_MAX_FILE_BYTES
}

fn default_max_total_bytes() -> u64 {
    DEFAULT_PRELOAD_TOTAL_BYTES
}

fn default_max_entries() -> u64 {
    DEFAULT_PRELOAD_MAX_ENTRIES
}
