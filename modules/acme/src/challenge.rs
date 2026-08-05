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
//! In-memory HTTP-01 challenge token store.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Token → key authorization body served at `/.well-known/acme-challenge/{token}`.
#[derive(Debug, Default, Clone)]
pub struct ChallengeStore {
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl ChallengeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, token: impl Into<String>, key_authorization: impl Into<String>) {
        if let Ok(mut map) = self.inner.write() {
            map.insert(token.into(), key_authorization.into());
        }
    }

    pub fn get(&self, token: &str) -> Option<String> {
        self.inner.read().ok()?.get(token).cloned()
    }

    pub fn remove(&self, token: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(token);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut map) = self.inner.write() {
            map.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_retrieves_tokens() {
        let store = ChallengeStore::new();
        store.set("tok", "key-auth");
        assert_eq!(store.get("tok").as_deref(), Some("key-auth"));
        store.remove("tok");
        assert!(store.get("tok").is_none());
    }
}
