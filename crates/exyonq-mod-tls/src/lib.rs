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
use rustls::server::{ClientHello, ResolvesServerCert, ServerSessionMemoryCache};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
pub use tokio_rustls::TlsAcceptor;

/// Install rustls ring crypto provider (call once at process startup).
pub fn install_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Prepared, unpublished TLS session-storage epoch.
///
/// Build a candidate acceptor against this epoch first, then publish the epoch
/// immediately before publishing that acceptor. Failed preparation leaves the
/// live epoch untouched.
#[derive(Debug, Clone)]
pub struct TlsSessionCacheEpoch {
    storage: Arc<ServerSessionMemoryCache>,
    storage_identity: u64,
}

impl TlsSessionCacheEpoch {
    fn as_arc(&self) -> Arc<ServerSessionMemoryCache> {
        Arc::clone(&self.storage)
    }
}

#[derive(Debug)]
struct TlsPublicationState {
    generation: u64,
    epoch: TlsSessionCacheEpoch,
}

#[derive(Debug)]
struct TlsSessionCacheInner {
    publication: Mutex<TlsPublicationState>,
    capacity: usize,
}

/// Shared, swappable TLS session/ticket cache (P11 hardening).
///
/// Clones observe the same current epoch. Certificate replacement uses a new
/// epoch so sessions established under old key material cannot resume under a
/// newly published TCP TLS acceptor.
#[derive(Debug, Clone)]
pub struct TlsSessionCache {
    inner: Arc<TlsSessionCacheInner>,
}

impl Default for TlsSessionCache {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl TlsSessionCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(TlsSessionCacheInner {
                publication: Mutex::new(TlsPublicationState {
                    generation: 0,
                    epoch: TlsSessionCacheEpoch {
                        storage: ServerSessionMemoryCache::new(capacity),
                        storage_identity: 0,
                    },
                }),
                capacity,
            }),
        }
    }

    /// Snapshot the current published session-storage epoch.
    pub fn as_arc(&self) -> Arc<ServerSessionMemoryCache> {
        self.snapshot_epoch().as_arc()
    }

    /// Current successful TCP TLS publication generation.
    pub fn publication_generation(&self) -> u64 {
        self.inner
            .publication
            .lock()
            .expect("tls publication lock")
            .generation
    }

    /// Opaque identity of the currently published session-storage epoch.
    pub fn session_storage_identity(&self) -> u64 {
        self.inner
            .publication
            .lock()
            .expect("tls publication lock")
            .epoch
            .storage_identity
    }

    fn snapshot_epoch(&self) -> TlsSessionCacheEpoch {
        self.inner
            .publication
            .lock()
            .expect("tls publication lock")
            .epoch
            .clone()
    }
}

/// Hot-swappable TLS acceptor for certificate reload (ACME / manual PEM rotation).
#[derive(Clone, Default)]
pub struct SharedTlsAcceptor {
    inner: Arc<RwLock<PublishedTls>>,
}

/// One self-contained TCP TLS publication observed by a newly accepted connection.
///
/// The acceptor was configured with the session storage identified by
/// [`PublishedTls::session_storage_identity`]. Certificate and key material are
/// deliberately not exposed.
#[derive(Clone, Default)]
pub struct PublishedTls {
    acceptor: Option<TlsAcceptor>,
    cert_chain_der: Option<Vec<Vec<u8>>>,
    publication_generation: u64,
    session_epoch: Option<TlsSessionCacheEpoch>,
}

impl PublishedTls {
    /// Clone the acceptor captured by this publication (`None` means TCP TLS is disabled).
    pub fn acceptor(&self) -> Option<TlsAcceptor> {
        self.acceptor.clone()
    }

    /// Monotonic generation assigned when this snapshot was published.
    pub fn publication_generation(&self) -> u64 {
        self.publication_generation
    }

    /// Opaque identity of the session storage configured into this snapshot's acceptor.
    pub fn session_storage_identity(&self) -> Option<u64> {
        self.session_epoch
            .as_ref()
            .map(|epoch| epoch.storage_identity)
    }

    /// Whether this publication contains a live TCP TLS acceptor.
    pub fn is_loaded(&self) -> bool {
        self.acceptor.is_some()
    }
}

/// Offline TCP TLS acceptor candidate with its certificate identity and optional new epoch.
pub struct PreparedTlsReload {
    acceptor: Option<TlsAcceptor>,
    cert_chain_der: Option<Vec<Vec<u8>>>,
    epoch: TlsSessionCacheEpoch,
    expected_publication_generation: u64,
}

