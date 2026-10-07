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
//! Hot reload: generation swap and config reload mechanism (watcher lives in reload-runtime).
//!
//! Cap013 contract:
//! - prepare (parse / TLS offline / ServerState) completes before any live publish
//! - process-wide async mutex serializes structural reload; `reload_in_progress` while held
//! - identical plan fingerprint without TLS → NO_OP (`EXY-RELOAD-0008`)
//! - identical plan fingerprint with TLS → TCP H1/H2 material refresh without generation bump
//! - listen / http3_listen bind changes → reject (`EXY-RELOAD-0005`)
//!
//! TCP TLS publication does not rotate the separately owned HTTP/3 certificate.
//! H3 certificate rotation remains a known blocker and must not be inferred from
//! [`ReloadDisposition::TlsMaterialPublishedTcp`].

use crate::config::AppConfig;
use crate::discovery_overlay;
use crate::server::state::ServerState;
use crate::tls::{SharedTlsAcceptor, TlsSessionCache, TlsSettings};
use exyonq_mod_proxy::ProxyClient;
use exyonq_mod_tls::{PreparedTlsReload, TlsAcceptor};
use exyonq_runtime_plan::{compile_runtime_plan, load_config_for_reload};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::Mutex;

pub type SharedServerState = Arc<RwLock<Arc<ServerState>>>;

static RELOAD_GENERATION: AtomicU64 = AtomicU64::new(1);
static RELOAD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
/// Generation-scoped Cap067/ARCH-002: whether live state requires WAF wire materialization.
/// Published with [`publish_runtime_generation`] — Cap067 may skip `read_state` when false.
static WAF_WIRE_INSPECTION_ACTIVE: AtomicBool = AtomicBool::new(false);
static RELOAD_MUTEX: Mutex<()> = Mutex::const_new(());

/// When set, the next reload fails after offline prepare and before publish (test only).
static FAIL_AFTER_PREPARE_FOR_TESTS: AtomicBool = AtomicBool::new(false);

/// When set, reload spins inside the critical section until released (test only).
static RELOAD_TEST_HOLD_ARMED: AtomicBool = AtomicBool::new(false);

/// Process-visible runtime generation (advanced on successful reload publish).
pub fn active_runtime_generation() -> u64 {
    RELOAD_GENERATION.load(Ordering::SeqCst)
}

/// True while a structural reload holds the process-wide reload mutex (PREPARE/COMMIT).
pub fn reload_in_progress() -> bool {
    RELOAD_IN_PROGRESS.load(Ordering::Acquire)
}

/// Publish the live generation observed by in-flight FPC fill / reload consumers.
pub fn publish_runtime_generation(generation: u64) {
    RELOAD_GENERATION.store(generation, Ordering::SeqCst);
}

/// Publish whether Cap067 must materialize WAF wire work for the live generation.
pub fn publish_waf_wire_inspection_active(active: bool) {
    WAF_WIRE_INSPECTION_ACTIVE.store(active, Ordering::Release);
}

/// Process-visible ARCH-002 gate (Acquire). Prefer over `read_state` when only the flag is needed.
pub fn waf_wire_inspection_active_published() -> bool {
    WAF_WIRE_INSPECTION_ACTIVE.load(Ordering::Acquire)
}

/// Publish generation + Cap067 WAF wire-inspection flag from a newly live [`ServerState`].
pub fn publish_runtime_observe_authority(state: &ServerState) {
    publish_runtime_generation(state.generation);
    publish_waf_wire_inspection_active(state.waf_wire_inspection_active);
    #[cfg(target_os = "linux")]
    crate::server::sdp_start::publish_projection(state);
}

/// Force the next reload to fail after prepare, before TLS/generation/shared publish.
pub fn set_reload_fail_after_prepare_for_tests(fail: bool) {
    FAIL_AFTER_PREPARE_FOR_TESTS.store(fail, Ordering::SeqCst);
}

/// Arm a hold inside the reload critical section (after `reload_in_progress=true`).
/// Call [`release_reload_hold_for_tests`] from another task to continue.
pub fn arm_reload_hold_for_tests() {
    RELOAD_TEST_HOLD_ARMED.store(true, Ordering::SeqCst);
}

/// Release a hold armed via [`arm_reload_hold_for_tests`].
pub fn release_reload_hold_for_tests() {
    RELOAD_TEST_HOLD_ARMED.store(false, Ordering::SeqCst);
}

pub fn wrap_state(state: Arc<ServerState>) -> SharedServerState {
    publish_runtime_observe_authority(&state);
    crate::waf::commit_waf_reload(state.generation);
    Arc::new(RwLock::new(state))
}

pub fn read_state(shared: &SharedServerState) -> Arc<ServerState> {
    Arc::clone(&*shared.read().expect("server state lock poisoned"))
}

