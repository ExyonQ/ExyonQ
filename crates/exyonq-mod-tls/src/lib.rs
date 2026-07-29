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
//! TLS runtime — certificate load, rustls `ServerConfig`, hot-swappable acceptor (KD4.5).

use exyonq_module_api::tls_runtime::TlsListenerBinding;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::ServerSessionMemoryCache;
use rustls::ServerConfig;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio_rustls::TlsAcceptor;

/// Install rustls ring crypto provider (call once at process startup).
pub fn install_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Shared TLS session/ticket cache reused across reloads (P11 hardening).
#[derive(Debug, Clone)]
pub struct TlsSessionCache {
    inner: Arc<ServerSessionMemoryCache>,
}

impl Default for TlsSessionCache {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl TlsSessionCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: ServerSessionMemoryCache::new(capacity),
        }
    }

    pub fn as_arc(&self) -> Arc<ServerSessionMemoryCache> {
        Arc::clone(&self.inner)
    }
}

/// Hot-swappable TLS acceptor for certificate reload (ACME / manual PEM rotation).
#[derive(Clone, Default)]
pub struct SharedTlsAcceptor {
    inner: Arc<RwLock<Option<TlsAcceptor>>>,
}

impl SharedTlsAcceptor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(
        &self,
        settings: &TlsSettings,
        session_cache: &TlsSessionCache,
        alpn: &[&[u8]],
    ) -> io::Result<()> {
        let config = load_rustls_config(settings, alpn, Some(session_cache))?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        *self.inner.write().expect("tls acceptor lock") = Some(acceptor);
        Ok(())
    }

    pub fn load_binding(
        &self,
        binding: &TlsListenerBinding,
        session_cache: &TlsSessionCache,
        alpn: &[&[u8]],
    ) -> io::Result<()> {
        self.load(
            &TlsSettings {
                cert_path: PathBuf::from(&binding.cert_path),
                key_path: PathBuf::from(&binding.key_path),
            },
            session_cache,
            alpn,
        )
    }

    pub fn clear(&self) {
        *self.inner.write().expect("tls acceptor lock") = None;
    }

    pub fn snapshot(&self) -> Option<TlsAcceptor> {
        self.inner.read().expect("tls acceptor lock").clone()
    }
}

#[derive(Debug, Clone)]
pub struct TlsSettings {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

impl From<&TlsListenerBinding> for TlsSettings {
    fn from(binding: &TlsListenerBinding) -> Self {
        Self {
            cert_path: PathBuf::from(&binding.cert_path),
            key_path: PathBuf::from(&binding.key_path),
        }
    }
}

pub fn load_acceptor(settings: &TlsSettings) -> io::Result<TlsAcceptor> {
    let config = load_rustls_config(settings, &[b"h2", b"http/1.1"], None)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub fn load_rustls_config(
    settings: &TlsSettings,
    alpn: &[&[u8]],
    session_cache: Option<&TlsSessionCache>,
) -> io::Result<ServerConfig> {
    let certs = load_certs(&settings.cert_path)?;
    let key = load_private_key(&settings.key_path)?;
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    config.alpn_protocols = alpn.iter().map(|proto| proto.to_vec()).collect();
    if let Some(cache) = session_cache {
        config.session_storage = cache.as_arc();
    }
    Ok(config)
}

fn load_certs(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    CertificateDer::pem_file_iter(path)
        .map_err(pem_io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(pem_io_error)
}

fn load_private_key(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    // Contract: PKCS#8 only (same as prior rustls-pemfile::pkcs8_private_keys).
    // Do not widen to PKCS#1/SEC1 without an explicit product contract change.
    let key = PrivatePkcs8KeyDer::from_pem_file(path).map_err(pem_io_error)?;
    Ok(PrivateKeyDer::from(key))
}

fn pem_io_error(err: rustls::pki_types::pem::Error) -> io::Error {
    match err {
        rustls::pki_types::pem::Error::Io(io_err) => io_err,
        other => io::Error::new(io::ErrorKind::InvalidData, other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn write_ephemeral_pkcs8_pair(dir: &Path) -> (PathBuf, PathBuf) {
        install_rustls_provider();
        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");
        let status = Command::new("openssl")
            .args(["req", "-x509", "-newkey", "rsa:2048", "-keyout"])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .args([
                "-days",
                "1",
                "-nodes",
                "-subj",
                "/CN=exyonq-p14v042-tls-test",
            ])
            .status()
            .expect("openssl available for ephemeral TLS fixtures");
        assert!(status.success(), "openssl ephemeral cert generation failed");
        (cert, key)
    }

    #[test]
    fn rejects_missing_private_key() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        std::fs::write(&cert, b"not a cert").unwrap();
        let err = load_rustls_config(
            &TlsSettings {
                cert_path: cert,
                key_path: key,
            },
            &[b"h2"],
            None,
        )
        .unwrap_err();
        assert!(
            matches!(
                err.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::NotFound
            ),
            "unexpected error kind: {:?}",
            err.kind()
        );
    }

    #[test]
    fn loads_ephemeral_pkcs8_cert_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
        let cfg = load_rustls_config(
            &TlsSettings {
                cert_path: cert,
                key_path: key,
            },
            &[b"h2", b"http/1.1"],
            None,
        )
        .expect("valid PKCS#8 pair must load");
        assert_eq!(cfg.alpn_protocols, [b"h2".to_vec(), b"http/1.1".to_vec()]);
    }

    #[test]
    fn rejects_empty_key_file() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, _) = write_ephemeral_pkcs8_pair(dir.path());
        let empty_key = dir.path().join("empty-key.pem");
        std::fs::write(&empty_key, b"").unwrap();
        let err = load_private_key(&empty_key).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let _ = load_certs(&cert).expect("cert still readable");
    }

    #[test]
    fn rejects_truncated_pem() {
        let dir = tempfile::tempdir().unwrap();
        let truncated = dir.path().join("trunc.pem");
        // Missing END marker must fail closed.
        std::fs::write(&truncated, b"-----BEGIN CERTIFICATE-----\nMIIB\n").unwrap();
        assert!(load_certs(&truncated).is_err());
    }

    #[test]
    fn rejects_pkcs1_rsa_private_key_without_contract_widening() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, pkcs8_key) = write_ephemeral_pkcs8_pair(dir.path());
        let pkcs1 = dir.path().join("pkcs1.pem");
        let status = Command::new("openssl")
            .args(["rsa", "-in"])
            .arg(&pkcs8_key)
            .args(["-traditional", "-out"])
            .arg(&pkcs1)
            .status()
            .expect("openssl rsa");
        assert!(status.success());
        let err = load_private_key(&pkcs1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        // Cert chain still loads independently.
        assert!(!load_certs(&cert).unwrap().is_empty());
    }

    #[test]
    fn shared_acceptor_clear_and_snapshot() {
        let acceptor = SharedTlsAcceptor::new();
        assert!(acceptor.snapshot().is_none());
        acceptor.clear();
        assert!(acceptor.snapshot().is_none());
    }
}
