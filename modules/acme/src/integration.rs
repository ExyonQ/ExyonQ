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
//! ACME integration runtime — HTTP-01 store, bootstrap, renewal.

use crate::{ensure_certificates, spawn_renewal_loop, AcmeRenewalOutcome, ChallengeStore};
use async_trait::async_trait;
use exyonq_config_ir::AppConfig;
use exyonq_module_api::acme_integration::{
    register_acme_integration_service, AcmeIntegrationRegisterError, AcmeIntegrationService,
    CertificatePublicationPort, ACME_HTTP01_WELL_KNOWN_PREFIX,
};
use std::sync::Arc;
use tracing::{info, warn};

pub struct AcmeIntegrationRuntime {
    challenges: ChallengeStore,
}

impl AcmeIntegrationRuntime {
    pub fn new() -> Self {
        Self {
            challenges: ChallengeStore::new(),
        }
    }

    pub fn challenge_store(&self) -> &ChallengeStore {
        &self.challenges
    }

    fn lookup_token(path: &str) -> Option<&str> {
        let token = path.strip_prefix(ACME_HTTP01_WELL_KNOWN_PREFIX)?;
        if token.is_empty() || token.contains('/') {
            return None;
        }
        Some(token)
    }
}

impl Default for AcmeIntegrationRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AcmeIntegrationService for AcmeIntegrationRuntime {
    fn lookup_http01_key_authorization(&self, path: &str) -> Option<String> {
        let token = Self::lookup_token(path)?;
        self.challenges.get(token)
    }

    async fn bootstrap_if_enabled(
        &self,
        config: &AppConfig,
        publication: Arc<dyn CertificatePublicationPort>,
    ) {
        // One order per `[[server]]` block: each block owns its cert/key files,
        // so a multi-domain listener issues and renews every certificate.
        let jobs = crate::manager::acme_jobs(config);
        if jobs.is_empty() {
            return;
        }

        let mut issued = false;
        for (idx, tls, acme_cfg) in &jobs {
            match ensure_certificates(tls, acme_cfg, &self.challenges, publication.as_ref(), false)
                .await
            {
                Ok(AcmeRenewalOutcome::Issued(paths)) => {
                    issued = true;
                    info!(
                        server = idx,
                        cert = %paths.cert.display(),
                        domains = ?acme_cfg.domains,
                        "ACME bootstrap issued"
                    );
                }
                Ok(AcmeRenewalOutcome::SkippedExisting) => {}
                Err(err) => warn!(server = idx, %err, "ACME bootstrap failed"),
            }
        }
        if issued {
            match publication.reload_tls_from_config(config) {
                Ok(()) => info!("TLS acceptor reloaded after ACME bootstrap"),
                Err(err) => warn!(%err, "TLS reload after ACME bootstrap failed"),
            }
        }

        let config = Arc::new(config.clone());
        let jobs = jobs.into_iter().map(|(_, tls, acme)| (tls, acme)).collect();
        let renewal_publication = Arc::clone(&publication);
        spawn_renewal_loop(
            jobs,
            self.challenges.clone(),
            renewal_publication,
            move || {
                if publication.reload_tls_from_config(&config).is_ok() {
                    info!("TLS acceptor reloaded after ACME renewal");
                }
            },
        )
        .await;
    }
}

pub fn register_acme_integration() -> Result<(), AcmeIntegrationRegisterError> {
    register_acme_integration_service(Arc::new(AcmeIntegrationRuntime::new()))
}

/// Build plaintext HTTP-01 response body lookup (tests and external callers).
pub fn http01_key_authorization(runtime: &AcmeIntegrationRuntime, path: &str) -> Option<String> {
    runtime.lookup_http01_key_authorization(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_module_api::{
        acme_integration_service, clear_acme_integration_for_tests, AcmeIntegrationTestGuard,
        ACME_HTTP01_WELL_KNOWN_PREFIX,
    };

    #[test]
    fn lookup_http01_strips_prefix_and_rejects_nested_paths() {
        let runtime = AcmeIntegrationRuntime::new();
        runtime.challenge_store().set("tok", "key-auth-body");
        let path = format!("{ACME_HTTP01_WELL_KNOWN_PREFIX}tok");
        assert_eq!(
            runtime.lookup_http01_key_authorization(&path).as_deref(),
            Some("key-auth-body")
        );
        assert!(runtime
            .lookup_http01_key_authorization(&format!("{ACME_HTTP01_WELL_KNOWN_PREFIX}bad/extra"))
            .is_none());
    }

    #[test]
    fn register_once_and_test_guard_override() {
        let _gate = exyonq_module_api::acme_integration_registration_test_gate();
        clear_acme_integration_for_tests();
        register_acme_integration().expect("register");
        assert!(acme_integration_service().is_some());
        let custom = Arc::new(AcmeIntegrationRuntime::new());
        custom.challenge_store().set("x", "y");
        let _guard = AcmeIntegrationTestGuard::install(custom);
        let path = format!("{ACME_HTTP01_WELL_KNOWN_PREFIX}x");
        assert_eq!(
            acme_integration_service()
                .unwrap()
                .lookup_http01_key_authorization(&path)
                .as_deref(),
            Some("y")
        );
    }
}