/// Successful reload publication disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadDisposition {
    /// Compiled plan and effective non-file material were unchanged.
    NoOp,
    /// TCP TLS H1/H2 material and a fresh session epoch were published only.
    TlsMaterialPublishedTcp,
    /// A new runtime generation was published.
    RuntimeGenerationPublished,
}

/// Result of a live reload attempt.
#[derive(Clone)]
pub struct ReloadResult {
    pub state: Arc<ServerState>,
    pub disposition: ReloadDisposition,
}

impl std::fmt::Debug for ReloadResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReloadResult")
            .field("generation", &self.state.generation)
            .field("disposition", &self.disposition)
            .finish()
    }
}

struct PreparedTcpTls {
    publication: PreparedTlsReload,
}

impl PreparedTcpTls {
    fn publish(
        self,
        tls_acceptor: &SharedTlsAcceptor,
        tls_session_cache: &TlsSessionCache,
    ) -> anyhow::Result<()> {
        tls_acceptor
            .install_reload(self.publication, tls_session_cache)
            .map(|_| ())
            .map_err(|err| anyhow::anyhow!("TCP TLS publication rejected (KEEP_OLD): {err}"))
    }
}

struct ReloadInProgressGuard;

impl Drop for ReloadInProgressGuard {
    fn drop(&mut self) {
        RELOAD_IN_PROGRESS.store(false, Ordering::Release);
    }
}

/// Apply config from disk and swap the live snapshot (control socket / KernelControlPort).
pub async fn reload_from_path(
    path: &Path,
    shared: &SharedServerState,
    proxy_client: &ProxyClient,
    tls_acceptor: &SharedTlsAcceptor,
    tls_session_cache: &TlsSessionCache,
) -> anyhow::Result<ReloadResult> {
    let _mutex = RELOAD_MUTEX.lock().await;
    RELOAD_IN_PROGRESS.store(true, Ordering::Release);
    let _in_progress = ReloadInProgressGuard;

    while RELOAD_TEST_HOLD_ARMED.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let current = read_state(shared);
    let mut config = load_config_for_reload(path)?;
    config = discovery_overlay::apply_env_discovery_overlay(config);

    reject_listen_bind_changes(&current.config, &config)?;

    // Fingerprint compare uses compiled plan content (generation-independent).
    let candidate_plan = compile_runtime_plan(current.generation, config.clone())?;
    if candidate_plan.fingerprint.as_str() == current.fingerprint() {
        if config.primary_server().tls.is_some() {
            let prepared_tls = prepare_tcp_tls(&config, tls_acceptor, tls_session_cache, true)?;
            prepared_tls.publish(tls_acceptor, tls_session_cache)?;
            return Ok(ReloadResult {
                state: current,
                disposition: ReloadDisposition::TlsMaterialPublishedTcp,
            });
        }
        return Ok(ReloadResult {
            state: current,
            disposition: ReloadDisposition::NoOp,
        });
    }

    // Offline TLS prepare — does not mutate the live acceptor.
    // A TLS config/path change gets a fresh epoch. Unchanged TLS config keeps the
    // current epoch during an unrelated runtime-generation publication.
    let tls_config_changed = current.config.primary_server().tls != config.primary_server().tls;
    let prepared_tls =
        prepare_tcp_tls(&config, tls_acceptor, tls_session_cache, tls_config_changed)?;

    let next_generation = current.generation.saturating_add(1);

    // Cap013 / Cap015: stage WAF before ServerState construction so the new generation
    // freezes the candidate binding (not the previous generation's engine/enforce/abuse).
    if let Err(err) = crate::waf::prepare_waf_reload(&config) {
        crate::waf::discard_waf_reload();
        return Err(anyhow::anyhow!("waf reload prepare failed: {err}"));
    }

    let state = match ServerState::new_with_generation_unpublished(
        next_generation,
        config,
        proxy_client.clone(),
    )
    .await
    {
        Ok(state) => state,
        Err(err) => {
            crate::waf::discard_waf_reload();
            return Err(err);
        }
    };

    if FAIL_AFTER_PREPARE_FOR_TESTS.swap(false, Ordering::SeqCst) {
        crate::waf::discard_waf_reload();
        return Err(anyhow::anyhow!(
            "EXY-RELOAD-0004: prepare failed before publish"
        ));
    }

    // Cap061: commit observability against candidate config BEFORE runtime publish.
    // On failure KEEP_OLD sinks and do not publish the new generation.
    if let Err(err) = crate::observability::apply_logging_reload(&state.config.logging) {
        crate::waf::discard_waf_reload();
        crate::observability::emit_audit(crate::observability::AuditEvent {
            action: "reload.observability",
            result: "failure",
            detail: Some(&err),
            request_id: None,
        });
        return Err(anyhow::anyhow!(
            "EXY-RELOAD-OBSERVABILITY: observability prepare/commit failed (KEEP_OLD): {err}"
        ));
    }

    // COMMIT — Cap048: bind module slots here (not during prepare). On bind failure,
    // restore the previous generation's slots and discard staged WAF.
    if let Err(err) = state.publish_compiled_module_slots() {
        let _ = current.publish_compiled_module_slots();
        crate::waf::discard_waf_reload();
        // LA-CAP061-002: never swallow observability rollback — KEEP_OLD must be honest.
        if let Err(o11y_err) = crate::observability::apply_logging_reload(&current.config.logging) {
            crate::observability::emit_audit(crate::observability::AuditEvent {
                action: "reload.observability_rollback",
                result: "failure",
                detail: Some(&o11y_err),
                request_id: None,
            });
            return Err(anyhow::anyhow!(
                "EXY-RELOAD-OBSERVABILITY-ROLLBACK: publish failed ({err}); \
                 observability restore also failed ({o11y_err})"
            ));
        }
        return Err(err);
    }
    if let Err(tls_err) = prepared_tls.publish(tls_acceptor, tls_session_cache) {
        let slot_restore = current.publish_compiled_module_slots();
        crate::waf::discard_waf_reload();
        let observability_restore =
            crate::observability::apply_logging_reload(&current.config.logging);
        if let Err(restore_err) = slot_restore {
            return Err(anyhow::anyhow!(
                "{tls_err}; compiled module slot restore also failed: {restore_err}"
            ));
        }
        if let Err(restore_err) = observability_restore {
            return Err(anyhow::anyhow!(
                "{tls_err}; observability restore also failed: {restore_err}"
            ));
        }
        return Err(tls_err);
    }
    // Cap067 LA-CAP067-R3-001: advertise WAF-on before shared becomes WAF-capable so Cap067
    // keepalive cannot early-exit skip while live ServerState already requires inspect.
    if state.waf_wire_inspection_active && !current.waf_wire_inspection_active {
        publish_waf_wire_inspection_active(true);
    }
    // Shared snapshot before generation counters so status/request path agree first.
    *shared.write().expect("server state lock poisoned") = Arc::clone(&state);
    publish_runtime_observe_authority(&state);
    crate::waf::commit_waf_reload(state.generation);
    exyonq_cache::global_response_cache()
        .invalidate_runtime_generation(state.generation.saturating_sub(1));

    Ok(ReloadResult {
        state,
        disposition: ReloadDisposition::RuntimeGenerationPublished,
    })
}

