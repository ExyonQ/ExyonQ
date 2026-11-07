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
//! 0.5 — nginx `auth_basic` with htpasswd (bcrypt, {SHA}, {PLAIN}, plain).

use base64::Engine as _;
use exyonq_config_ir::AuthConfig;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;
use subtle::ConstantTimeEq;

const MAX_HTPASSWD_BYTES: u64 = 1024 * 1024;
const MAX_HTPASSWD_LINE_BYTES: usize = 4096;
const MAX_HTPASSWD_ENTRIES: usize = 4096;
const MAX_CACHED_HTPASSWD_FILES: usize = 64;

#[derive(Debug, Clone)]
struct HtpasswdEntry {
    user: String,
    hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthOutcome {
    Allowed { user: String },
    Challenge { realm: String },
}

#[derive(Debug, Clone)]
struct CachedHtpasswd {
    modified: SystemTime,
    len: u64,
    entries: Arc<[HtpasswdEntry]>,
}

#[derive(Debug, Default)]
struct HtpasswdCache {
    files: HashMap<PathBuf, CachedHtpasswd>,
}

impl HtpasswdCache {
    fn entries(&mut self, path: &Path) -> Option<Arc<[HtpasswdEntry]>> {
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_HTPASSWD_BYTES {
            return None;
        }
        let modified = metadata.modified().ok()?;
        let len = metadata.len();
        if let Some(cached) = self.files.get(path) {
            if cached.modified == modified && cached.len == len {
                return Some(Arc::clone(&cached.entries));
            }
        }

        let raw = std::fs::read_to_string(path).ok()?;
        if raw.len() as u64 > MAX_HTPASSWD_BYTES {
            return None;
        }
        let entries = parse_htpasswd(&raw)?;
        if self.files.len() >= MAX_CACHED_HTPASSWD_FILES && !self.files.contains_key(path) {
            if let Some(victim) = self.files.keys().next().cloned() {
                self.files.remove(&victim);
            }
        }
        self.files.insert(
            path.to_path_buf(),
            CachedHtpasswd {
                modified,
                len,
                entries: Arc::clone(&entries),
            },
        );
        Some(entries)
    }
}

fn parse_htpasswd(raw: &str) -> Option<Arc<[HtpasswdEntry]>> {
    let mut entries = Vec::new();
    for line in raw.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.len() > MAX_HTPASSWD_LINE_BYTES {
            return None;
        }
        let Some((user, hash)) = line.split_once(':') else {
            continue;
        };
        if user.is_empty() {
            continue;
        }
        if entries.len() >= MAX_HTPASSWD_ENTRIES {
            return None;
        }
        entries.push(HtpasswdEntry {
            user: user.to_string(),
            hash: hash.to_string(),
        });
    }
    Some(Arc::from(entries.into_boxed_slice()))
}

static HTPASSWD_CACHE: OnceLock<Mutex<HtpasswdCache>> = OnceLock::new();

