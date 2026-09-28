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
//! Static tree-preload memory limits (`[static.preload]`) and encoding cache
//! (`[static.encoding_cache]`, ADR-046).
//!
//! Authority: STATIC_PRELOAD_MEMORY_POLICY_DECISION.
//! Zero on any preload knob disables the whole preload build (not unlimited).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

/// Default managed cache directory for Cap067 static encoding cache (ADR-046).
pub const DEFAULT_ENCODING_CACHE_DIR: &str = "/var/cache/exyonq/static-encoding";
/// OLS-parity default gzip/brotli quality for static cache (not hot-path dynamic level 1).
pub const DEFAULT_ENCODING_CACHE_LEVEL: u32 = 6;
/// Minimum source bytes before static encoding cache engages (OLS-like).
pub const DEFAULT_ENCODING_CACHE_MIN_BYTES: u64 = 300;
/// Maximum source bytes accepted into the static encoding cache (10 MiB).
pub const DEFAULT_ENCODING_CACHE_MAX_BYTES: u64 = 10_485_760;
/// Default max coded objects retained in the encoding cache directory.
pub const DEFAULT_ENCODING_CACHE_MAX_ENTRIES: u64 = 4096;
/// Default max total bytes of coded objects (1 GiB).
pub const DEFAULT_ENCODING_CACHE_MAX_TOTAL_BYTES: u64 = 1_073_741_824;
/// Hard ceiling for `max_bytes` (64 MiB) — unbounded cache objects forbidden.
pub const ENCODING_CACHE_MAX_BYTES_CEILING: u64 = 67_108_864;
/// Hard ceiling for compression level (flate2 0..=9 / brotli quality clamp).
pub const ENCODING_CACHE_LEVEL_CEILING: u32 = 11;
/// Hard ceiling for `max_entries`.
pub const ENCODING_CACHE_MAX_ENTRIES_CEILING: u64 = 1_000_000;
/// Hard ceiling for `max_total_bytes` (8 GiB).
pub const ENCODING_CACHE_MAX_TOTAL_BYTES_CEILING: u64 = 8_589_934_592;

/// Top-level `[static]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StaticSectionConfig {
    #[serde(default)]
    pub preload: StaticPreloadConfig,
    /// Cap067 static encoding cache (ADR-046). Default OFF.
    #[serde(default)]
    pub encoding_cache: StaticEncodingCacheConfig,
}

/// `[static.encoding_cache]` — compress-once disk cache for Cap067 sendfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticEncodingCacheConfig {
    /// Kill switch — default false (behavior identical to pre-ADR-046).
    #[serde(default)]
    pub enabled: bool,
    /// Managed cache directory (hex filenames only; no path traversal).
    #[serde(default = "default_encoding_cache_dir")]
    pub cache_dir: PathBuf,
    /// Compression level (gzip flate2 / brotli quality). Default 6 (OLS parity).
    #[serde(default = "default_encoding_cache_level")]
    pub level: u32,
    /// Skip encoding when source size is below this (default 300).
    #[serde(default = "default_encoding_cache_min_bytes")]
    pub min_bytes: u64,
    /// Skip encoding when source size is above this (default 10 MiB).
    #[serde(default = "default_encoding_cache_max_bytes")]
    pub max_bytes: u64,
    /// Refuse new cache entries when this many coded objects exist (ADR unbounded growth).
    #[serde(default = "default_encoding_cache_max_entries")]
    pub max_entries: u64,
    /// Refuse new cache entries when total coded bytes would exceed this.
    #[serde(default = "default_encoding_cache_max_total_bytes")]
    pub max_total_bytes: u64,
    /// Offer gzip static cache objects.
    #[serde(default = "default_true")]
    pub gzip: bool,
    /// Offer brotli (`br`) static cache objects (default false).
    #[serde(default)]
    pub brotli: bool,
}

impl Default for StaticEncodingCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cache_dir: PathBuf::from(DEFAULT_ENCODING_CACHE_DIR),
            level: DEFAULT_ENCODING_CACHE_LEVEL,
            min_bytes: DEFAULT_ENCODING_CACHE_MIN_BYTES,
            max_bytes: DEFAULT_ENCODING_CACHE_MAX_BYTES,
            max_entries: DEFAULT_ENCODING_CACHE_MAX_ENTRIES,
            max_total_bytes: DEFAULT_ENCODING_CACHE_MAX_TOTAL_BYTES,
            gzip: true,
            brotli: false,
        }
    }
}

fn default_encoding_cache_dir() -> PathBuf {
    PathBuf::from(DEFAULT_ENCODING_CACHE_DIR)
}

fn default_encoding_cache_level() -> u32 {
    DEFAULT_ENCODING_CACHE_LEVEL
}

fn default_encoding_cache_min_bytes() -> u64 {
    DEFAULT_ENCODING_CACHE_MIN_BYTES
}

fn default_encoding_cache_max_bytes() -> u64 {
    DEFAULT_ENCODING_CACHE_MAX_BYTES
}

fn default_encoding_cache_max_entries() -> u64 {
    DEFAULT_ENCODING_CACHE_MAX_ENTRIES
}

fn default_encoding_cache_max_total_bytes() -> u64 {
    DEFAULT_ENCODING_CACHE_MAX_TOTAL_BYTES
}

fn default_true() -> bool {
    true
}

/// `[static.preload]` tree-preload budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
