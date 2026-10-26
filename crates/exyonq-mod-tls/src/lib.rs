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
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ProducesTickets, ResolvesServerCert, ServerSessionMemoryCache};
use rustls::sign::CertifiedKey;
use rustls::{CipherSuite, ConfigBuilder, NamedGroup, ServerConfig, WantsVerifier};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
pub use tokio_rustls::TlsAcceptor;

/// Install rustls ring crypto provider (call once at process startup).
pub fn install_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Shared TLS session/ticket state reused across reloads (P11 hardening, NG-14).
/// Carries the stateful session-ID store plus a rotating stateless ticket
/// producer so resumption survives acceptor rebuilds.
#[derive(Debug, Clone)]
pub struct TlsSessionCache {
    inner: Arc<ServerSessionMemoryCache>,
    /// rustls `TicketRotator`: Chacha20Poly1305, keys rotate every 6h and
    /// tickets stay decryptable for 12h. `None` only if OS RNG fails —
    /// session-ID resumption still works via `inner`.
    ticketer: Option<Arc<dyn ProducesTickets>>,
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
            ticketer: rustls::crypto::ring::Ticketer::new().ok(),
        }
    }

    pub fn as_arc(&self) -> Arc<ServerSessionMemoryCache> {
        Arc::clone(&self.inner)
    }

    pub fn ticketer(&self) -> Option<Arc<dyn ProducesTickets>> {
        self.ticketer.clone()
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
        let acceptor = Self::prepare(settings, session_cache, alpn)?;
        self.install(Some(acceptor));
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

    /// Build a `TlsAcceptor` without publishing it to the shared slot (Cap013 prepare).
    pub fn prepare(
        settings: &TlsSettings,
        session_cache: &TlsSessionCache,
        alpn: &[&[u8]],
    ) -> io::Result<TlsAcceptor> {
        let config = load_rustls_config(settings, alpn, Some(session_cache))?;
        Ok(TlsAcceptor::from(Arc::new(config)))
    }

    /// Build a multi-certificate SNI `TlsAcceptor` without publishing it (NG-15).
    /// `strict_sni` (AP-02) aborts handshakes whose SNI matches no configured name.
    /// `crypto` (NG-12) restricts protocol versions, cipher suites and curves.
    pub fn prepare_multi(
        certs: &[SniCert],
        strict_sni: bool,
        crypto: &TlsCryptoPolicy,
        session_cache: &TlsSessionCache,
        alpn: &[&[u8]],
    ) -> io::Result<TlsAcceptor> {
        let config =
            load_rustls_config_multi(certs, strict_sni, crypto, alpn, Some(session_cache))?;
        Ok(TlsAcceptor::from(Arc::new(config)))
    }

    /// Atomically replace the live acceptor (`None` clears TLS).
    pub fn install(&self, next: Option<TlsAcceptor>) {
        *self.inner.write().expect("tls acceptor lock") = next;
    }

    pub fn clear(&self) {
        self.install(None);
    }

    pub fn snapshot(&self) -> Option<TlsAcceptor> {
        self.inner.read().expect("tls acceptor lock").clone()
    }

    pub fn is_loaded(&self) -> bool {
        self.inner.read().expect("tls acceptor lock").is_some()
    }
}

/// NG-12: listener-level TLS crypto restriction (`[server.tls] protocols`,
/// `ciphers`, `curves`). All-empty means rustls defaults (TLS 1.2+1.3,
/// every AEAD suite and ECDHE group the ring provider ships).
/// Names are pre-validated by config-ir; this type still fails closed on
/// unknown tokens because it is a public runtime boundary.
#[derive(Debug, Clone, Default)]
pub struct TlsCryptoPolicy {
    pub protocols: Vec<String>,
    pub ciphers: Vec<String>,
    pub curves: Vec<String>,
}

impl TlsCryptoPolicy {
    fn is_default(&self) -> bool {
        self.protocols.is_empty() && self.ciphers.is_empty() && self.curves.is_empty()
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
        if let Some(ticketer) = cache.ticketer() {
            config.ticketer = ticketer;
        }
    }
    Ok(config)
}