/// Failure to publish a prepared TCP TLS candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsPublicationError {
    /// Another publisher committed after this candidate captured its base.
    StaleCandidate { expected: u64, actual: u64 },
    /// No further monotonic publication generation can be represented.
    GenerationExhausted,
}

impl std::fmt::Display for TlsPublicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleCandidate { expected, actual } => write!(
                f,
                "stale TLS publication candidate (expected generation {expected}, current {actual})"
            ),
            Self::GenerationExhausted => f.write_str("TLS publication generation exhausted"),
        }
    }
}

impl std::error::Error for TlsPublicationError {}

struct PublicationBase {
    generation: u64,
    epoch: TlsSessionCacheEpoch,
    cert_chain_der: Option<Vec<Vec<u8>>>,
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
        let prepared = self.prepare_reload(settings, session_cache, alpn, true)?;
        self.install_reload(prepared, session_cache)
            .map(|_| ())
            .map_err(publication_io_error)
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

    /// Build a `TlsAcceptor` without publishing it to the shared slot (Cap013 prepare).
    pub fn prepare(
        settings: &TlsSettings,
        session_cache: &TlsSessionCache,
        alpn: &[&[u8]],
    ) -> io::Result<TlsAcceptor> {
        let config = load_rustls_config_with_storage(settings, alpn, Some(session_cache.as_arc()))?;
        Ok(TlsAcceptor::from(Arc::new(config)))
    }

    /// Build a `TlsAcceptor` against an unpublished session-storage epoch.
    pub fn prepare_with_epoch(
        settings: &TlsSettings,
        epoch: &TlsSessionCacheEpoch,
        alpn: &[&[u8]],
    ) -> io::Result<TlsAcceptor> {
        let config = load_rustls_config_with_storage(settings, alpn, Some(epoch.as_arc()))?;
        Ok(TlsAcceptor::from(Arc::new(config)))
    }

    /// Several certificates on one listener. The first certificate is the fallback
    /// for a ClientHello whose name is not listed. Names are matched case-insensitively.
    pub fn prepare_sni_reload(
        &self,
        identities: &[TlsSniCertificate],
        session_cache: &TlsSessionCache,
        alpn: &[&[u8]],
        force_fresh_epoch: bool,
    ) -> io::Result<PreparedTlsReload> {
        let base = self.capture_publication_base(session_cache)?;
        let (config_without_storage, cert_chain_der) = build_sni_material(identities, alpn, None)?;
        let _ = config_without_storage;
        let material_changed = base.cert_chain_der.as_ref() != Some(&cert_chain_der);
        let epoch = if force_fresh_epoch || material_changed {
            TlsSessionCacheEpoch {
                storage: ServerSessionMemoryCache::new(session_cache.inner.capacity),
                storage_identity: base
                    .generation
                    .checked_add(1)
                    .ok_or_else(publication_generation_exhausted_io)?,
            }
        } else {
            base.epoch
        };
        let (config, _) = build_sni_material(identities, alpn, Some(epoch.as_arc()))?;
        Ok(PreparedTlsReload {
            acceptor: Some(TlsAcceptor::from(Arc::new(config))),
            cert_chain_der: Some(cert_chain_der),
            epoch,
            expected_publication_generation: base.generation,
        })
    }

    /// Prepare a reload candidate from one material read.
    ///
    /// A fresh session epoch is selected when explicitly required or when the
    /// certificate chain differs from the currently published TCP acceptor.
    pub fn prepare_reload(
        &self,
        settings: &TlsSettings,
        session_cache: &TlsSessionCache,
        alpn: &[&[u8]],
        force_fresh_epoch: bool,
    ) -> io::Result<PreparedTlsReload> {
        // Capture the base before reading files. If another publisher commits
        // while file I/O/config construction is in progress, commit rejects
        // this candidate instead of letting an older read overwrite it.
        let base = self.capture_publication_base(session_cache)?;
        let (certs, key) = load_tls_material(settings)?;
        let cert_chain_der = certs
            .iter()
            .map(|cert| cert.as_ref().to_vec())
            .collect::<Vec<_>>();
        let material_changed = base.cert_chain_der.as_ref() != Some(&cert_chain_der);
        let epoch = if force_fresh_epoch || material_changed {
            TlsSessionCacheEpoch {
                storage: ServerSessionMemoryCache::new(session_cache.inner.capacity),
                storage_identity: base
                    .generation
                    .checked_add(1)
                    .ok_or_else(publication_generation_exhausted_io)?,
            }
        } else {
            base.epoch
        };
        let storage = epoch.as_arc();
        let config = build_rustls_config(certs, key, alpn, Some(storage))?;
        Ok(PreparedTlsReload {
            acceptor: Some(TlsAcceptor::from(Arc::new(config))),
            cert_chain_der: Some(cert_chain_der),
            epoch,
            expected_publication_generation: base.generation,
        })
    }

