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
//! Offline MMDB country reader.
//!
//! The reader is provider-neutral: it opens any database that exposes the
//! `country.iso_code` / `continent.code` records of a GeoLite2-Country-shaped
//! `.mmdb` (MaxMind GeoLite2/GeoIP2 Country, IP2Location LITE DB1, DB-IP
//! Country, ...). The file is read once into memory and requests never call an
//! external service; the mapped-reader optimization would need a new `unsafe`
//! block outside the ADR-045 FFI capsule, so it stays out of v1 (see
//! `docs/architecture/084-geoip-country-policy.md`).
//!
//! A reload interval makes the reader re-open the same path lazily on the
//! first lookup after the interval, so replacing the file is enough to refresh
//! country data without restarting the server. A failed reload keeps the
//! previous mapped database and records the error (fail closed for the country
//! policy is decided by the caller from [`GeoIp::country`] returning `None`).

use maxminddb::{geoip2, Reader};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

/// Country and continent resolved for one address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Country {
    /// ISO 3166-1 alpha-2, uppercase.
    pub iso_code: String,
    /// Two-letter continent code (`EU`, `NA`, ...), when present.
    pub continent_code: Option<String>,
}

impl Country {
    /// Country code for policy comparisons.
    pub fn code(&self) -> &str {
        &self.iso_code
    }
}

/// Metadata exposed by the reader for diagnostics and startup logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseInfo {
    pub database_type: String,
    pub build_epoch: u64,
    pub ip_version: u16,
    pub node_count: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum GeoIpError {
    #[error("cannot read GeoIP database {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid GeoIP database {path}: {source}")]
    Database {
        path: PathBuf,
        #[source]
        source: maxminddb::MaxMindDbError,
    },
}

#[derive(Debug)]
struct GeoIpDatabase {
    reader: Reader<Vec<u8>>,
    info: DatabaseInfo,
}

impl GeoIpDatabase {
    fn open(path: &Path) -> Result<Self, GeoIpError> {
        let reader = Reader::open_readfile(path).map_err(|source| GeoIpError::Database {
            path: path.to_path_buf(),
            source,
        })?;
        let metadata = reader.metadata();
        let info = DatabaseInfo {
            database_type: metadata.database_type.clone(),
            build_epoch: metadata.build_epoch,
            ip_version: metadata.ip_version,
            node_count: metadata.node_count,
        };
        Ok(Self { reader, info })
    }

    fn lookup(&self, ip: IpAddr) -> Option<Country> {
        let result = self.reader.lookup(ip).ok()?;
        if !result.has_data() {
            return None;
        }
        let record: geoip2::Country = result.decode().ok()??;
        let iso_code = record.country.iso_code?;
        if iso_code.len() != 2 || !iso_code.bytes().all(|b| b.is_ascii_alphabetic()) {
            return None;
        }
        Some(Country {
            iso_code: iso_code.to_ascii_uppercase(),
            continent_code: record
                .continent
                .code
                .filter(|code| code.len() == 2)
                .map(|code| code.to_ascii_uppercase()),
        })
    }
}

#[derive(Debug)]
struct State {
    database: Option<GeoIpDatabase>,
    last_error: Option<String>,
}

/// Loaded country database plus its lazy reload policy.
#[derive(Debug)]
pub struct GeoIp {
    path: PathBuf,
    reload_interval: Duration,
    fail_closed: bool,
    state: RwLock<State>,
    last_check: Mutex<Instant>,
}

impl GeoIp {
    /// Load the database once. An unreadable or invalid database is an error:
    /// an operator asking for GeoIP must not start with a silently empty one.
    pub fn open(
        path: PathBuf,
        reload_interval: Duration,
        fail_closed: bool,
    ) -> Result<Self, GeoIpError> {
        let database = GeoIpDatabase::open(&path)?;
        tracing::info!(
            path = %path.display(),
            database_type = %database.info.database_type,
            build_epoch = database.info.build_epoch,
            ip_version = database.info.ip_version,
            "GeoIP country database loaded"
        );
        Ok(Self {
            path,
            reload_interval,
            fail_closed,
            state: RwLock::new(State {
                database: Some(database),
                last_error: None,
            }),
            last_check: Mutex::new(Instant::now()),
        })
    }

    /// Country for `ip`, or `None` when the database has no record for it.
    /// The first lookup after the reload interval re-reads the file.
    pub fn country(&self, ip: IpAddr) -> Option<Country> {
        self.refresh_if_due();
        self.state
            .read()
            .ok()
            .and_then(|state| state.database.as_ref().and_then(|db| db.lookup(ip)))
    }

