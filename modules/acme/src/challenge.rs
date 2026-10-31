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
//! In-memory HTTP-01 challenge token store, optionally shared across
//! co-located ExyonQ processes through a directory on the filesystem.
//!
//! A separate plaintext listener (e.g. the :80 redirect instance) cannot
//! see this process's memory. When `EXYONQ_ACME_CHALLENGE_DIR` is set in
//! both units, `set`/`remove` also maintain `<dir>/<token>` files and `get`
//! falls back to reading them, so the sibling process can answer the CA
//! while this one runs the order.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::warn;

/// Shared challenge directory env var (systemd drop-in, e.g. under
/// `RuntimeDirectory`). Unset → memory-only store (single-process layout).
pub const CHALLENGE_DIR_ENV: &str = "EXYONQ_ACME_CHALLENGE_DIR";

/// Token → key authorization body served at `/.well-known/acme-challenge/{token}`.
#[derive(Debug, Default, Clone)]
pub struct ChallengeStore {
    inner: Arc<RwLock<HashMap<String, String>>>,
    dir: Option<PathBuf>,
}

/// ACME tokens are base64url (`[A-Za-z0-9_-]`); anything else never becomes a
/// filename, so `..` or separators cannot escape `dir`.
fn token_filename_ok(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 256
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Write `contents` atomically (tmp + rename) with owner-only permissions.
fn write_private(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(contents)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl ChallengeStore {
    pub fn new() -> Self {
        Self::with_dir(std::env::var_os(CHALLENGE_DIR_ENV).map(PathBuf::from))
    }

    /// Memory store plus optional on-disk sharing directory.
    pub fn with_dir(dir: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            dir,
        }
    }

    fn token_path(&self, token: &str) -> Option<PathBuf> {
        if !token_filename_ok(token) {
            return None;
        }
        self.dir.as_ref().map(|dir| dir.join(token))
    }

    pub fn set(&self, token: impl Into<String>, key_authorization: impl Into<String>) {
        let token = token.into();
        let key_authorization = key_authorization.into();
        if let Some(path) = self.token_path(&token) {
            if let Err(err) = write_private(&path, key_authorization.as_bytes()) {
                warn!(%err, path = %path.display(), "ACME HTTP-01 challenge file write failed");
            }
        }
        if let Ok(mut map) = self.inner.write() {
            map.insert(token, key_authorization);
        }
    }

    pub fn get(&self, token: &str) -> Option<String> {
        if let Ok(map) = self.inner.read() {
            if let Some(value) = map.get(token) {
                return Some(value.clone());
            }
        }
        let path = self.token_path(token)?;
        match std::fs::read_to_string(&path) {
            Ok(value) => Some(value),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                warn!(%err, path = %path.display(), "ACME HTTP-01 challenge file read failed");
                None
            }
        }
    }

    pub fn remove(&self, token: &str) {
        if let Some(path) = self.token_path(token) {
            if let Err(err) = std::fs::remove_file(&path) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    warn!(%err, path = %path.display(), "ACME HTTP-01 challenge file remove failed");
                }
            }
        }
        if let Ok(mut map) = self.inner.write() {
            map.remove(token);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut map) = self.inner.write() {
            for token in map.keys() {
                if let Some(path) = self.token_path(token) {
                    let _ = std::fs::remove_file(path);
                }
            }
            map.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_retrieves_tokens() {
        let store = ChallengeStore::default();
        store.set("tok", "key-auth");
        assert_eq!(store.get("tok").as_deref(), Some("key-auth"));
        store.remove("tok");
        assert!(store.get("tok").is_none());
    }

    #[test]
    fn shares_tokens_through_directory() {
        let dir = std::env::temp_dir().join(format!("exq-acme-{}", std::process::id()));
        let writer = ChallengeStore::with_dir(Some(dir.clone()));
        writer.set("tok-ABC_123", "key-auth-body");
        // A second store models the sibling process reading the same dir.
        let reader = ChallengeStore::with_dir(Some(dir.clone()));
        assert_eq!(reader.get("tok-ABC_123").as_deref(), Some("key-auth-body"));
        writer.remove("tok-ABC_123");
        assert_eq!(reader.get("tok-ABC_123"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsafe_token_names_never_touch_disk() {
        let dir = std::env::temp_dir().join(format!("exq-acme-bad-{}", std::process::id()));
        let store = ChallengeStore::with_dir(Some(dir.clone()));
        store.set("../escape", "x");
        store.set("a/b", "x");
        store.set("..", "x");
        assert!(std::fs::read_dir(&dir).map_or(true, |mut d| d.next().is_none()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
