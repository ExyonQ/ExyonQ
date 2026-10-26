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

//! ACME bootstrap-from-zero: `prepare_tls_acceptor` must not hard-fail when
//! `acme.enabled` and the cert/key pair is not on disk yet — the listener
//! serves cleartext (HTTP-01) until the bootstrap publishes the issued
//! certificate. Without ACME, a missing pair stays fail-closed.

use exyonq_core::config::AppConfig;
use exyonq_core::reload::prepare_tls_acceptor;
use exyonq_mod_tls::TlsSessionCache;
use tempfile::TempDir;

fn config_with_tls(cert: &str, key: &str, acme: &str) -> AppConfig {
    format!(
        r#"config_version = 1

[[server]]
listen = "127.0.0.1:443"
routes = ["site"]

[server.tls]
cert = "{cert}"
key = "{key}"
{acme}
[[route]]
name = "site"
match = {{ path = "/" }}
root = "tests/fixtures/www"
index = "index.html"
"#
    )
    .parse()
    .expect("config parses")
}

const ACME_ENABLED: &str = r#"
[server.tls.acme]
email = "ops@example.test"
domains = ["example.test"]
"#;

#[test]
fn acme_enabled_missing_pair_returns_none_acceptor() {
    let dir = TempDir::new().unwrap();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    let config = config_with_tls(
        &cert.display().to_string(),
        &key.display().to_string(),
        ACME_ENABLED,
    );
    let acceptor = prepare_tls_acceptor(&config, &TlsSessionCache::default())
        .expect("pending ACME pair must not fail prepare");
    assert!(
        acceptor.is_none(),
        "no acceptor until the ACME-issued pair exists"
    );
}

#[test]
fn missing_pair_without_acme_still_fails_closed() {
    let dir = TempDir::new().unwrap();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    let config = config_with_tls(&cert.display().to_string(), &key.display().to_string(), "");
    assert!(
        prepare_tls_acceptor(&config, &TlsSessionCache::default()).is_err(),
        "missing cert without ACME must stay fail-closed"
    );
}

#[test]
fn acme_enabled_existing_pair_builds_acceptor() {
    let dir = TempDir::new().unwrap();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    exyonq_mod_tls::install_rustls_provider();
    let status = std::process::Command::new("openssl")
        .args(["req", "-x509", "-newkey", "rsa:2048", "-keyout"])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .args(["-days", "1", "-nodes", "-subj", "/CN=acme-prepare-test"])
        .status()
        .expect("openssl available for ephemeral TLS fixtures");
    assert!(status.success());
    let config = config_with_tls(
        &cert.display().to_string(),
        &key.display().to_string(),
        ACME_ENABLED,
    );
    let acceptor = prepare_tls_acceptor(&config, &TlsSessionCache::default())
        .expect("issued pair builds acceptor");
    assert!(acceptor.is_some());
}