    /// True when a missing country must reject a country policy.
    pub fn fail_closed(&self) -> bool {
        self.fail_closed
    }

    /// Path currently watched.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Information about the loaded database. `None` if every load failed and
    /// the reader has no usable database (only possible after a reload).
    pub fn info(&self) -> Option<DatabaseInfo> {
        self.state
            .read()
            .ok()
            .and_then(|state| state.database.as_ref().map(|db| db.info.clone()))
    }

    /// Last reload error, retained for diagnostics.
    pub fn last_error(&self) -> Option<String> {
        self.state
            .read()
            .ok()
            .and_then(|state| state.last_error.clone())
    }

    /// Force a reload now; used by tests and diagnostics. A failure keeps the
    /// previous database and records the error.
    pub fn reload(&self) -> Result<(), GeoIpError> {
        match GeoIpDatabase::open(&self.path) {
            Ok(database) => {
                let info = database.info.clone();
                let mut state = self.state.write().map_err(|_| GeoIpError::Io {
                    path: self.path.clone(),
                    source: std::io::Error::other("GeoIP state lock poisoned"),
                })?;
                state.database = Some(database);
                state.last_error = None;
                tracing::info!(
                    path = %self.path.display(),
                    database_type = %info.database_type,
                    build_epoch = info.build_epoch,
                    "GeoIP country database reloaded"
                );
                Ok(())
            }
            Err(err) => {
                if let Ok(mut state) = self.state.write() {
                    state.last_error = Some(err.to_string());
                }
                tracing::warn!(error = %err, "GeoIP reload failed; keeping previous database");
                Err(err)
            }
        }
    }

    fn refresh_if_due(&self) {
        if self.reload_interval.is_zero() {
            return;
        }
        let Ok(mut last) = self.last_check.lock() else {
            return;
        };
        if last.elapsed() < self.reload_interval {
            return;
        }
        *last = Instant::now();
        drop(last);
        let _ = self.reload();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/GeoIP2-Country-Test.mmdb")
    }

    fn reader() -> GeoIp {
        GeoIp::open(fixture(), Duration::from_secs(3600), true).expect("open fixture")
    }

    #[test]
    fn resolves_country_and_continent_from_real_mmdb() -> Result<(), Box<dyn std::error::Error>> {
        let geo = reader();
        let gb: IpAddr = "81.2.69.160".parse()?;
        let found = geo.country(gb).expect("GB record");
        assert_eq!(found.code(), "GB");
        assert_eq!(found.continent_code.as_deref(), Some("EU"));

        let se: IpAddr = "89.160.20.128".parse()?;
        assert_eq!(geo.country(se).expect("SE record").code(), "SE");
        assert!(geo.country("192.0.2.1".parse()?).is_none());
        Ok(())
    }

    #[test]
    fn reload_reopens_replaced_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("country.mmdb");
        fs::copy(fixture(), &path)?;
        let geo = GeoIp::open(path.clone(), Duration::ZERO, true)?;
        assert_eq!(
            geo.country("81.2.69.160".parse()?)
                .expect("GB record")
                .code(),
            "GB"
        );

        // A truncated replacement must fail the reload but keep the old reader.
        fs::write(&path, b"not an mmdb")?;
        assert!(geo.reload().is_err());
        assert!(geo.last_error().is_some());
        assert!(geo.info().is_some());
        assert_eq!(
            geo.country("81.2.69.160".parse()?)
                .expect("GB record")
                .code(),
            "GB"
        );

        fs::copy(fixture(), &path)?;
        geo.reload()?;
        assert_eq!(
            geo.country("81.2.69.160".parse()?)
                .expect("GB record")
                .code(),
            "GB"
        );
        Ok(())
    }

    #[test]
    fn invalid_database_is_a_startup_error() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("bad.mmdb");
        fs::write(&path, b"bad")?;
        assert!(GeoIp::open(path, Duration::ZERO, true).is_err());
        Ok(())
    }

    #[test]
    fn lazy_reload_after_interval_keeps_old_database_on_failure(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("country.mmdb");
        fs::copy(fixture(), &path)?;
        let geo = GeoIp::open(path.clone(), Duration::from_millis(10), true)?;
        fs::write(&path, b"corrupt replacement")?;
        std::thread::sleep(Duration::from_millis(20));
        // The lookup itself triggers the lazy reload; the prior reader survives.
        assert_eq!(
            geo.country("81.2.69.160".parse()?)
                .expect("GB record after failed reload")
                .code(),
            "GB"
        );
        assert!(geo.last_error().is_some());
        Ok(())
    }
}