/// Standalone prepare+publish without a shared Arc swap (used by tests / callers that
/// only need the new state). Still serializes via the process-wide reload mutex.
pub async fn reload_from_path_standalone(
    path: &Path,
    proxy_client: &ProxyClient,
    tls_acceptor: &SharedTlsAcceptor,
    tls_session_cache: &TlsSessionCache,
) -> anyhow::Result<Arc<ServerState>> {
    // Build a throwaway shared wrapper from a minimal placeholder — callers that need
    // fingerprint / listen checks against live state must use `reload_from_path`.
    // Preserve prior standalone semantics for ACME-adjacent tooling: prepare+publish TLS
    // and generation when building a new state without an existing SharedServerState.
    let _mutex = RELOAD_MUTEX.lock().await;
    RELOAD_IN_PROGRESS.store(true, Ordering::Release);
    let _in_progress = ReloadInProgressGuard;

    let mut config = load_config_for_reload(path)?;
    config = discovery_overlay::apply_env_discovery_overlay(config);
    let prepared_tls = prepare_tcp_tls(&config, tls_acceptor, tls_session_cache, true)?;
    let next_generation = active_runtime_generation().saturating_add(1);

    if let Err(err) = crate::waf::prepare_waf_reload(&config) {
        crate::waf::discard_waf_reload();
        return Err(anyhow::anyhow!("waf reload prepare failed: {err}"));
    }

    let state = match ServerState::new_with_generation_unpublished(
        next_generation,
        config,
        proxy_client.clone(),
    )
    .await
    {
        Ok(state) => state,
        Err(err) => {
            crate::waf::discard_waf_reload();
            return Err(err);
        }
    };

    if FAIL_AFTER_PREPARE_FOR_TESTS.swap(false, Ordering::SeqCst) {
        crate::waf::discard_waf_reload();
        return Err(anyhow::anyhow!(
            "EXY-RELOAD-0004: prepare failed before publish"
        ));
    }

    if let Err(err) = state.publish_compiled_module_slots() {
        crate::waf::discard_waf_reload();
        return Err(err);
    }
    prepared_tls.publish(tls_acceptor, tls_session_cache)?;
    publish_runtime_observe_authority(&state);
    crate::waf::commit_waf_reload(state.generation);
    exyonq_cache::global_response_cache()
        .invalidate_runtime_generation(state.generation.saturating_sub(1));
    Ok(state)
}

