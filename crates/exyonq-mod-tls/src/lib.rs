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
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::ServerSessionMemoryCache;
use rustls::ServerConfig;
use std::fs::File;
use std::io::{self, BufReader};
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
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn load_private_key(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    if let Some(key) = rustls_pemfile::pkcs8_private_keys(&mut reader).next() {
        return key
            .map(PrivateKeyDer::Pkcs8)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "no PKCS8 private key found",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn shared_acceptor_clear_and_snapshot() {
        let acceptor = SharedTlsAcceptor::new();
        assert!(acceptor.snapshot().is_none());
        acceptor.clear();
        assert!(acceptor.snapshot().is_none());
    }
}
