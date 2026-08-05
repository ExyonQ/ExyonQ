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
//! KD4.11 — ACME integration and certificate publication contracts.

use async_trait::async_trait;
use exyonq_config_ir::AppConfig;
use std::cell::RefCell;
use std::sync::{Arc, Mutex, MutexGuard};

/// HTTP-01 challenge path prefix (RFC 8555).
pub const ACME_HTTP01_WELL_KNOWN_PREFIX: &str = "/.well-known/acme-challenge/";

/// Neutral TLS reload hook after certificate material is published on disk.
pub trait CertificatePublicationPort: Send + Sync {
    fn reload_tls_from_config(&self, config: &AppConfig) -> Result<(), String>;
}

/// ACME challenge lookup + bootstrap (implementation in `exyonq-acme`).
#[async_trait]
pub trait AcmeIntegrationService: Send + Sync {
    /// Return key authorization body when `path` is a registered HTTP-01 challenge.
    fn lookup_http01_key_authorization(&self, path: &str) -> Option<String>;

    /// Bootstrap certificates and start renewal loop when ACME is enabled in config.
    async fn bootstrap_if_enabled(
        &self,
        config: &AppConfig,
        publication: Arc<dyn CertificatePublicationPort>,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcmeIntegrationRegisterError {
    AlreadyRegistered,
    Poisoned,
}

enum TestOverride<S: ?Sized> {
    Inherit,
    ForceAbsent,
    Override(Arc<S>),
}

impl<S: ?Sized> Clone for TestOverride<S> {
    fn clone(&self) -> Self {
        match self {
            Self::Inherit => Self::Inherit,
            Self::ForceAbsent => Self::ForceAbsent,
            Self::Override(svc) => Self::Override(Arc::clone(svc)),
        }
    }
}

thread_local! {
    static ACME_INTEGRATION_TLS: RefCell<TestOverride<dyn AcmeIntegrationService>> =
        const { RefCell::new(TestOverride::Inherit) };
}

static ACME_INTEGRATION_SLOT: Mutex<Option<Arc<dyn AcmeIntegrationService>>> = Mutex::new(None);

pub fn register_acme_integration_service(
    service: Arc<dyn AcmeIntegrationService>,
) -> Result<(), AcmeIntegrationRegisterError> {
    let mut slot = ACME_INTEGRATION_SLOT
        .lock()
        .map_err(|_| AcmeIntegrationRegisterError::Poisoned)?;
    if slot.is_some() {
        return Err(AcmeIntegrationRegisterError::AlreadyRegistered);
    }
    *slot = Some(service);
    Ok(())
}

pub fn acme_integration_service() -> Option<Arc<dyn AcmeIntegrationService>> {
    let override_state = ACME_INTEGRATION_TLS.with(|c| c.borrow().clone());
    match override_state {
        TestOverride::Inherit => ACME_INTEGRATION_SLOT.lock().ok()?.clone(),
        TestOverride::ForceAbsent => None,
        TestOverride::Override(service) => Some(service),
    }
}

static REGISTRATION_GATE: Mutex<()> = Mutex::new(());

#[doc(hidden)]
pub fn acme_integration_registration_test_gate() -> MutexGuard<'static, ()> {
    REGISTRATION_GATE
        .lock()
        .expect("acme integration registration test gate poisoned")
}

pub struct AcmeIntegrationTestGuard {
    previous: TestOverride<dyn AcmeIntegrationService>,
}

impl AcmeIntegrationTestGuard {
    pub fn install(service: Arc<dyn AcmeIntegrationService>) -> Self {
        let previous = ACME_INTEGRATION_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::Override(service);
            prev
        });
        Self { previous }
    }

    pub fn force_absent() -> Self {
        let previous = ACME_INTEGRATION_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::ForceAbsent;
            prev
        });
        Self { previous }
    }
}

impl Drop for AcmeIntegrationTestGuard {
    fn drop(&mut self) {
        ACME_INTEGRATION_TLS.with(|cell| *cell.borrow_mut() = self.previous.clone());
    }
}

#[doc(hidden)]
pub fn clear_acme_integration_for_tests() {
    if let Ok(mut slot) = ACME_INTEGRATION_SLOT.lock() {
        *slot = None;
    }
}
