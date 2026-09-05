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

    /// TLS-001: El token ACME debe permanecer servible hasta que poll_ready termine.
    ///
    /// Bug: `ensure_certificates` hace `challenges.set` → `challenge.set_ready().await?`
    /// → **`challenges.remove(&token)`** → luego `order.poll_ready`.
    /// El token debe seguir servible hasta que poll_ready termine, pero se elimina antes.
    ///
    /// Este test documenta el contrato esperado: tras set_ready (simulado), el token
    /// DEBE seguir disponible vía get() hasta que el order esté Ready.
    #[test]
    #[ignore = "TLS-001 gap: needs instant-acme Order/Challenge mock; remove() before poll_ready at manager.rs ~L125-128"]
    fn challenge_token_must_remain_available_until_order_ready() {
        // Contrato: después de set_ready() y ANTES de poll_ready() terminar,
        // el token debe seguir servible. Hoy se elimina inmediatamente tras set_ready.
        //
        // Simular el flujo ACME requiere mockear instant_acme::Account, Order, Challenge,
        // lo cual es un mock enorme. Este test documenta el gap.
        let store = ChallengeStore::new();
        let token = "test-acme-token-abc123";
        let key_auth = "key-authorization-xyz";

        // Paso 1: set (correcto)
        store.set(token, key_auth);
        assert!(
            store.get(token).is_some(),
            "TLS-001: token debe existir tras set()"
        );

        // Paso 2: simular set_ready (en producción sería challenge.set_ready().await)
        // --- aquí el código de producción hace challenges.remove(&token) ANTES de poll_ready ---

        // Paso 3: contrato - token DEBE seguir disponible hasta poll_ready OK
        // Este assert documenta lo que DEBERÍA pasar (hoy falla porque remove es prematuro)
        assert!(
            store.get(token).is_some(),
            "TLS-001: token DEBE permanecer hasta poll_ready complete; \
             hoy se elimina prematuramente en manager.rs L126"
        );
    }
}
