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
//! RFC 8737 TLS-ALPN-01 challenge certificate.
//!
//! The CA validates the identifier by connecting with ALPN `acme-tls/1` and
//! checking a self-signed certificate whose critical `acmeIdentifier`
//! extension contains SHA-256(keyAuthorization). The pair is ephemeral and is
//! never written to disk.

use rcgen::{CertificateParams, CustomExtension, DnType, KeyPair};
use sha2::{Digest, Sha256};

/// Build the self-signed challenge certificate and its PKCS#8 private key.
///
/// # Errors
/// A malformed DNS identifier or a key/certificate generation failure.
pub fn challenge_certificate(
    domain: &str,
    key_authorization: &str,
) -> anyhow::Result<(String, String)> {
    if !valid_dns_identifier(domain) {
        anyhow::bail!("tls-alpn-01: invalid DNS identifier {domain:?}");
    }
    let mut params = CertificateParams::new(vec![domain.to_string()])?;
    params
        .distinguished_name
        .push(DnType::CommonName, domain.to_string());
    let digest = Sha256::digest(key_authorization.as_bytes());
    params
        .custom_extensions
        .push(CustomExtension::new_acme_identifier(&digest));
    let key = KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    Ok((cert.pem(), key.serialize_pem()))
}

fn valid_dns_identifier(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && !domain.ends_with('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls_pki_types::pem::PemObject;
    use rustls_pki_types::CertificateDer;
    use x509_parser::prelude::*;

    #[test]
    fn builds_critical_acme_identifier_and_key() {
        let (cert_pem, key_pem) =
            challenge_certificate("exyonq.test", "token.thumbprint").expect("challenge pair");
        assert!(
            cert_pem.contains("BEGIN CERTIFICATE"),
            "challenge certificate must be PEM"
        );
        assert!(
            key_pem.contains("BEGIN PRIVATE KEY"),
            "challenge key must be PKCS#8 PEM"
        );

        let der = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
            .next()
            .expect("one certificate")
            .expect("valid PEM");
        let (_, cert) = parse_x509_certificate(der.as_ref()).expect("parse certificate");
        assert!(
            cert.subject_alternative_name()
                .expect("SAN extension")
                .is_some(),
            "challenge certificate must carry the DNS SAN"
        );
        let acme = cert
            .extensions()
            .iter()
            .find(|ext| ext.oid.to_id_string() == "1.3.6.1.5.5.7.1.31")
            .expect("acmeIdentifier extension");
        assert!(acme.critical, "acmeIdentifier must be critical");
        let digest = Sha256::digest(b"token.thumbprint");
        // x509-parser exposes the DER-encoded OCTET STRING: 0x04, length 32,
        // then the SHA-256 of the key authorization.
        assert_eq!(
            &acme.value[..2],
            &[0x04, 0x20],
            "acmeIdentifier must wrap a 32-byte OCTET STRING"
        );
        assert_eq!(
            &acme.value[2..],
            digest.as_slice(),
            "acmeIdentifier digest must match the key authorization"
        );
    }

    #[test]
    fn rejects_an_invalid_identifier() {
        assert!(challenge_certificate("not a dns name", "auth").is_err());
    }
}