fn lock_cache() -> std::sync::MutexGuard<'static, HtpasswdCache> {
    HTPASSWD_CACHE
        .get_or_init(|| Mutex::new(HtpasswdCache::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

fn verify(hash: &str, password: &str) -> bool {
    if hash.starts_with("$2a$") || hash.starts_with("$2b$") || hash.starts_with("$2y$") {
        return bcrypt::verify(password, hash).unwrap_or(false);
    }
    if let Some(rest) = hash.strip_prefix("{SHA}") {
        let digest = Sha1::digest(password.as_bytes());
        let encoded = base64::engine::general_purpose::STANDARD.encode(digest);
        return ct_eq(&encoded, rest);
    }
    if let Some(rest) = hash.strip_prefix("{PLAIN}") {
        return ct_eq(rest, password);
    }
    // apr1/other crypt formats are not supported: fail closed.
    if hash.starts_with('$') {
        return false;
    }
    ct_eq(hash, password)
}

/// Returns the authenticated username or the realm to challenge.
pub(crate) fn check(
    config: &AuthConfig,
    route_name: &str,
    headers: &[(String, String)],
) -> Option<AuthOutcome> {
    let route = config.routes.get(route_name)?;
    check_user_file(&route.realm, &route.user_file, headers)
}

/// Verify Basic credentials against a bounded, mtime-checked htpasswd cache.
pub(crate) fn check_user_file(
    realm: &str,
    user_file: &str,
    headers: &[(String, String)],
) -> Option<AuthOutcome> {
    let challenge = || AuthOutcome::Challenge {
        realm: realm.to_string(),
    };
    let Some(value) = header_value(headers, "authorization") else {
        return Some(challenge());
    };
    let Some((scheme, encoded)) = value.split_once(' ') else {
        return Some(challenge());
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return Some(challenge());
    };
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let Some((user, password)) = decoded.as_deref().and_then(|v| v.split_once(':')) else {
        return Some(challenge());
    };
    let entries = {
        let mut cache = lock_cache();
        cache.entries(Path::new(user_file))
    };
    let Some(entries) = entries else {
        return Some(challenge());
    };
    for entry in entries.iter() {
        if ct_eq(&entry.user, user) && verify(&entry.hash, password) {
            return Some(AuthOutcome::Allowed {
                user: user.to_string(),
            });
        }
    }
    Some(challenge())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_config_ir::{AuthConfig, AuthRouteConfig};
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::time::Duration;
    use tempfile::NamedTempFile;

    fn auth_config(user_file: &str) -> AuthConfig {
        let mut routes = BTreeMap::new();
        routes.insert(
            "site".to_string(),
            AuthRouteConfig {
                realm: "restricted".into(),
                user_file: user_file.into(),
                auth_request: None,
            },
        );
        AuthConfig { routes }
    }

    fn basic(user: &str, password: &str) -> String {
        let raw = format!("{user}:{password}");
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        )
    }

    #[test]
    fn verify_supports_bcrypt_sha_plain_and_fails_closed() {
        let bcrypt_hash = bcrypt::hash("pw", 4).expect("bcrypt hash");
        assert!(verify(&bcrypt_hash, "pw"));
        assert!(!verify(&bcrypt_hash, "bad"));

        let sha = format!(
            "{{SHA}}{}",
            base64::engine::general_purpose::STANDARD.encode(Sha1::digest(b"pw"))
        );
        assert!(verify(&sha, "pw"));
        assert!(!verify(&sha, "bad"));

        assert!(verify("{PLAIN}pw", "pw"));
        assert!(!verify("{PLAIN}pw", "bad"));
        assert!(!verify("$apr1$unsupported", "pw"));
    }

    #[test]
    fn check_challenges_and_accepts_plain_credentials() {
        let mut file = NamedTempFile::new().expect("temp htpasswd");
        writeln!(file, "u:{{PLAIN}}pw").expect("write htpasswd");
        let config = auth_config(file.path().to_str().expect("utf-8 path"));

        assert_eq!(
            check(&config, "site", &[]),
            Some(AuthOutcome::Challenge {
                realm: "restricted".to_string()
            }),
            "missing Authorization must challenge"
        );

        let good = vec![("authorization".to_string(), basic("u", "pw"))];
        assert_eq!(
            check(&config, "site", &good),
            Some(AuthOutcome::Allowed {
                user: "u".to_string()
            })
        );

        let bad = vec![("authorization".to_string(), basic("u", "wrong"))];
        assert_eq!(
            check(&config, "site", &bad),
            Some(AuthOutcome::Challenge {
                realm: "restricted".to_string()
            }),
            "wrong password must challenge"
        );
    }

    #[test]
    fn check_fails_closed_when_htpasswd_is_missing() {
        let config = auth_config("/definitely/not/a/htpasswd");
        let good = vec![("authorization".to_string(), basic("u", "pw"))];
        assert_eq!(
            check(&config, "site", &good),
            Some(AuthOutcome::Challenge {
                realm: "restricted".to_string()
            }),
            "unreadable user_file must not authenticate"
        );
    }

    #[test]
    fn cache_reuses_entries_and_reloads_after_same_size_rewrite() {
        let mut file = NamedTempFile::new().expect("temp htpasswd");
        writeln!(file, "u:{{PLAIN}}pw").expect("write htpasswd");
        file.flush().expect("flush");
        let path = file.path().to_path_buf();
        let mut cache = HtpasswdCache::default();

        let first = cache.entries(&path).expect("first load");
        let second = cache.entries(&path).expect("cache hit");
        assert!(Arc::ptr_eq(&first, &second), "cache must reuse entries");

        std::fs::write(&path, "u:{PLAIN}zz\n").expect("same-size rewrite");
        let stamp = SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(stamp)
            .expect("set mtime");

        let third = cache.entries(&path).expect("reload");
        assert!(!Arc::ptr_eq(&first, &third), "mtime change must reload");
        assert!(third.iter().any(|entry| verify(&entry.hash, "zz")));
        assert!(!third.iter().any(|entry| verify(&entry.hash, "pw")));
    }

    #[test]
    fn cache_fails_closed_for_missing_file() {
        let mut cache = HtpasswdCache::default();
        assert!(cache
            .entries(Path::new("/definitely/not/a/htpasswd"))
            .is_none());
    }
}
