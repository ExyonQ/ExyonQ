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
    Account, AccountCredentials, AuthorizationStatus, CertificateIdentifier, ChallengeType,
    Identifier, LetsEncrypt, NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::CertificateDer;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// Renew when the leaf certificate expires within this window (ARI absent).
const RENEW_MARGIN: Duration = Duration::from_secs(30 * 24 * 3600);

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

/// Account credentials file: `acme.account_path` or `<cert>.acme-account.json`.
fn account_path(tls: &TlsConfig, acme: &AcmeConfig) -> PathBuf {
    acme.account_path
        .clone()
        .unwrap_or_else(|| tls.cert.with_extension("acme-account.json"))
}

/// Marker recording which ACME directory issued `<cert>`. `replaces` (RFC
/// 9773 ARI) may only be sent when the certificate on disk came from the same
/// directory: a placeholder or foreign certificate has an Authority Key
/// Identifier the CA cannot resolve, and the order is rejected as malformed.
fn origin_marker_path(cert: &Path) -> PathBuf {
    let mut name = cert.as_os_str().to_os_string();
    name.push(".acme-origin");
    PathBuf::from(name)
}

fn record_origin(cert: &Path, directory: &str) -> anyhow::Result<()> {
    write_private(&origin_marker_path(cert), directory.as_bytes())
}

fn origin_matches(cert: &Path, directory: &str) -> bool {
    std::fs::read_to_string(origin_marker_path(cert))
        .map(|raw| raw.trim() == directory)
        .unwrap_or(false)
}