/// One certificate bound to the `server_name` set of a `[[server]]` block (NG-15).
/// `names` may be empty: that certificate can still serve as the listener default.
#[derive(Debug, Clone)]
pub struct SniCert {
    /// Exact DNS names this certificate claims (case-insensitive).
    pub names: Vec<String>,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    /// NG-13: DER OCSP response stapled to this certificate (`None` = no
    /// stapling). Loaded at acceptor build; missing/empty file fails closed.
    pub ocsp_path: Option<PathBuf>,
}

/// rustls SNI resolver: exact `server_name` match, then nginx-style `*.suffix`
/// wildcard, else the first configured certificate (deterministic default,
/// mirrors the former `servers[0]` cert).
#[derive(Debug)]
struct SniResolver {
    by_name: HashMap<String, Arc<CertifiedKey>>,
    default: Arc<CertifiedKey>,
    /// AP-02: when true, unknown SNI aborts the handshake instead of
    /// falling back to `default`.
    strict: bool,
}

impl ResolvesServerCert for SniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if let Some(name) = client_hello.server_name() {
            let lower = name.trim_end_matches('.').to_ascii_lowercase();
            if let Some(key) = self.by_name.get(lower.as_str()) {
                return Some(Arc::clone(key));
            }
            let mut rest = lower.as_str();
            while let Some((_, suffix)) = rest.split_once('.') {
                let wildcard = format!("*.{suffix}");
                if let Some(key) = self.by_name.get(wildcard.as_str()) {
                    return Some(Arc::clone(key));
                }
                rest = suffix;
            }
        }
        if self.strict {
            None
        } else {
            Some(Arc::clone(&self.default))
        }
    }
}

/// Build a rustls `ServerConfig` that selects one of `certs` by SNI (NG-15).
/// `certs[0]` is the deterministic default for missing/unknown SNI.
///
/// # Errors
/// `InvalidInput` when `certs` is empty or two entries claim the same name;
/// the underlying PEM/key load errors otherwise.
pub fn load_rustls_config_multi(
    certs: &[SniCert],
    strict_sni: bool,
    crypto: &TlsCryptoPolicy,
    alpn: &[&[u8]],
    session_cache: Option<&TlsSessionCache>,
) -> io::Result<ServerConfig> {
    if certs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tls: no certificates configured",
        ));
    }
    if certs.len() == 1
        && certs[0].names.is_empty()
        && crypto.is_default()
        && !strict_sni
        && certs[0].ocsp_path.is_none()
    {
        // Identical to the historical single-cert path.
        return load_rustls_config(
            &TlsSettings {
                cert_path: certs[0].cert_path.clone(),
                key_path: certs[0].key_path.clone(),
            },
            alpn,
            session_cache,
        );
    }
    let mut by_name = HashMap::new();
    let mut default = None;
    for (idx, entry) in certs.iter().enumerate() {
        let certified = Arc::new(load_certified_key(idx, entry)?);
        if idx == 0 {
            default = Some(Arc::clone(&certified));
        }
        for name in &entry.names {
            let key = name.trim_end_matches('.').to_ascii_lowercase();
            if by_name
                .insert(key.clone(), Arc::clone(&certified))
                .is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("tls: duplicate server_name {key:?} on shared listener"),
                ));
            }
        }
    }
    let default = default.expect("non-empty certs");
    let mut config = crypto_builder(crypto)?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(SniResolver {
            by_name,
            default,
            strict: strict_sni,
        }));
    config.alpn_protocols = alpn.iter().map(|proto| proto.to_vec()).collect();
    if let Some(cache) = session_cache {
        config.session_storage = cache.as_arc();
        if let Some(ticketer) = cache.ticketer() {
            config.ticketer = ticketer;
        }
    }
    Ok(config)
}

