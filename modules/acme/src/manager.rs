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
//! ACME order flow: HTTP-01 challenge registration and certificate persistence.

use crate::ChallengeStore;
use exyonq_config_ir::{AcmeConfig, TlsConfig};
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder,
    OrderStatus, RetryPolicy,
};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Resolved PEM paths after a successful ACME run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcmeCertificatePaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcmeRenewalOutcome {
    /// Certificate already present and ACME not forced.
    SkippedExisting,
    /// New or renewed certificate written to disk.
    Issued(AcmeCertificatePaths),
}

fn acme_directory(staging: bool) -> String {
    if staging {
        LetsEncrypt::Staging.url().to_string()
    } else {
        LetsEncrypt::Production.url().to_string()
    }
}

fn write_pem_pair(
    cert_path: &Path,
    key_path: &Path,
    cert_pem: &str,
    key_pem: &str,
) -> anyhow::Result<()> {
    if let Some(parent) = cert_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(cert_path, cert_pem.as_bytes())?;
    std::fs::write(key_path, key_pem.as_bytes())?;
    Ok(())
}

/// Obtain or renew certificates for `tls` using HTTP-01 and `challenges` for validation.
pub async fn ensure_certificates(
    tls: &TlsConfig,
    acme: &AcmeConfig,
    challenges: &ChallengeStore,
    force: bool,
) -> anyhow::Result<AcmeRenewalOutcome> {
    if !acme.enabled {
        anyhow::bail!("acme.enabled is false");
    }
    if acme.domains.is_empty() {
        anyhow::bail!("acme.domains must not be empty");
    }

    let cert_path = tls.cert.clone();
    let key_path = tls.key.clone();
    if !force && cert_path.exists() && key_path.exists() {
        return Ok(AcmeRenewalOutcome::SkippedExisting);
    }

    let contact = format!("mailto:{}", acme.email);
    let (account, _credentials) = Account::builder()?
        .create(
            &NewAccount {
                contact: &[&contact],
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            acme_directory(acme.staging),
            None,
        )
        .await?;

    let identifiers: Vec<Identifier> = acme
        .domains
        .iter()
        .map(|domain| Identifier::Dns(domain.clone()))
        .collect();

    let mut order = account.new_order(&NewOrder::new(&identifiers)).await?;

    let mut authorizations = order.authorizations();
    while let Some(result) = authorizations.next().await {
        let mut authz = result?;
        if authz.status == AuthorizationStatus::Valid {
            continue;
        }

        let domain = format!("{:?}", authz.identifier());
        let mut challenge = authz
            .challenge(ChallengeType::Http01)
            .ok_or_else(|| anyhow::anyhow!("no HTTP-01 challenge for {domain}"))?;

        let key_auth = challenge.key_authorization().as_str().to_string();
        let token = challenge.token.clone();
        challenges.set(token.clone(), key_auth);
        info!(%token, %domain, "ACME HTTP-01 challenge registered");
        challenge.set_ready().await?;
        challenges.remove(&token);
    }

    let status = order.poll_ready(&RetryPolicy::default()).await?;
    if status != OrderStatus::Ready {
        anyhow::bail!("ACME order not ready: {status:?}");
    }

    let key_pem = order.finalize().await?;
    let cert_pem = order.poll_certificate(&RetryPolicy::default()).await?;

    write_pem_pair(&cert_path, &key_path, &cert_pem, &key_pem)?;
    info!(
        cert = %cert_path.display(),
        key = %key_path.display(),
        "ACME certificate issued"
    );

    Ok(AcmeRenewalOutcome::Issued(AcmeCertificatePaths {
        cert: cert_path,
        key: key_path,
    }))
}

/// Background renewal loop (daily check; re-issue when cert missing or near expiry).
pub async fn spawn_renewal_loop(
    tls: TlsConfig,
    acme: AcmeConfig,
    challenges: ChallengeStore,
    on_renewed: impl Fn() + Send + Sync + 'static,
) {
    if !acme.enabled {
        return;
    }
    let challenges = std::sync::Arc::new(challenges);
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(86_400);
        loop {
            match ensure_certificates(&tls, &acme, challenges.as_ref(), false).await {
                Ok(AcmeRenewalOutcome::Issued(_)) => on_renewed(),
                Ok(AcmeRenewalOutcome::SkippedExisting) => {}
                Err(err) => warn!(%err, "ACME renewal failed"),
            }
            tokio::time::sleep(interval).await;
        }
    });
}