    /// Prepare an unpublished clear operation.
    pub fn prepare_clear(
        &self,
        session_cache: &TlsSessionCache,
        force_fresh_epoch: bool,
    ) -> io::Result<PreparedTlsReload> {
        let base = self.capture_publication_base(session_cache)?;
        let epoch = if force_fresh_epoch {
            TlsSessionCacheEpoch {
                storage: ServerSessionMemoryCache::new(session_cache.inner.capacity),
                storage_identity: base
                    .generation
                    .checked_add(1)
                    .ok_or_else(publication_generation_exhausted_io)?,
            }
        } else {
            base.epoch
        };
        Ok(PreparedTlsReload {
            acceptor: None,
            cert_chain_der: None,
            epoch,
            expected_publication_generation: base.generation,
        })
    }

    /// Publish a prepared acceptor and its exact session epoch.
    ///
    /// Every publisher uses the cache-owned serialization mutex. A candidate
    /// prepared from an older generation is rejected without mutation.
    pub fn install_reload(
        &self,
        prepared: PreparedTlsReload,
        session_cache: &TlsSessionCache,
    ) -> Result<u64, TlsPublicationError> {
        let mut publication = session_cache
            .inner
            .publication
            .lock()
            .expect("tls publication lock");
        if publication.generation != prepared.expected_publication_generation {
            return Err(TlsPublicationError::StaleCandidate {
                expected: prepared.expected_publication_generation,
                actual: publication.generation,
            });
        }
        let next_generation = publication
            .generation
            .checked_add(1)
            .ok_or(TlsPublicationError::GenerationExhausted)?;
        let published = PublishedTls {
            acceptor: prepared.acceptor,
            cert_chain_der: prepared.cert_chain_der,
            publication_generation: next_generation,
            session_epoch: Some(prepared.epoch.clone()),
        };

        // Keep the publication mutex across both writes. Acceptor readers get
        // a self-contained storage Arc; future preparations cannot observe a
        // half-published cache/acceptor pair.
        *self.inner.write().expect("tls acceptor lock") = published;
        publication.epoch = prepared.epoch;
        publication.generation = next_generation;
        Ok(next_generation)
    }

    /// Serialize a TCP TLS clear with all other publishers.
    pub fn clear(&self, session_cache: &TlsSessionCache) -> io::Result<u64> {
        let prepared = self.prepare_clear(session_cache, true)?;
        self.install_reload(prepared, session_cache)
            .map_err(publication_io_error)
    }

    /// Snapshot acceptor, publication generation, and storage identity together.
    pub fn snapshot(&self) -> PublishedTls {
        self.inner.read().expect("tls acceptor lock").clone()
    }

    pub fn is_loaded(&self) -> bool {
        self.snapshot().is_loaded()
    }

    fn capture_publication_base(
        &self,
        session_cache: &TlsSessionCache,
    ) -> io::Result<PublicationBase> {
        // Lock order is always publication -> acceptor. Snapshot-only readers
        // take only the acceptor lock, so this is non-reentrant and deadlock-free.
        let publication = session_cache
            .inner
            .publication
            .lock()
            .expect("tls publication lock");
        let published = self.inner.read().expect("tls acceptor lock");
        if published.publication_generation != publication.generation
            || published.session_epoch.as_ref().is_some_and(|epoch| {
                epoch.storage_identity != publication.epoch.storage_identity
                    || !Arc::ptr_eq(&epoch.storage, &publication.epoch.storage)
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "TLS acceptor and session cache are not one publication pair",
            ));
        }
        Ok(PublicationBase {
            generation: publication.generation,
            epoch: publication.epoch.clone(),
            cert_chain_der: published.cert_chain_der.clone(),
        })
    }
}