/// Load one `SniCert` into a rustls `CertifiedKey`: cert chain, matching
/// private key, optional stapled OCSP (NG-13, fails closed on missing/empty).
fn load_certified_key(idx: usize, entry: &SniCert) -> io::Result<CertifiedKey> {
    let chain = load_certs(&entry.cert_path).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("tls cert {} ({:?}): {err}", entry.cert_path.display(), idx),
        )
    })?;
    let key = load_private_key(&entry.key_path).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("tls key {} ({:?}): {err}", entry.key_path.display(), idx),
        )
    })?;
    let signing_key = rustls::crypto::ring::sign::any_supported_type(&key).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("tls key {} ({:?}): {err}", entry.key_path.display(), idx),
        )
    })?;
    let mut certified_key = CertifiedKey::new(chain, signing_key);
    if let Some(ocsp_path) = &entry.ocsp_path {
        let der = std::fs::read(ocsp_path).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("tls ocsp {}: {err}", ocsp_path.display()),
            )
        })?;
        if der.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("tls ocsp {}: empty response", ocsp_path.display()),
            ));
        }
        certified_key.ocsp = Some(der);
    }
    certified_key.keys_match().map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "tls pair {} cert/key do not correspond: {err}",
                entry.cert_path.display()
            ),
        )
    })?;
    Ok(certified_key)
}

/// NG-12: resolve `TlsCryptoPolicy` into a rustls builder. An all-default
/// policy keeps the stock builder; any restriction rebuilds a filtered
/// ring `CryptoProvider` plus an explicit protocol-version list.
/// Fails closed: unknown tokens and policies that leave no usable suite
/// are `InvalidInput`.
fn crypto_builder(
    crypto: &TlsCryptoPolicy,
) -> io::Result<ConfigBuilder<ServerConfig, WantsVerifier>> {
    if crypto.is_default() {
        return Ok(ServerConfig::builder());
    }
    let invalid = |token: &str| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("tls: unsupported crypto token {token:?}"),
        )
    };
    let default_provider = rustls::crypto::ring::default_provider();
    let cipher_suites = if crypto.ciphers.is_empty() {
        default_provider.cipher_suites.clone()
    } else {
        let mut wanted = Vec::new();
        for entry in &crypto.ciphers {
            for token in entry.split(':').map(str::trim).filter(|s| !s.is_empty()) {
                wanted.push(cipher_suite_token(token).ok_or_else(|| invalid(token))?);
            }
        }
        rustls::crypto::ring::ALL_CIPHER_SUITES
            .iter()
            .copied()
            .filter(|suite| wanted.contains(&suite.suite()))
            .collect()
    };
    let kx_groups = if crypto.curves.is_empty() {
        default_provider.kx_groups.clone()
    } else {
        let mut wanted = Vec::new();
        for entry in &crypto.curves {
            for token in entry.split(':').map(str::trim).filter(|s| !s.is_empty()) {
                wanted.push(named_group_token(token).ok_or_else(|| invalid(token))?);
            }
        }
        rustls::crypto::ring::ALL_KX_GROUPS
            .iter()
            .copied()
            .filter(|group| wanted.contains(&group.name()))
            .collect()
    };
    let protocols: Vec<&'static rustls::SupportedProtocolVersion> = if crypto.protocols.is_empty() {
        vec![&rustls::version::TLS12, &rustls::version::TLS13]
    } else {
        crypto
            .protocols
            .iter()
            .map(|proto| match proto.trim() {
                "TLSv1.2" | "TLS1.2" | "1.2" => Ok(&rustls::version::TLS12),
                "TLSv1.3" | "TLS1.3" | "1.3" => Ok(&rustls::version::TLS13),
                other => Err(invalid(other)),
            })
            .collect::<io::Result<_>>()?
    };
    ServerConfig::builder_with_provider(Arc::new(CryptoProvider {
        cipher_suites,
        kx_groups,
        ..default_provider
    }))
    .with_protocol_versions(&protocols)
    .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))
}

