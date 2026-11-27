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
//! `[geoip]` — offline MMDB country database configuration.
//!
//! The reader is intentionally provider-neutral: any database that exposes the
//! `country.iso_code` / `continent.code` records of a GeoLite2-Country-shaped
//! file works (MaxMind GeoLite2/GeoIP2 Country, IP2Location LITE DB1, DB-IP
//! Country and other MMDB-compatible products). There is no `provider` field
//! to pin operators to one vendor or licence.

use crate::ConfigError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_reload_interval() -> String {
    "24h".into()
}

/// Maximum accepted reload interval (30 days).
pub const MAX_GEOIP_RELOAD_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Behavior when the database has no usable record for a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GeoIpMissingPolicy {
    /// Country allowlists reject and country denylists do not match.
    #[default]
    FailClosed,
    /// Country policy is skipped when no country is known.
    FailOpen,
}

/// Top-level `[geoip]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeoIpConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Path to a GeoLite2-Country-compatible `.mmdb` file; relative paths
    /// resolve against the process
    /// working directory. Required when enabled.
    #[serde(default)]
    pub database: Option<PathBuf>,
    /// `30s`, `15m`, `24h`, `7d`. Reload is lazy on the first request after
    /// the interval; it never calls a network service.
    #[serde(default = "default_reload_interval")]
    pub reload_interval: String,
    #[serde(default)]
    pub on_missing_db: GeoIpMissingPolicy,
}

impl Default for GeoIpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            database: None,
            reload_interval: default_reload_interval(),
            on_missing_db: GeoIpMissingPolicy::FailClosed,
        }
    }
}

impl GeoIpConfig {
    /// Validate the table without touching the filesystem. The database format
    /// is checked when the runtime plan is compiled (`config test`).
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.enabled {
            return Ok(());
        }
        let database = self.database.as_ref().ok_or_else(|| {
            ConfigError::Parse("geoip.database is required when geoip.enabled = true".into())
        })?;
        if database.as_os_str().is_empty() {
            return Err(ConfigError::Parse(
                "geoip.database must not be empty".into(),
            ));
        }
        let seconds = parse_reload_interval(&self.reload_interval).ok_or_else(|| {
            ConfigError::Parse(
                "geoip.reload_interval must be a duration such as 30s, 15m, 24h or 7d".into(),
            )
        })?;
        if seconds == 0 || seconds > MAX_GEOIP_RELOAD_SECONDS {
            return Err(ConfigError::Parse(format!(
                "geoip.reload_interval must be between 1s and {MAX_GEOIP_RELOAD_SECONDS}s"
            )));
        }
        Ok(())
    }

    /// Parsed reload interval. Validation has already rejected bad values.
    pub fn reload_seconds(&self) -> Option<u64> {
        parse_reload_interval(&self.reload_interval)
    }
}

/// Parse a compact duration (`30s`, `15m`, `24h`, `7d`) into seconds.
pub fn parse_reload_interval(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw.len() < 2 {
        return None;
    }
    let (number, suffix) = raw.split_at(raw.len() - 1);
    let value: u64 = number.parse().ok()?;
    let multiplier = match suffix {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => return None,
    };
    value.checked_mul(multiplier)
}

/// True for a two-letter ISO 3166-1 alpha-2 code. Continent codes use the same
/// shape (ISO 3166-1 alpha-2 reserved ranges).
pub fn is_country_code(value: &str) -> bool {
    let value = value.trim();
    value.len() == 2 && value.bytes().all(|b| b.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compact_durations() {
        assert_eq!(parse_reload_interval("30s"), Some(30));
        assert_eq!(parse_reload_interval("15m"), Some(900));
        assert_eq!(parse_reload_interval("24h"), Some(86_400));
        assert_eq!(parse_reload_interval("7d"), Some(604_800));
        assert_eq!(parse_reload_interval("1w"), None);
        assert_eq!(parse_reload_interval("0s"), Some(0));
    }

    #[test]
    fn enabled_requires_database_and_valid_interval() {
        let mut cfg = GeoIpConfig {
            enabled: true,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
        cfg.database = Some("/tmp/country.mmdb".into());
        cfg.validate().expect("valid geoip");
        cfg.reload_interval = "forever".into();
        assert!(cfg.validate().is_err());
        cfg.reload_interval = "0s".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn disabled_table_is_inert() {
        let cfg = GeoIpConfig::default();
        cfg.validate().expect("disabled");
    }

    #[test]
    fn country_codes_are_two_ascii_letters() {
        assert!(is_country_code("ES"));
        assert!(is_country_code("us"));
        assert!(!is_country_code("ESP"));
        assert!(!is_country_code("E1"));
        assert!(!is_country_code(""));
    }
}
