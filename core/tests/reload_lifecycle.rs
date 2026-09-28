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

// Suite serialization uses a sync Mutex held across await; not product async code.
#![allow(clippy::await_holding_lock)]

use exyonq_core::kernel_control_port::CoreKernelControlPort;
use exyonq_core::lifecycle::LifecycleState;
use exyonq_core::reload::{
    active_runtime_generation, arm_reload_hold_for_tests, read_state,
    release_reload_hold_for_tests, reload_from_path, reload_in_progress,
    set_reload_fail_after_prepare_for_tests, wrap_state, ReloadDisposition,
};
use exyonq_core::server::state::ServerState;
use exyonq_mod_proxy::build_incoming_client;
use exyonq_mod_tls::{SharedTlsAcceptor, TlsSessionCache, TlsSettings};
use exyonq_module_api::kernel_control::KernelControlPort;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};
use tempfile::TempDir;

/// Process-wide reload counters/flags are shared; serialize this integration suite.
static RELOAD_LIFECYCLE_GATE: Mutex<()> = Mutex::new(());

fn lock_reload_suite() -> MutexGuard<'static, ()> {
    ensure_test_logging_reload_hook();
    RELOAD_LIFECYCLE_GATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Cap061: reload COMMIT calls `apply_logging_reload`; composition-root hook is
/// registered by `cli/exyonq`. Integration tests must install a sink-less hook so
/// KEEP_OLD fail-closed does not reject valid reloads.
fn ensure_test_logging_reload_hook() {
    use std::sync::Once;
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        exyonq_core::observability::register_logging_reload_hook(|logging| {
            exyonq_core::observability::set_access_logging_enabled(logging.access.enabled);
            exyonq_core::observability::set_audit_logging_enabled(logging.audit.enabled);
            exyonq_core::observability::set_otel_spans_enabled(logging.otel.enabled);
            Ok(())
        });
    });
}

fn reset_reload_test_hooks() {
    release_reload_hold_for_tests();
    set_reload_fail_after_prepare_for_tests(false);
}

fn write_valid_config(path: &std::path::Path) {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/static.toml");
    std::fs::write(path, std::fs::read_to_string(fixture).unwrap()).unwrap();
}

fn write_config_with_listen(path: &std::path::Path, listen: &str) {
    let body = format!(
        r#"config_version = 1

[[server]]
listen = "{listen}"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/site" }}
root = "tests/fixtures/www"
index = "index.html"
"#
    );
    std::fs::write(path, body).unwrap();
}

fn write_config_with_http3_listen(path: &std::path::Path, listen: &str, http3: &str) {
    let body = format!(
        r#"config_version = 1

[[server]]
listen = "{listen}"
http3_listen = "{http3}"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/site" }}
root = "tests/fixtures/www"
index = "index.html"
"#
    );
    std::fs::write(path, body).unwrap();
}

fn write_config_with_root(path: &std::path::Path, root: &str) {
    let body = format!(
        r#"config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{root}"
index = "index.html"
"#
    );
    std::fs::write(path, body).unwrap();
}

fn write_config_with_tls(path: &std::path::Path, cert: &std::path::Path, key: &std::path::Path) {
    let body = format!(
        r#"config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
tls = {{ cert = "{}", key = "{}" }}

[[route]]
name = "site"
match = {{ path = "/site" }}
root = "tests/fixtures/www"
index = "index.html"
"#,
        cert.display(),
        key.display()
    );
    std::fs::write(path, body).unwrap();
}

fn openssl_bin() -> &'static str {
    "openssl"
}

fn write_ephemeral_pkcs8_pair(dir: &std::path::Path) -> (PathBuf, PathBuf) {
    let cert = dir.join("cert.pem");
    let key = dir.join("key.pem");
    overwrite_ephemeral_pkcs8_pair(&cert, &key, "exyonq-cap013-reload-test");
    (cert, key)
}

fn overwrite_ephemeral_pkcs8_pair(
    cert: &std::path::Path,
    key: &std::path::Path,
    common_name: &str,
) {
    exyonq_mod_tls::install_rustls_provider();
    let status = Command::new(openssl_bin())
        .args(["req", "-x509", "-newkey", "rsa:2048", "-keyout"])
        .arg(key)
        .arg("-out")
        .arg(cert)
        .args(["-days", "1", "-nodes", "-subj"])
        .arg(format!("/CN={common_name}"))
        .status()
        .expect("openssl available for ephemeral TLS fixtures");
    assert!(status.success(), "openssl ephemeral cert generation failed");
}