fn reject_listen_bind_changes(current: &AppConfig, candidate: &AppConfig) -> anyhow::Result<()> {
    if current.servers.len() != candidate.servers.len() {
        return Err(anyhow::anyhow!(
            "EXY-RELOAD-0005: listen/http3 bind change requires restart (server count changed)"
        ));
    }
    for (idx, (cur, cand)) in current
        .servers
        .iter()
        .zip(candidate.servers.iter())
        .enumerate()
    {
        if cur.listen != cand.listen {
            return Err(anyhow::anyhow!(
                "EXY-RELOAD-0005: listen bind change requires restart (server[{idx}] listen {} → {})",
                cur.listen,
                cand.listen
            ));
        }
        if cur.http3_listen != cand.http3_listen {
            return Err(anyhow::anyhow!(
                "EXY-RELOAD-0005: http3_listen bind change requires restart (server[{idx}])"
            ));
        }
    }
    Ok(())
}

/// Prepare a TLS acceptor offline (or `None` to clear on commit).
pub fn prepare_tls_acceptor(
    config: &AppConfig,
    tls_session_cache: &TlsSessionCache,
) -> anyhow::Result<Option<TlsAcceptor>> {
    if let Some(tls) = &config.primary_server().tls {
        let acceptor = SharedTlsAcceptor::prepare(
            &TlsSettings {
                cert_path: tls.cert.clone(),
                key_path: tls.key.clone(),
            },
            tls_session_cache,
            &[b"h2", b"http/1.1"],
        )
        .map_err(|err| {
            anyhow::anyhow!("EXY-RELOAD-0004: TCP TLS prepare failed (KEEP_OLD): {err}")
        })?;
        Ok(Some(acceptor))
    } else {
        Ok(None)
    }
}

fn tls_identities_on_primary_listen(config: &AppConfig) -> Vec<exyonq_mod_tls::TlsSniCertificate> {
    let listen = config.primary_server().listen.as_str();
    config
        .servers
        .iter()
        .filter(|server| server.listen == listen)
        .filter_map(|server| {
            let tls = server.tls.as_ref()?;
            Some(exyonq_mod_tls::TlsSniCertificate {
                names: server.server_name_list(),
                settings: TlsSettings {
                    cert_path: tls.cert.clone(),
                    key_path: tls.key.clone(),
                },
            })
        })
        .collect()
}

fn prepare_tcp_tls(
    config: &AppConfig,
    tls_acceptor: &SharedTlsAcceptor,
    tls_session_cache: &TlsSessionCache,
    fresh_epoch: bool,
) -> anyhow::Result<PreparedTcpTls> {
    let identities = tls_identities_on_primary_listen(config);
    if identities.len() > 1 {
        let acceptor = tls_acceptor
            .prepare_sni_reload(&identities, tls_session_cache, &[b"h2", b"http/1.1"], fresh_epoch)
            .map_err(|err| {
                anyhow::anyhow!("EXY-RELOAD-0004: TCP TLS prepare failed (KEEP_OLD): {err}")
            })?;
        return Ok(PreparedTcpTls {
            publication: acceptor,
        });
    }
    if let Some(identity) = identities.first() {
        let acceptor = tls_acceptor
            .prepare_reload(
                &identity.settings,
                tls_session_cache,
                &[b"h2", b"http/1.1"],
                fresh_epoch,
            )
            .map_err(|err| {
                anyhow::anyhow!("EXY-RELOAD-0004: TCP TLS prepare failed (KEEP_OLD): {err}")
            })?;
        Ok(PreparedTcpTls {
            publication: acceptor,
        })
    } else {
        let publication = tls_acceptor
            .prepare_clear(tls_session_cache, fresh_epoch)
            .map_err(|err| {
                anyhow::anyhow!("EXY-RELOAD-0004: TCP TLS clear prepare failed (KEEP_OLD): {err}")
            })?;
        Ok(PreparedTcpTls { publication })
    }
}

/// Immediate TCP TLS H1/H2 publish (ACME / certificate publication).
///
/// This does not rotate HTTP/3 certificate material.
pub fn reload_tls_acceptor(
    config: &AppConfig,
    tls_acceptor: &SharedTlsAcceptor,
    tls_session_cache: &TlsSessionCache,
) -> anyhow::Result<()> {
    let prepared = prepare_tcp_tls(config, tls_acceptor, tls_session_cache, true)?;
    prepared.publish(tls_acceptor, tls_session_cache)
}

// Config load lives in `exyonq-runtime-plan` (KD4.6: compiler boundary).