/// IANA or OpenSSL spelling → rustls `CipherSuite`. Must stay in sync with
/// `config-ir::validate_tls_crypto` (which rejects unknown names earlier).
fn cipher_suite_token(name: &str) -> Option<CipherSuite> {
    Some(match name.to_ascii_uppercase().as_str() {
        "TLS13_AES_256_GCM_SHA384" | "TLS_AES_256_GCM_SHA384" => {
            CipherSuite::TLS13_AES_256_GCM_SHA384
        }
        "TLS13_AES_128_GCM_SHA256" | "TLS_AES_128_GCM_SHA256" => {
            CipherSuite::TLS13_AES_128_GCM_SHA256
        }
        "TLS13_CHACHA20_POLY1305_SHA256" | "TLS_CHACHA20_POLY1305_SHA256" => {
            CipherSuite::TLS13_CHACHA20_POLY1305_SHA256
        }
        "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384" | "ECDHE-ECDSA-AES256-GCM-SHA384" => {
            CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
        }
        "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256" | "ECDHE-ECDSA-AES128-GCM-SHA256" => {
            CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
        }
        "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256" | "ECDHE-ECDSA-CHACHA20-POLY1305" => {
            CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
        }
        "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384" | "ECDHE-RSA-AES256-GCM-SHA384" => {
            CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384
        }
        "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256" | "ECDHE-RSA-AES128-GCM-SHA256" => {
            CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
        }
        "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256" | "ECDHE-RSA-CHACHA20-POLY1305" => {
            CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256
        }
        _ => return None,
    })
}

fn named_group_token(name: &str) -> Option<NamedGroup> {
    Some(match name.to_ascii_lowercase().as_str() {
        "x25519" => NamedGroup::X25519,
        "secp256r1" | "prime256v1" | "p-256" | "p256" => NamedGroup::secp256r1,
        "secp384r1" | "p-384" | "p384" => NamedGroup::secp384r1,
        _ => return None,
    })
}

/// Q-09: `notAfter` (unix epoch) of the first PEM certificate in `path`,
/// or `None` when it cannot be read/parsed. Used for status visibility.
pub fn pem_leaf_not_after_epoch(path: &Path) -> Option<u64> {
    let der = CertificateDer::pem_file_iter(path).ok()?.next()?.ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
    u64::try_from(cert.validity().not_after.timestamp()).ok()
}