#[tokio::test]
async fn valid_reload_increments_generation() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let gen_before = read_state(&shared).generation;
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();

    write_config_with_root(&path, "tests/fixtures/www-cap013-b");

    let result = reload_from_path(&path, &shared, proxy, &tls, &cache)
        .await
        .unwrap();
    assert_eq!(
        result.disposition,
        ReloadDisposition::RuntimeGenerationPublished
    );
    assert!(format!("{result:?}").contains("RuntimeGenerationPublished"));
    assert!(read_state(&shared).generation > gen_before);
}

#[tokio::test]
async fn invalid_reload_keeps_previous_generation() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let gen_before = read_state(&shared).generation;
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();

    std::fs::write(&path, "not valid toml [[[").unwrap();
    assert!(reload_from_path(&path, &shared, proxy, &tls, &cache)
        .await
        .is_err());
    assert_eq!(read_state(&shared).generation, gen_before);
}

#[tokio::test]
async fn late_fail_after_prepare_keeps_generation_tls_and_snapshot() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let gen_before = read_state(&shared).generation;
    let fp_before = read_state(&shared).fingerprint().to_string();
    let live_gen_before = active_runtime_generation();

    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();
    // Seed live TLS when the host can mint a rustls-loadable ephemeral pair.
    // Generation/fingerprint keep-last is the hard Cap013 invariant; TLS
    // non-mutation follows from commit-after-prepare (install only on success).
    let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
    let seeded_tls = tls
        .load(
            &TlsSettings {
                cert_path: cert,
                key_path: key,
            },
            &cache,
            &[b"h2", b"http/1.1"],
        )
        .is_ok()
        && tls.is_loaded();

    // Distinct fingerprint so prepare proceeds past NO_OP, then inject late failure.
    write_config_with_root(&path, "tests/fixtures/www-cap013-late-fail");
    set_reload_fail_after_prepare_for_tests(true);
    let err = reload_from_path(&path, &shared, proxy, &tls, &cache)
        .await
        .expect_err("late prepare failure must reject");
    assert!(
        err.to_string().contains("EXY-RELOAD-0004"),
        "unexpected error: {err}"
    );

    assert_eq!(read_state(&shared).generation, gen_before);
    assert_eq!(read_state(&shared).fingerprint(), fp_before);
    assert_eq!(active_runtime_generation(), live_gen_before);
    if seeded_tls {
        assert!(
            tls.is_loaded(),
            "live TLS must not be cleared/replaced on prepare failure"
        );
    }
    assert!(!reload_in_progress());
}

#[tokio::test]
async fn identical_fingerprint_is_noop_without_generation_bump() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let gen_before = read_state(&shared).generation;
    let fp_before = read_state(&shared).fingerprint().to_string();
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();

    let result = reload_from_path(&path, &shared, proxy, &tls, &cache)
        .await
        .unwrap();
    assert_eq!(result.disposition, ReloadDisposition::NoOp);
    assert_eq!(result.state.generation, gen_before);
    assert_eq!(read_state(&shared).generation, gen_before);
    assert_eq!(read_state(&shared).fingerprint(), fp_before);
    assert_eq!(active_runtime_generation(), gen_before);

    let port = CoreKernelControlPort {
        shared: Arc::clone(&shared),
        proxy_client: proxy.clone(),
        tls_acceptor: tls,
        tls_session_cache: cache,
        lifecycle: LifecycleState::new(),
    };
    let outcome = port.request_reload(&path).await;
    assert!(outcome.ok);
    assert_eq!(outcome.code.as_deref(), Some("EXY-RELOAD-0008"));
    assert_eq!(outcome.snapshot.generation, gen_before);
    assert!(outcome.code_result_consistent());
}