/// Write `contents` with owner-only permissions, atomically (tmp + rename).
/// The ACME account JSON and the certificate key are private material.
fn write_private(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
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

/// Restore the persisted account, or create a new one and persist it.
/// ACME accounts survive across runs: required for ARI `replaces` and for
/// CA-side revocation tracking. Returns the account plus whether it was
/// restored (a fresh account must not claim `replaces` for an old cert).
async fn load_or_create_account(
    tls: &TlsConfig,
    acme: &AcmeConfig,
) -> anyhow::Result<(Account, bool)> {
    let path = account_path(tls, acme);
    if let Ok(raw) = std::fs::read_to_string(&path) {
        let credentials: AccountCredentials = serde_json::from_str(&raw)
            .map_err(|err| anyhow::anyhow!("acme account file {}: {err}", path.display()))?;
        let account = Account::builder()?
            .from_credentials(credentials)
            .await
            .map_err(anyhow::Error::from)?;
        return Ok((account, true));
    }
    let contact = format!("mailto:{}", acme.email);
    let (account, credentials) = Account::builder()?
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
    write_private(&path, serde_json::to_string(&credentials)?.as_bytes())?;
    Ok((account, false))
}

/// Leaf certificate DER + notAfter from `cert_path`, when it parses.
fn leaf_cert(path: &Path) -> Option<(CertificateDer<'static>, i64)> {
    let der = CertificateDer::pem_file_iter(path).ok()?.next()?.ok()?;
    let not_after = {
        let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
        cert.validity().not_after.timestamp()
    };
    Some((der, not_after))
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Should the existing certificate be re-issued? ARI when the directory
/// supports it, falling back to a local expiry margin. Unparseable or
/// already-expired certificates always renew.
async fn should_renew(account: &Account, cert_path: &Path) -> bool {
    let Some((der, not_after)) = leaf_cert(cert_path) else {
        return true;
    };
    if let Ok(identifier) = CertificateIdentifier::try_from(&der) {
        match account.renewal_info(&identifier).await {
            Ok((info, _retry)) => {
                let start = info.suggested_window.start.unix_timestamp();
                info!(renewal_window_start = start, "ACME ARI renewal window");
                return start <= now_epoch();
            }
            Err(err) => {
                info!(%err, "ACME ARI unavailable — falling back to expiry margin");
            }
        }
    }
    renewal_needed(not_after, now_epoch())
}

/// Local fallback: renew inside `RENEW_MARGIN` (or when already expired).
fn renewal_needed(not_after: i64, now: i64) -> bool {
    (not_after - now) < RENEW_MARGIN.as_secs() as i64
}

/// Order identifiers, verbatim. Wildcards must reach the CA as
/// `*.example.com`: that is what makes the authorization — and the issued
/// certificate — wildcard-scoped. Stripping the prefix silently orders the
/// apex instead.
fn order_identifiers(acme: &AcmeConfig) -> Vec<Identifier> {
    acme.domains.iter().cloned().map(Identifier::Dns).collect()
}

/// Run `acme.dns_hook` to publish or remove the DNS-01 TXT record.
async fn run_dns_hook(
    hook: &str,
    action: &str,
    domain: &str,
    token: &str,
    txt: &str,
) -> anyhow::Result<()> {
    let status = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(hook)
        .env("EXYONQ_ACME_ACTION", action)
        .env("EXYONQ_ACME_DOMAIN", domain)
        .env("EXYONQ_ACME_TOKEN", token)
        .env("EXYONQ_ACME_TXT_VALUE", txt)
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("acme.dns_hook {action} for {domain} exited {status}");
    }
    Ok(())
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
    std::fs::write(cert_path, cert_pem.as_bytes())?;
    write_private(key_path, key_pem.as_bytes())?;
    Ok(())
}

/// Obtain or renew certificates for `tls`. Challenge per `acme.challenge`
/// (HTTP-01 via `challenges`, or DNS-01 via `acme.dns_hook`). An existing
/// cert renews when ARI says the window opened, or inside the expiry margin.
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
    let (account, account_reused) = load_or_create_account(tls, acme).await?;
    let renewing = cert_path.exists() && key_path.exists();
    if renewing && !force && !should_renew(&account, &cert_path).await {
        return Ok(AcmeRenewalOutcome::SkippedExisting);
    }

    let identifiers = order_identifiers(acme);

    // ARI: link the renewal order to the certificate it replaces (RFC 9773).
    // Only a *reused* account may claim it; a fresh account has no history.
    let directory = acme_directory(acme.staging);
    let mut new_order = NewOrder::new(&identifiers);
    if renewing && account_reused && origin_matches(&cert_path, &directory) {
        if let Some((der, _)) = leaf_cert(&cert_path) {
            if let Ok(identifier) = CertificateIdentifier::try_from(&der) {
                new_order = new_order.replaces(identifier.into_owned());
            }
        }
    }
    let mut order = account.new_order(&new_order).await?;

    let dns01 = acme.challenge_type() == "dns-01";
    let mut dns_cleanups: Vec<(String, String, String)> = Vec::new();
    // HTTP-01 tokens must stay served until the CA has fetched them, i.e.
    // until `poll_ready` finishes — not just until `set_ready` returns.
    let mut http01_tokens: Vec<String> = Vec::new();
    let challenge_result: anyhow::Result<()> = async {
        let mut authorizations = order.authorizations();
        while let Some(result) = authorizations.next().await {
            let mut authz = result?;
            if authz.status == AuthorizationStatus::Valid {
                continue;
            }

            let domain = authz.identifier().to_string();
            if dns01 {
                let mut challenge = authz
                    .challenge(ChallengeType::Dns01)
                    .ok_or_else(|| anyhow::anyhow!("no DNS-01 challenge for {domain}"))?;
                let key_auth = challenge.key_authorization();
                let txt = key_auth.dns_value();
                let token = challenge.token.clone();
                let hook = acme.dns_hook.as_deref().expect("dns-01 validated");
                let record_domain = domain.trim_start_matches("*.");
                run_dns_hook(hook, "present", record_domain, &token, &txt).await?;
                dns_cleanups.push((record_domain.to_string(), token.clone(), txt));
                info!(%token, %domain, "ACME DNS-01 challenge published");
                tokio::time::sleep(Duration::from_secs(acme.dns_propagation_secs.unwrap_or(15)))
                    .await;
                challenge.set_ready().await?;
            } else {
                let mut challenge = authz
                    .challenge(ChallengeType::Http01)
                    .ok_or_else(|| anyhow::anyhow!("no HTTP-01 challenge for {domain}"))?;
                let key_auth = challenge.key_authorization().as_str().to_string();
                let token = challenge.token.clone();
                challenges.set(token.clone(), key_auth);
                info!(%token, %domain, "ACME HTTP-01 challenge registered");
                challenge.set_ready().await?;
                http01_tokens.push(token);
            }
        }
        Ok(())
    }
    .await;

    // Best-effort TXT cleanup on every outcome (success and failure).
    if let Some(hook) = acme.dns_hook.as_deref() {
        for (domain, token, txt) in dns_cleanups {
            if let Err(err) = run_dns_hook(hook, "cleanup", &domain, &token, &txt).await {
                warn!(%err, %domain, "ACME DNS-01 cleanup failed");
            }
        }
    }
    challenge_result?;

    let ready = order.poll_ready(&RetryPolicy::default()).await;
    for token in &http01_tokens {
        challenges.remove(token);
    }
    if ready? != OrderStatus::Ready {
        anyhow::bail!("ACME order not ready");
    }

    let key_pem = order.finalize().await?;
    let cert_pem = order.poll_certificate(&RetryPolicy::default()).await?;

    write_pem_pair(&cert_path, &key_path, &cert_pem, &key_pem)?;
    record_origin(&cert_path, &acme_directory(acme.staging))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Q-09/F-01: wildcards reach the CA verbatim; stripping `*.` would order
    /// (and issue) the apex instead.
    #[test]
    fn order_identifiers_keep_wildcard_prefix() {
        let acme = AcmeConfig {
            enabled: true,
            email: "ops@a.test".into(),
            domains: vec!["a.test".into(), "*.a.test".into()],
            staging: false,
            challenge: Some("dns-01".into()),
            dns_hook: Some("/bin/true".into()),
            dns_propagation_secs: None,
            account_path: None,
        };
        assert_eq!(
            order_identifiers(&acme),
            vec![
                Identifier::Dns("a.test".into()),
                Identifier::Dns("*.a.test".into()),
            ]
        );
    }

    /// A certificate that was not issued by this ACME directory (a
    /// placeholder, or one from another CA/staging run) must not claim
    /// `replaces`: the CA rejects the order with an Authority Key Identifier
    /// mismatch.
    #[test]
    fn origin_marker_gates_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("tls-lab.pem");
        std::fs::write(&cert, b"placeholder").unwrap();
        let production = acme_directory(false);
        let staging = acme_directory(true);

        assert!(!origin_matches(&cert, &production), "no marker yet");
        record_origin(&cert, &production).unwrap();
        assert!(origin_matches(&cert, &production));
        assert!(
            !origin_matches(&cert, &staging),
            "staging must not inherit a production marker"
        );

        record_origin(&cert, &staging).unwrap();
        assert!(origin_matches(&cert, &staging));
        assert!(!origin_matches(&cert, &production));
    }

    #[test]
    fn private_writes_are_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("acct.json");
        write_private(&path, b"{}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "private material must be 0600");
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        assert!(
            !path.with_extension("tmp").exists(),
            "tmp file renamed away"
        );
    }

    #[test]
    fn renewal_margin_is_thirty_days() {
        let now = 1_000_000_000;
        assert!(renewal_needed(now + 29 * 86_400, now));
        assert!(!renewal_needed(now + 31 * 86_400, now));
        assert!(renewal_needed(now - 1, now), "expired cert always renews");
    }

    #[tokio::test]
    async fn dns_hook_receives_documented_environment() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("env.txt");
        let hook = format!(
            "printf '%s|%s|%s|%s' \"$EXYONQ_ACME_ACTION\" \"$EXYONQ_ACME_DOMAIN\" \
             \"$EXYONQ_ACME_TOKEN\" \"$EXYONQ_ACME_TXT_VALUE\" > {}",
            out.display()
        );
        run_dns_hook(&hook, "present", "example.test", "tok", "txt-value")
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "present|example.test|tok|txt-value"
        );

        let failing = "exit 3";
        assert!(
            run_dns_hook(failing, "cleanup", "example.test", "tok", "txt")
                .await
                .is_err()
        );
    }
}