fn load_certs(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    let certs = CertificateDer::pem_file_iter(path)
        .map_err(pem_io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(pem_io_error)?;
    if certs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("no CERTIFICATE blocks in {}", path.display()),
        ));
    }
    Ok(certs)
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
        install_rustls_provider();
        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");
        let status = Command::new(openssl_bin())
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

    /// Ephemeral pair whose leaf carries the given `subjectAltName` — rustls
    /// ignores CN, so SNI tests need real SANs.
    fn write_ephemeral_pair_with_san(dir: &Path, stem: &str, san: &str) -> (PathBuf, PathBuf) {
        let cert = dir.join(format!("{stem}.pem"));
        let key = dir.join(format!("{stem}.key"));
        let status = Command::new(openssl_bin())
            .args(["req", "-x509", "-newkey", "rsa:2048", "-keyout"])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .args(["-days", "1", "-nodes", "-subj", "/CN=exyonq-ng15-test"])
            .args(["-addext", &format!("subjectAltName={san}")])
            .args(["-addext", "basicConstraints=critical,CA:FALSE"])
            .status()
            .expect("openssl available for ephemeral TLS fixtures");
        assert!(status.success(), "openssl SAN cert generation failed");
        (cert, key)
    }

    /// Accepts any certificate — the point of the test is observing which cert
    /// the server picked, not client validation.
    #[derive(Debug)]
    struct AcceptAnyCert;

    impl rustls::client::danger::ServerCertVerifier for AcceptAnyCert {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    /// Client → server rustls handshake over a real TCP socketpair; returns the
    /// leaf certificate the server selected for `sni`.
    fn handshake_leaf(server_config: Arc<ServerConfig>, sni: &str) -> CertificateDer<'static> {
        use rustls::pki_types::ServerName;
        use rustls::{ClientConfig, ClientConnection, ServerConnection, Stream};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut conn = ServerConnection::new(server_config).unwrap();
            let mut tls = Stream::new(&mut conn, &mut sock);
            // Drive the handshake to completion.
            std::io::Read::read(&mut tls, &mut [0u8; 8]).ok();
        });

        let client_config = Arc::new(
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
                .with_no_client_auth(),
        );
        let name = ServerName::try_from(sni.to_string()).unwrap();
        let mut conn = ClientConnection::new(client_config, name).unwrap();
        let mut sock = std::net::TcpStream::connect(addr).unwrap();
        let mut tls = Stream::new(&mut conn, &mut sock);
        std::io::Write::write_all(&mut tls, b"ping").unwrap();
        let leaf = conn
            .peer_certificates()
            .and_then(|chain| chain.first().cloned())
            .expect("server presented a certificate");
        server.join().unwrap();
        leaf
    }

    fn leaf_der(path: &Path) -> CertificateDer<'static> {
        load_certs(path).unwrap().into_iter().next().unwrap()
    }

    #[test]
    fn sni_resolver_selects_certificate_by_server_name() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (cert_a, key_a) =
            write_ephemeral_pair_with_san(dir.path(), "a", "DNS:a.test,DNS:www.a.test");
        let (cert_b, key_b) = write_ephemeral_pair_with_san(dir.path(), "b", "DNS:b.test");
        let (cert_w, key_w) = write_ephemeral_pair_with_san(dir.path(), "w", "DNS:*.wild.test");
        let certs = [
            SniCert {
                names: vec!["a.test".into(), "www.a.test".into()],
                cert_path: cert_a.clone(),
                key_path: key_a,
                ocsp_path: None,
            },
            SniCert {
                names: vec!["b.test".into()],
                cert_path: cert_b.clone(),
                key_path: key_b,
                ocsp_path: None,
            },
            SniCert {
                names: vec!["*.wild.test".into()],
                cert_path: cert_w.clone(),
                key_path: key_w,
                ocsp_path: None,
            },
        ];
        let config = Arc::new(
            load_rustls_config_multi(
                &certs,
                false,
                &TlsCryptoPolicy::default(),
                &[b"http/1.1"],
                None,
            )
            .expect("multi cert config"),
        );
        assert_eq!(
            handshake_leaf(Arc::clone(&config), "b.test"),
            leaf_der(&cert_b)
        );
        assert_eq!(
            handshake_leaf(Arc::clone(&config), "A.TEST"),
            leaf_der(&cert_a),
            "SNI match is case-insensitive"
        );
        assert_eq!(
            handshake_leaf(Arc::clone(&config), "www.a.test"),
            leaf_der(&cert_a)
        );
        // Wildcard server_name matches any subdomain (nginx suffix semantics).
        assert_eq!(
            handshake_leaf(Arc::clone(&config), "x.wild.test"),
            leaf_der(&cert_w)
        );
        assert_eq!(
            handshake_leaf(Arc::clone(&config), "a.b.wild.test"),
            leaf_der(&cert_w)
        );
        // Unknown SNI falls back to the first configured certificate.
        assert_eq!(
            handshake_leaf(Arc::clone(&config), "unknown.test"),
            leaf_der(&cert_a)
        );
    }

    /// Returns `true` if the handshake completes, `false` if it is aborted
    /// (e.g. strict SNI `unrecognized_name` alert).
    fn handshake_completes(server_config: Arc<ServerConfig>, sni: &str) -> bool {
        use rustls::pki_types::ServerName;
        use rustls::{ClientConfig, ClientConnection, ServerConnection, Stream};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                if let Ok(mut conn) = ServerConnection::new(server_config) {
                    let mut tls = Stream::new(&mut conn, &mut sock);
                    std::io::Read::read(&mut tls, &mut [0u8; 8]).ok();
                }
            }
        });
        let client_config = Arc::new(
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
                .with_no_client_auth(),
        );
        let name = ServerName::try_from(sni.to_string()).unwrap();
        let mut conn = ClientConnection::new(client_config, name).unwrap();
        let ok = std::net::TcpStream::connect(addr)
            .map(|mut sock| {
                let mut tls = Stream::new(&mut conn, &mut sock);
                std::io::Write::write_all(&mut tls, b"ping").is_ok()
            })
            .unwrap_or(false);
        server.join().unwrap();
        ok
    }

    #[test]
    fn strict_sni_aborts_unknown_names_and_keeps_known() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (cert_a, key_a) = write_ephemeral_pair_with_san(dir.path(), "a", "DNS:a.test");
        let (cert_b, key_b) = write_ephemeral_pair_with_san(dir.path(), "b", "DNS:b.test");
        let certs = [
            SniCert {
                names: vec!["a.test".into()],
                cert_path: cert_a,
                key_path: key_a,
                ocsp_path: None,
            },
            SniCert {
                names: vec!["b.test".into()],
                cert_path: cert_b,
                key_path: key_b,
                ocsp_path: None,
            },
        ];
        let strict = Arc::new(
            load_rustls_config_multi(
                &certs,
                true,
                &TlsCryptoPolicy::default(),
                &[b"http/1.1"],
                None,
            )
            .unwrap(),
        );
        assert!(handshake_completes(Arc::clone(&strict), "a.test"));
        assert!(handshake_completes(Arc::clone(&strict), "B.TEST"));
        assert!(
            !handshake_completes(Arc::clone(&strict), "unknown.test"),
            "strict_sni must abort handshakes with unmatched SNI"
        );
    }

    #[test]
    fn sni_multi_rejects_mismatched_cert_key_pair() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (_cert_a, key_a) = write_ephemeral_pair_with_san(dir.path(), "a", "DNS:a.test");
        let (cert_b, _key_b) = write_ephemeral_pair_with_san(dir.path(), "b", "DNS:b.test");
        let swapped = [SniCert {
            names: vec!["a.test".into()],
            cert_path: cert_b,
            key_path: key_a,
            ocsp_path: None,
        }];
        assert_eq!(
            load_rustls_config_multi(&swapped, false, &TlsCryptoPolicy::default(), &[b"h2"], None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn sni_multi_rejects_duplicate_names_and_empty_input() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (cert_a, key_a) =
            write_ephemeral_pair_with_san(dir.path(), "a", "DNS:a.test,DNS:www.a.test");
        let (cert_b, key_b) = write_ephemeral_pair_with_san(dir.path(), "b", "DNS:b.test");
        let dup = [
            SniCert {
                names: vec!["dup.test".into()],
                cert_path: cert_a,
                key_path: key_a,
                ocsp_path: None,
            },
            SniCert {
                names: vec!["DUP.test".into()],
                cert_path: cert_b,
                key_path: key_b,
                ocsp_path: None,
            },
        ];
        assert_eq!(
            load_rustls_config_multi(&dup, false, &TlsCryptoPolicy::default(), &[b"h2"], None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            load_rustls_config_multi(&[], false, &TlsCryptoPolicy::default(), &[b"h2"], None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn sni_multi_single_cert_matches_historical_path() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
        let single = [SniCert {
            names: Vec::new(),
            cert_path: cert,
            key_path: key,
            ocsp_path: None,
        }];
        let cfg =
            load_rustls_config_multi(&single, false, &TlsCryptoPolicy::default(), &[b"h2"], None)
                .unwrap();
        assert_eq!(cfg.alpn_protocols, [b"h2".to_vec()]);
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
        assert!(acceptor.snapshot().is_none());
        acceptor.clear();
        assert!(acceptor.snapshot().is_none());
    }

    /// NG-14: one client handshake against a fresh listener; returns the
    /// negotiated `HandshakeKind`. The server reads the client flight, then
    /// sends one app-data byte so the client fully processes post-handshake
    /// messages (TLS 1.3 NewSessionTicket).
    fn negotiated_kind(
        server_config: Arc<ServerConfig>,
        client_config: Arc<rustls::ClientConfig>,
        sni: &str,
    ) -> rustls::HandshakeKind {
        use rustls::pki_types::ServerName;
        use rustls::{ClientConnection, ServerConnection, Stream};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut conn = ServerConnection::new(server_config).unwrap();
            let mut tls = Stream::new(&mut conn, &mut sock);
            std::io::Read::read(&mut tls, &mut [0u8; 8]).ok();
            std::io::Write::write_all(&mut tls, b"p").ok();
        });
        let name = ServerName::try_from(sni.to_string()).unwrap();
        let mut conn = ClientConnection::new(client_config, name).unwrap();
        let mut sock = std::net::TcpStream::connect(addr).unwrap();
        let kind = {
            let mut tls = Stream::new(&mut conn, &mut sock);
            std::io::Write::write_all(&mut tls, b"ping").unwrap();
            std::io::Read::read(&mut tls, &mut [0u8; 1]).ok();
            conn.handshake_kind()
        };
        server.join().unwrap();
        kind.expect("handshake completed")
    }

    fn any_cert_client() -> Arc<rustls::ClientConfig> {
        use rustls::ClientConfig;
        Arc::new(
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
                .with_no_client_auth(),
        )
    }

    /// NG-14: stateful session IDs and stateless tickets both resume against a
    /// *second* ServerConfig built from the same TlsSessionCache — i.e.
    /// resumption survives acceptor rebuilds (reload).
    #[test]
    fn session_state_resumes_across_acceptor_rebuild() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
        let cache = TlsSessionCache::default();
        let settings = TlsSettings {
            cert_path: cert,
            key_path: key,
        };
        let build = || {
            Arc::new(
                load_rustls_config(&settings, &[b"http/1.1"], Some(&cache)).expect("server config"),
            )
        };
        let first = build();
        let reloaded = build(); // same cache, new config — stands in for a reload

        // TLS 1.3: resumption via the shared rotating ticketer.
        let client = any_cert_client();
        assert_eq!(
            negotiated_kind(Arc::clone(&first), Arc::clone(&client), "a.test"),
            rustls::HandshakeKind::Full
        );
        assert_eq!(
            negotiated_kind(Arc::clone(&reloaded), client, "a.test"),
            rustls::HandshakeKind::Resumed,
            "TLS 1.3 ticket resumption must survive acceptor rebuild"
        );

        // TLS 1.2: resumption via the shared session-ID store.
        let client12 = Arc::new(
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
                .with_no_client_auth(),
        );
        assert_eq!(
            negotiated_kind(Arc::clone(&first), Arc::clone(&client12), "a.test"),
            rustls::HandshakeKind::Full
        );
        assert_eq!(
            negotiated_kind(Arc::clone(&reloaded), client12, "a.test"),
            rustls::HandshakeKind::Resumed,
            "TLS 1.2 session-ID resumption must survive acceptor rebuild"
        );
    }

    /// NG-12: one handshake with an explicit client config; returns the
    /// negotiated `(version, cipher_suite)` or `None` when the handshake
    /// fails (e.g. version or suite filtered out by the server policy).
    fn negotiated_params(
        server_config: Arc<ServerConfig>,
        client_config: Arc<rustls::ClientConfig>,
        sni: &str,
    ) -> Option<(rustls::ProtocolVersion, CipherSuite)> {
        use rustls::pki_types::ServerName;
        use rustls::{ClientConnection, ServerConnection, Stream};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                if let Ok(mut conn) = ServerConnection::new(server_config) {
                    let mut tls = Stream::new(&mut conn, &mut sock);
                    std::io::Read::read(&mut tls, &mut [0u8; 8]).ok();
                }
            }
        });
        let name = ServerName::try_from(sni.to_string()).unwrap();
        let mut conn = ClientConnection::new(client_config, name).unwrap();
        let result = std::net::TcpStream::connect(addr)
            .ok()
            .and_then(|mut sock| {
                let mut tls = Stream::new(&mut conn, &mut sock);
                std::io::Write::write_all(&mut tls, b"ping").ok()?;
                Some((
                    conn.protocol_version()?,
                    conn.negotiated_cipher_suite()?.suite(),
                ))
            });
        server.join().unwrap();
        result
    }

    /// NG-12: `TlsCryptoPolicy` restricts the negotiated protocol version,
    /// cipher suite and key-exchange group; a TLS 1.2-only client is refused.
    #[test]
    fn ng12_crypto_policy_restricts_version_and_suite() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
        let certs = [SniCert {
            names: vec!["a.test".into()],
            cert_path: cert,
            key_path: key,
            ocsp_path: None,
        }];
        let crypto = TlsCryptoPolicy {
            protocols: vec!["TLSv1.3".into()],
            ciphers: vec!["TLS13_CHACHA20_POLY1305_SHA256".into()],
            curves: vec!["X25519".into()],
        };
        let config = Arc::new(
            load_rustls_config_multi(&certs, false, &crypto, &[b"http/1.1"], None)
                .expect("crypto policy config"),
        );

        let client12 = Arc::new(
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
                .with_no_client_auth(),
        );
        assert_eq!(
            negotiated_params(Arc::clone(&config), client12, "a.test"),
            None,
            "TLS 1.2 client must fail against a TLSv1.3-only policy"
        );

        assert_eq!(
            negotiated_params(Arc::clone(&config), any_cert_client(), "a.test"),
            Some((
                rustls::ProtocolVersion::TLSv1_3,
                CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
            )),
            "TLS 1.3 client must negotiate exactly the pinned suite"
        );
    }

    /// NG-13: a stapled DER OCSP response lands in the CertifiedKey and the
    /// handshake still completes; missing/empty files fail closed.
    #[test]
    fn ng13_ocsp_stapling_loads_and_fails_closed() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
        let ocsp = dir.path().join("ocsp.der");
        std::fs::write(&ocsp, b"\x30\x03\x0a\x01\x00").unwrap();
        let entry = SniCert {
            names: vec!["a.test".into()],
            cert_path: cert.clone(),
            key_path: key.clone(),
            ocsp_path: Some(ocsp.clone()),
        };
        let loaded = load_certified_key(0, &entry).expect("certified key with ocsp");
        assert_eq!(loaded.ocsp.as_deref(), Some(&b"\x30\x03\x0a\x01\x00"[..]));

        let certs = [entry];
        let config = Arc::new(
            load_rustls_config_multi(
                &certs,
                false,
                &TlsCryptoPolicy::default(),
                &[b"http/1.1"],
                None,
            )
            .unwrap(),
        );
        assert!(handshake_completes(config, "a.test"));

        let missing = SniCert {
            names: vec![],
            cert_path: cert.clone(),
            key_path: key.clone(),
            ocsp_path: Some(dir.path().join("gone.der")),
        };
        assert!(load_certified_key(0, &missing).is_err());
        let empty_path = dir.path().join("empty.der");
        std::fs::write(&empty_path, b"").unwrap();
        let empty = SniCert {
            ocsp_path: Some(empty_path),
            ..missing
        };
        assert_eq!(
            load_certified_key(0, &empty).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    /// AP-02/F1: a single cert with no `server_name` must not bypass strict
    /// SNI through the single-cert shortcut — the resolver path applies.
    #[test]
    fn strict_sni_single_unnamed_cert_rejects_all_sni() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
        let certs = [SniCert {
            names: vec![],
            cert_path: cert,
            key_path: key,
            ocsp_path: None,
        }];
        let strict = Arc::new(
            load_rustls_config_multi(
                &certs,
                true,
                &TlsCryptoPolicy::default(),
                &[b"http/1.1"],
                None,
            )
            .unwrap(),
        );
        assert!(
            !handshake_completes(strict, "anything.test"),
            "strict_sni with no configured names must reject every SNI"
        );
    }

    /// NG-12: unknown tokens and policies that leave no usable suite for the
    /// enabled versions fail closed at acceptor build time.
    #[test]
    fn ng12_crypto_policy_fails_closed() {
        install_rustls_provider();
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
        let certs = [SniCert {
            names: vec!["a.test".into()],
            cert_path: cert,
            key_path: key,
            ocsp_path: None,
        }];
        let build = |crypto: TlsCryptoPolicy| {
            load_rustls_config_multi(&certs, false, &crypto, &[b"http/1.1"], None)
        };
        for crypto in [
            TlsCryptoPolicy {
                protocols: vec!["TLSv1.1".into()],
                ..Default::default()
            },
            TlsCryptoPolicy {
                ciphers: vec!["DES-CBC3-SHA".into()],
                ..Default::default()
            },
            TlsCryptoPolicy {
                curves: vec!["secp521r1".into()],
                ..Default::default()
            },
            // TLS 1.3 only + a TLS 1.2-only suite: zero usable suites.
            TlsCryptoPolicy {
                protocols: vec!["TLSv1.3".into()],
                ciphers: vec!["ECDHE-RSA-AES256-GCM-SHA384".into()],
                ..Default::default()
            },
        ] {
            assert_eq!(
                build(crypto).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}