#[tokio::test]
async fn identical_tls_config_publishes_material_without_runtime_generation() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
    write_config_with_tls(&path, &cert, &key);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let state_before = read_state(&shared);
    let generation_before = state_before.generation;
    let fingerprint_before = state_before.fingerprint().to_string();
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();
    tls.load(
        &TlsSettings {
            cert_path: cert.clone(),
            key_path: key.clone(),
        },
        &cache,
        &[b"h2", b"http/1.1"],
    )
    .unwrap();
    let cache_before = cache.as_arc();
    let publication_before = tls.snapshot();

    overwrite_ephemeral_pkcs8_pair(&cert, &key, "rotated.exyonq.test");
    let result = reload_from_path(&path, &shared, proxy, &tls, &cache)
        .await
        .unwrap();

    assert_eq!(
        result.disposition,
        ReloadDisposition::TlsMaterialPublishedTcp
    );
    assert!(Arc::ptr_eq(&result.state, &state_before));
    assert!(Arc::ptr_eq(&read_state(&shared), &state_before));
    assert_eq!(result.state.generation, generation_before);
    assert_eq!(result.state.fingerprint(), fingerprint_before);
    assert_eq!(active_runtime_generation(), generation_before);
    assert!(tls.is_loaded());
    assert!(!Arc::ptr_eq(&cache_before, &cache.as_arc()));
    let publication_after = tls.snapshot();
    assert_eq!(
        publication_after.publication_generation(),
        publication_before.publication_generation() + 1
    );
    assert_eq!(
        publication_after.session_storage_identity(),
        Some(cache.session_storage_identity())
    );

    let port = CoreKernelControlPort {
        shared,
        proxy_client: proxy.clone(),
        tls_acceptor: tls,
        tls_session_cache: cache,
        lifecycle: LifecycleState::new(),
    };
    let outcome = port.request_reload(&path).await;
    assert!(outcome.ok);
    assert_eq!(outcome.code, None);
    assert!(outcome
        .error
        .as_deref()
        .is_some_and(|note| note.contains("TCP TLS") && note.contains("HTTP/3")));
    assert_eq!(outcome.snapshot.generation, generation_before);
    assert!(outcome.code_result_consistent());
}

#[tokio::test]
async fn invalid_same_path_tls_material_keeps_state_acceptor_and_session_epoch() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    let (cert, key) = write_ephemeral_pkcs8_pair(dir.path());
    write_config_with_tls(&path, &cert, &key);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let state_before = read_state(&shared);
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();
    tls.load(
        &TlsSettings {
            cert_path: cert.clone(),
            key_path: key.clone(),
        },
        &cache,
        &[b"h2", b"http/1.1"],
    )
    .unwrap();
    let cache_before = cache.as_arc();
    let publication_before = tls.snapshot();

    std::fs::write(&cert, "not a certificate").unwrap();
    let err = reload_from_path(&path, &shared, proxy, &tls, &cache)
        .await
        .expect_err("invalid same-path PEM must fail closed");

    assert!(
        err.to_string().starts_with("EXY-RELOAD-0004:"),
        "unexpected error: {err}"
    );
    assert!(Arc::ptr_eq(&read_state(&shared), &state_before));
    assert!(tls.is_loaded());
    assert!(Arc::ptr_eq(&cache_before, &cache.as_arc()));
    let publication_after = tls.snapshot();
    assert_eq!(
        publication_after.publication_generation(),
        publication_before.publication_generation()
    );
    assert_eq!(
        publication_after.session_storage_identity(),
        publication_before.session_storage_identity()
    );
}

#[tokio::test]
async fn listen_bind_change_rejects_with_exy_reload_0005() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_config_with_listen(&path, "127.0.0.1:8080");

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let gen_before = read_state(&shared).generation;
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();

    write_config_with_listen(&path, "127.0.0.1:9090");
    let err = reload_from_path(&path, &shared, proxy, &tls, &cache)
        .await
        .expect_err("listen change must reject");
    assert!(
        err.to_string().contains("EXY-RELOAD-0005"),
        "unexpected error: {err}"
    );
    assert_eq!(read_state(&shared).generation, gen_before);

    let port = CoreKernelControlPort {
        shared,
        proxy_client: proxy.clone(),
        tls_acceptor: tls,
        tls_session_cache: cache,
        lifecycle: LifecycleState::new(),
    };
    write_config_with_listen(&path, "127.0.0.1:9091");
    let outcome = port.request_reload(&path).await;
    assert!(!outcome.ok);
    assert_eq!(outcome.code.as_deref(), Some("EXY-RELOAD-0005"));
    assert!(outcome.code_result_consistent());
}

#[tokio::test]
async fn http3_listen_bind_change_rejects_with_exy_reload_0005() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_config_with_http3_listen(&path, "127.0.0.1:8080", "127.0.0.1:8443");

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let gen_before = read_state(&shared).generation;
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();

    write_config_with_http3_listen(&path, "127.0.0.1:8080", "127.0.0.1:9443");
    let err = reload_from_path(&path, &shared, proxy, &tls, &cache)
        .await
        .expect_err("http3_listen change must reject");
    assert!(
        err.to_string().contains("EXY-RELOAD-0005"),
        "unexpected error: {err}"
    );
    assert_eq!(read_state(&shared).generation, gen_before);
}