fn publication_generation_exhausted_io() -> io::Error {
    io::Error::other(TlsPublicationError::GenerationExhausted)
}

fn publication_io_error(err: TlsPublicationError) -> io::Error {
    let kind = match err {
        TlsPublicationError::StaleCandidate { .. } => io::ErrorKind::WouldBlock,
        TlsPublicationError::GenerationExhausted => io::ErrorKind::Other,
    };
    io::Error::new(kind, err)
}

#[derive(Debug, Clone)]
pub struct TlsSettings {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

/// One certificate and the names that select it during the TLS handshake.
#[derive(Debug, Clone)]
pub struct TlsSniCertificate {
    pub names: Vec<String>,
    pub settings: TlsSettings,
}

struct SniResolver {
    by_name: HashMap<String, Arc<CertifiedKey>>,
    default: Arc<CertifiedKey>,
}

impl std::fmt::Debug for SniResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SniResolver")
            .field("names", &self.by_name.len())
            .finish()
    }
}

impl ResolvesServerCert for SniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if let Some(name) = client_hello.server_name() {
            if let Some(key) = self.by_name.get(&name.to_ascii_lowercase()) {
                return Some(Arc::clone(key));
            }
        }
        Some(Arc::clone(&self.default))
    }
}

fn build_sni_material(
    identities: &[TlsSniCertificate],
    alpn: &[&[u8]],
    storage: Option<Arc<ServerSessionMemoryCache>>,
) -> io::Result<(ServerConfig, Vec<Vec<u8>>)> {
    if identities.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no tls certificates",
        ));
    }
    let mut by_name = HashMap::new();
    let mut cert_chain_der = Vec::new();
    let mut default = None;
    for (index, identity) in identities.iter().enumerate() {
        let (certs, key) = load_tls_material(&identity.settings)?;
        let chain = certs.iter().map(|cert| cert.as_ref().to_vec()).collect::<Vec<_>>();
        cert_chain_der.extend(chain);
        let signing_key = rustls::crypto::ring::sign::any_supported_type(&key).map_err(|err| {
            io::Error::new(io::ErrorKind::InvalidData, err.to_string())
        })?;
        let certified = Arc::new(CertifiedKey::new(certs, signing_key));
        if index == 0 {
            default = Some(Arc::clone(&certified));
        }
        for name in &identity.names {
            let normalized = name.trim_end_matches('.').to_ascii_lowercase();
            if normalized.is_empty() {
                continue;
            }
            if by_name.insert(normalized, Arc::clone(&certified)).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("duplicate tls server name {name}"),
                ));
            }
        }
    }
    let default = default.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "no default tls certificate")
    })?;
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(SniResolver { by_name, default }));
    config.alpn_protocols = alpn.iter().map(|proto| proto.to_vec()).collect();
    if let Some(storage) = storage {
        config.session_storage = storage;
    }
    Ok((config, cert_chain_der))
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
    load_rustls_config_with_storage(settings, alpn, session_cache.map(TlsSessionCache::as_arc))
}

fn load_rustls_config_with_storage(
    settings: &TlsSettings,
    alpn: &[&[u8]],
    session_storage: Option<Arc<ServerSessionMemoryCache>>,
) -> io::Result<ServerConfig> {
    let (certs, key) = load_tls_material(settings)?;
    build_rustls_config(certs, key, alpn, session_storage)
}

fn load_tls_material(
    settings: &TlsSettings,
) -> io::Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let certs = load_certs(&settings.cert_path)?;
    let key = load_private_key(&settings.key_path)?;
    Ok((certs, key))
}