#[tokio::test]
async fn concurrent_reloads_serialize_single_generation_step() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let gen_before = read_state(&shared).generation;
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();

    write_config_with_root(&path, "tests/fixtures/www-cap013-concurrent");

    let shared_a = Arc::clone(&shared);
    let shared_b = Arc::clone(&shared);
    let proxy_a = proxy.clone();
    let proxy_b = proxy.clone();
    let tls_a = tls.clone();
    let tls_b = tls.clone();
    let cache_a = cache.clone();
    let cache_b = cache.clone();
    let path_a = path.clone();
    let path_b = path.clone();

    let (r1, r2) = tokio::join!(
        async move { reload_from_path(&path_a, &shared_a, &proxy_a, &tls_a, &cache_a).await },
        async move { reload_from_path(&path_b, &shared_b, &proxy_b, &tls_b, &cache_b).await },
    );
    let r1 = r1.expect("first reload");
    let r2 = r2.expect("second reload");
    // Serialized: exactly one apply, one identical NO_OP.
    assert_eq!(
        usize::from(r1.disposition == ReloadDisposition::NoOp)
            + usize::from(r2.disposition == ReloadDisposition::NoOp),
        1
    );
    assert_eq!(read_state(&shared).generation, gen_before + 1);
    assert!(!reload_in_progress());
}

#[tokio::test]
async fn reload_in_progress_visible_while_mutex_held() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);

    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let tls = SharedTlsAcceptor::new();
    let cache = TlsSessionCache::default();
    let lifecycle = LifecycleState::new();
    let port = CoreKernelControlPort {
        shared: Arc::clone(&shared),
        proxy_client: proxy.clone(),
        tls_acceptor: tls.clone(),
        tls_session_cache: cache.clone(),
        lifecycle,
    };

    write_config_with_root(&path, "tests/fixtures/www-cap013-hold");
    arm_reload_hold_for_tests();

    let path_bg = path.clone();
    let shared_bg = Arc::clone(&shared);
    let proxy_bg = proxy.clone();
    let tls_bg = tls.clone();
    let cache_bg = cache.clone();
    let handle = tokio::spawn(async move {
        reload_from_path(&path_bg, &shared_bg, &proxy_bg, &tls_bg, &cache_bg).await
    });

    // Wait until the reload critical section is entered.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while !reload_in_progress() {
        if tokio::time::Instant::now() > deadline {
            release_reload_hold_for_tests();
            panic!("reload_in_progress never became true");
        }
        tokio::task::yield_now().await;
    }
    assert!(port.read_status(false).snapshot.reload_in_progress);

    release_reload_hold_for_tests();
    let result = handle.await.unwrap().unwrap();
    assert_eq!(
        result.disposition,
        ReloadDisposition::RuntimeGenerationPublished
    );
    assert!(!reload_in_progress());
    assert!(!port.read_status(false).snapshot.reload_in_progress);
}

#[tokio::test]
async fn lifecycle_wait_for_shutdown_unblocks_after_request() {
    let ops = LifecycleState::new();
    let ops_wait = ops.clone();
    let handle = tokio::spawn(async move {
        LifecycleState::wait_for_shutdown(&ops_wait).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(!handle.is_finished());
    ops.request_shutdown();
    tokio::time::timeout(std::time::Duration::from_secs(1), handle)
        .await
        .expect("wait_for_shutdown should complete")
        .unwrap();
}

#[tokio::test]
async fn kernel_port_rejects_reload_after_shutdown() {
    let _gate = lock_reload_suite();
    reset_reload_test_hooks();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exyonq.toml");
    write_valid_config(&path);
    let config = exyonq_runtime_plan::load_config_for_reload(&path).unwrap();
    let proxy = build_incoming_client();
    let state = ServerState::new(config, proxy.clone()).await.unwrap();
    let shared = wrap_state(state);
    let ops = LifecycleState::new();
    ops.request_shutdown();
    let port = CoreKernelControlPort {
        shared,
        proxy_client: proxy.clone(),
        tls_acceptor: SharedTlsAcceptor::new(),
        tls_session_cache: TlsSessionCache::default(),
        lifecycle: ops,
    };
    let outcome = port.request_reload(&path).await;
    assert!(!outcome.ok);
}