fn build_rustls_config(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    alpn: &[&[u8]],
    session_storage: Option<Arc<ServerSessionMemoryCache>>,
) -> io::Result<ServerConfig> {
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    config.alpn_protocols = alpn.iter().map(|proto| proto.to_vec()).collect();
    if let Some(storage) = session_storage {
        config.session_storage = storage;
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
    use std::sync::Barrier;
    use std::thread;

    /// Prefer OpenSSL 3 over Darwin LibreSSL. LibreSSL rejects `-traditional`
    /// and can emit cert material rustls rejects (`UnsupportedCertVersion`).
    fn openssl_bin() -> &'static str {
        for cand in [
            "/opt/homebrew/opt/openssl@3/bin/openssl",
            "/usr/local/opt/openssl@3/bin/openssl",
        ] {
            if Path::new(cand).is_file() {
                return cand;
            }
        }
        "openssl"
    }

    fn write_ephemeral_pkcs8_pair(dir: &Path) -> (PathBuf, PathBuf) {
        write_named_ephemeral_pkcs8_pair(dir, "tls", "exyonq-p14v042-tls-test")
    }

    fn write_named_ephemeral_pkcs8_pair(
        dir: &Path,
        prefix: &str,
        common_name: &str,
    ) -> (PathBuf, PathBuf) {
        install_rustls_provider();
        let cert = dir.join(format!("{prefix}-cert.pem"));
        let key = dir.join(format!("{prefix}-key.pem"));
        let subject = format!("/CN={common_name}");
        let status = Command::new(openssl_bin())
            .args(["req", "-x509", "-newkey", "rsa:2048", "-keyout"])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .args(["-days", "1", "-nodes", "-subj", &subject])
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
        let status = Command::new(openssl_bin())
            .args(["rsa", "-in"])
            .arg(&pkcs8_key)
            .args(["-traditional", "-out"])
            .arg(&pkcs1)
            .status()
            .expect("openssl rsa");
        assert!(
            status.success(),
            "openssl rsa -traditional failed (need OpenSSL 3+, not LibreSSL)"
        );
        let err = load_private_key(&pkcs1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        // Cert chain still loads independently.
        assert!(!load_certs(&cert).unwrap().is_empty());
    }

    #[test]
    fn shared_acceptor_clear_and_snapshot() {
        let acceptor = SharedTlsAcceptor::new();
        let cache = TlsSessionCache::new(8);
        assert!(!acceptor.snapshot().is_loaded());
        assert_eq!(acceptor.clear(&cache).unwrap(), 1);
        let published = acceptor.snapshot();
        assert!(!published.is_loaded());
        assert_eq!(published.publication_generation(), 1);
        assert_eq!(
            published.session_storage_identity(),
            Some(cache.snapshot_epoch().storage_identity)
        );
    }

    #[test]
    fn session_cache_publication_is_shared_across_clones() {
        let cache = TlsSessionCache::new(8);
        let clone = cache.clone();
        let original = cache.as_arc();
        assert!(Arc::ptr_eq(&original, &clone.as_arc()));

        let acceptor = SharedTlsAcceptor::new();
        acceptor.clear(&clone).unwrap();
        let published = cache.as_arc();
        assert!(!Arc::ptr_eq(&original, &published));
        assert!(Arc::ptr_eq(&published, &clone.as_arc()));
        assert_eq!(cache.publication_generation(), 1);
        assert_eq!(clone.publication_generation(), 1);
    }

    #[test]
    fn reload_candidate_rotates_epoch_only_for_changed_valid_material() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
        let settings = TlsSettings {
            cert_path: cert.clone(),
            key_path: key,
        };
        let acceptor = SharedTlsAcceptor::new();
        let cache = TlsSessionCache::new(8);
        acceptor
            .load(&settings, &cache, &[b"h2", b"http/1.1"])
            .unwrap();
        let first_epoch = cache.as_arc();

        let unchanged = acceptor
            .prepare_reload(&settings, &cache, &[b"h2", b"http/1.1"], false)
            .unwrap();
        acceptor.install_reload(unchanged, &cache).unwrap();
        assert!(Arc::ptr_eq(&first_epoch, &cache.as_arc()));

        let _ = write_ephemeral_pkcs8_pair(dir.path());
        let changed = acceptor
            .prepare_reload(&settings, &cache, &[b"h2", b"http/1.1"], false)
            .unwrap();
        acceptor.install_reload(changed, &cache).unwrap();
        let changed_epoch = cache.as_arc();
        assert!(!Arc::ptr_eq(&first_epoch, &changed_epoch));

        std::fs::write(cert, b"invalid pem").unwrap();
        assert!(acceptor
            .prepare_reload(&settings, &cache, &[b"h2", b"http/1.1"], false)
            .is_err());
        assert!(Arc::ptr_eq(&changed_epoch, &cache.as_arc()));
        assert!(acceptor.is_loaded());
    }

    fn assert_acceptor_uses_snapshot_storage(published: &PublishedTls) {
        let acceptor = published.acceptor.as_ref().expect("loaded acceptor");
        let epoch = published.session_epoch.as_ref().expect("published epoch");
        let configured_storage = &acceptor.config().session_storage;
        let expected_storage: Arc<dyn rustls::server::StoresServerSessions> = epoch.storage.clone();
        assert!(
            Arc::ptr_eq(configured_storage, &expected_storage),
            "acceptor must retain the exact storage Arc in its publication snapshot"
        );
    }

    fn assert_current_pair(acceptor: &SharedTlsAcceptor, cache: &TlsSessionCache) -> PublishedTls {
        let publication = cache
            .inner
            .publication
            .lock()
            .expect("tls publication lock");
        let published = acceptor.snapshot();
        let epoch = published.session_epoch.as_ref().expect("published epoch");
        assert_eq!(published.publication_generation(), publication.generation);
        assert_eq!(epoch.storage_identity, publication.epoch.storage_identity);
        assert!(Arc::ptr_eq(&epoch.storage, &publication.epoch.storage));
        if published.is_loaded() {
            assert_acceptor_uses_snapshot_storage(&published);
        }
        published
    }

    fn run_ordered_publish_race(
        first_settings: TlsSettings,
        stale_settings: TlsSettings,
        acceptor: &SharedTlsAcceptor,
        cache: &TlsSessionCache,
    ) -> (
        Result<u64, TlsPublicationError>,
        Result<u64, TlsPublicationError>,
    ) {
        let prepare_start = Arc::new(Barrier::new(3));
        let prepare_done = Arc::new(Barrier::new(3));
        let (first_done_tx, first_done_rx) = std::sync::mpsc::channel();

        let first_acceptor = acceptor.clone();
        let first_cache = cache.clone();
        let first_prepare_start = Arc::clone(&prepare_start);
        let first_prepare_done = Arc::clone(&prepare_done);
        let first_thread = thread::spawn(move || {
            first_prepare_start.wait();
            let first = first_acceptor
                .prepare_reload(&first_settings, &first_cache, &[b"h2"], true)
                .expect("prepare first candidate");
            first_prepare_done.wait();
            let result = first_acceptor.install_reload(first, &first_cache);
            first_done_tx.send(()).expect("signal first publish");
            result
        });

        let stale_acceptor = acceptor.clone();
        let stale_cache = cache.clone();
        let stale_prepare_start = Arc::clone(&prepare_start);
        let stale_prepare_done = Arc::clone(&prepare_done);
        let stale_thread = thread::spawn(move || {
            stale_prepare_start.wait();
            let stale = stale_acceptor
                .prepare_reload(&stale_settings, &stale_cache, &[b"h2"], true)
                .expect("prepare stale candidate");
            stale_prepare_done.wait();
            first_done_rx.recv().expect("wait for first publish");
            stale_acceptor.install_reload(stale, &stale_cache)
        });

        prepare_start.wait();
        prepare_done.wait();
        (
            first_thread.join().expect("first publisher"),
            stale_thread.join().expect("stale publisher"),
        )
    }

    #[test]
    fn concurrent_acme_and_config_candidates_reject_stale_in_both_orders() {
        let dir = tempfile::tempdir().unwrap();
        let (old_cert, old_key) =
            write_named_ephemeral_pkcs8_pair(dir.path(), "old", "old.exyonq.test");
        let (new_cert, new_key) =
            write_named_ephemeral_pkcs8_pair(dir.path(), "new", "new.exyonq.test");
        let old = TlsSettings {
            cert_path: old_cert,
            key_path: old_key,
        };
        let new = TlsSettings {
            cert_path: new_cert,
            key_path: new_key,
        };

        for config_publishes_first in [true, false] {
            let acceptor = SharedTlsAcceptor::new();
            let cache = TlsSessionCache::new(8);
            acceptor.load(&old, &cache, &[b"h2"]).unwrap();

            // ACME and config prepare concurrently from generation 1. A second
            // barrier makes both candidates ready before either can publish.
            let (first_settings, stale_settings) = if config_publishes_first {
                (old.clone(), new.clone())
            } else {
                (new.clone(), old.clone())
            };
            let expected_first_chain = load_certs(&first_settings.cert_path)
                .unwrap()
                .into_iter()
                .map(|cert| cert.as_ref().to_vec())
                .collect::<Vec<_>>();

            let (first_result, stale_result) =
                run_ordered_publish_race(first_settings, stale_settings, &acceptor, &cache);
            assert_eq!(first_result.unwrap(), 2);
            assert_eq!(
                stale_result.unwrap_err(),
                TlsPublicationError::StaleCandidate {
                    expected: 1,
                    actual: 2,
                }
            );
            let published = assert_current_pair(&acceptor, &cache);
            assert_eq!(published.publication_generation(), 2);
            assert_eq!(
                published.cert_chain_der.as_ref(),
                Some(&expected_first_chain),
                "stale finisher must not overwrite the first committed candidate"
            );
        }
    }

    #[test]
    fn initial_config_runtime_and_clear_publishers_share_one_generation() {
        let dir = tempfile::tempdir().unwrap();
        let (cert_a, key_a) = write_named_ephemeral_pkcs8_pair(dir.path(), "a", "a.exyonq.test");
        let (cert_b, key_b) = write_named_ephemeral_pkcs8_pair(dir.path(), "b", "b.exyonq.test");
        let settings_a = TlsSettings {
            cert_path: cert_a,
            key_path: key_a,
        };
        let settings_b = TlsSettings {
            cert_path: cert_b,
            key_path: key_b,
        };
        let acceptor = SharedTlsAcceptor::new();
        let cache = TlsSessionCache::new(8);

        acceptor.load(&settings_a, &cache, &[b"h2"]).unwrap();
        let initial = assert_current_pair(&acceptor, &cache);
        assert_eq!(initial.publication_generation(), 1);

        let config = acceptor
            .prepare_reload(&settings_b, &cache, &[b"h2"], true)
            .unwrap();
        assert_eq!(acceptor.install_reload(config, &cache).unwrap(), 2);
        let config_storage = assert_current_pair(&acceptor, &cache)
            .session_storage_identity()
            .unwrap();

        let runtime = acceptor
            .prepare_reload(&settings_b, &cache, &[b"h2"], false)
            .unwrap();
        assert_eq!(acceptor.install_reload(runtime, &cache).unwrap(), 3);
        assert_eq!(
            assert_current_pair(&acceptor, &cache).session_storage_identity(),
            Some(config_storage),
            "unchanged runtime TLS must keep the configured session epoch"
        );

        assert_eq!(acceptor.clear(&cache).unwrap(), 4);
        let cleared = assert_current_pair(&acceptor, &cache);
        assert!(!cleared.is_loaded());
        assert_eq!(cleared.publication_generation(), 4);
        assert_ne!(
            cleared.session_storage_identity(),
            Some(config_storage),
            "clear must rotate session storage"
        );
    }

    #[test]
    fn concurrent_publication_stress_has_no_deadlock_or_storage_mismatch() {
        const THREADS: usize = 4;
        const SUCCESSES_PER_THREAD: usize = 32;

        let dir = tempfile::tempdir().unwrap();
        let (cert_a, key_a) =
            write_named_ephemeral_pkcs8_pair(dir.path(), "stress-a", "stress-a.exyonq.test");
        let (cert_b, key_b) =
            write_named_ephemeral_pkcs8_pair(dir.path(), "stress-b", "stress-b.exyonq.test");
        let settings = Arc::new([
            TlsSettings {
                cert_path: cert_a,
                key_path: key_a,
            },
            TlsSettings {
                cert_path: cert_b,
                key_path: key_b,
            },
        ]);
        let acceptor = SharedTlsAcceptor::new();
        let cache = TlsSessionCache::new(32);
        acceptor.load(&settings[0], &cache, &[b"h2"]).unwrap();
        let start = Arc::new(Barrier::new(THREADS));

        let handles = (0..THREADS)
            .map(|thread_id| {
                let acceptor = acceptor.clone();
                let cache = cache.clone();
                let settings = Arc::clone(&settings);
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    let mut published = 0;
                    while published < SUCCESSES_PER_THREAD {
                        let candidate = acceptor
                            .prepare_reload(
                                &settings[(thread_id + published) % settings.len()],
                                &cache,
                                &[b"h2"],
                                true,
                            )
                            .expect("prepare stress candidate");
                        match acceptor.install_reload(candidate, &cache) {
                            Ok(_) => published += 1,
                            Err(TlsPublicationError::StaleCandidate { .. }) => continue,
                            Err(err) => panic!("unexpected publication error: {err}"),
                        }
                        assert_acceptor_uses_snapshot_storage(&acceptor.snapshot());
                    }
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().expect("stress publisher");
        }
        let final_publication = assert_current_pair(&acceptor, &cache);
        assert_eq!(
            final_publication.publication_generation(),
            1 + (THREADS * SUCCESSES_PER_THREAD) as u64
        );
    }
}
