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
//! Kernel control port — executes reload/drain/shutdown; ops runtime owns command surface.

use crate::lifecycle::LifecycleState;
use crate::reload::{read_state, reload_from_path, reload_in_progress, SharedServerState};
use crate::tls::{SharedTlsAcceptor, TlsSessionCache};
use crate::VERSION;
use async_trait::async_trait;
use exyonq_mod_proxy::ProxyClient;
use exyonq_module_api::kernel_control::{
    KernelControlPort, KernelStatusSnapshot, OpsCommand, OpsCommandOutcome,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

static STARTED_AT: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

pub fn mark_process_started() {
    let _ = STARTED_AT.set(Instant::now());
}

fn uptime_secs() -> u64 {
    STARTED_AT.get().map(|t| t.elapsed().as_secs()).unwrap_or(0)
}

pub struct CoreKernelControlPort {
    pub shared: SharedServerState,
    pub proxy_client: ProxyClient,
    pub tls_acceptor: SharedTlsAcceptor,
    pub tls_session_cache: TlsSessionCache,
    pub lifecycle: Arc<LifecycleState>,
}

impl CoreKernelControlPort {
    fn snapshot(&self, include_version: bool) -> KernelStatusSnapshot {
        let state = read_state(&self.shared);
        KernelStatusSnapshot {
            generation: state.generation,
            fingerprint: state.fingerprint().to_string(),
            uptime_s: uptime_secs(),
            active_connections: self.lifecycle.active_connections(),
            draining: self.lifecycle.is_draining(),
            version: if include_version {
                Some(VERSION.to_string())
            } else {
                None
            },
            reload_in_progress: reload_in_progress(),
            tls_cert_not_after_epoch: state
                .config
                .primary_server()
                .tls
                .as_ref()
                .and_then(|tls| exyonq_mod_tls::pem_leaf_not_after_epoch(&tls.cert)),
        }
    }

    fn outcome(
        &self,
        ok: bool,
        command: OpsCommand,
        error: Option<String>,
        include_version: bool,
    ) -> OpsCommandOutcome {
        OpsCommandOutcome::from_parts(ok, command, self.snapshot(include_version), error, None)
    }
}

#[async_trait]
impl KernelControlPort for CoreKernelControlPort {
    async fn request_reload(&self, config_path: &Path) -> OpsCommandOutcome {
        if self.lifecycle.shutdown_requested() {
            return self.outcome(
                false,
                OpsCommand::Reload,
                Some("reload rejected: shutdown in progress".into()),
                false,
            );
        }
        match reload_from_path(
            config_path,
            &self.shared,
            &self.proxy_client,
            &self.tls_acceptor,
            &self.tls_session_cache,
        )
        .await
        {
            Ok(result) => {
                let note = if result.identical_no_op {
                    Some("EXY-RELOAD-0008: identical config — NO_OP".into())
                } else {
                    None
                };
                let detail = if result.identical_no_op {
                    "identical_no_op"
                } else {
                    "generation_published"
                };
                crate::observability::emit_audit(crate::observability::AuditEvent {
                    action: "reload",
                    result: "success",
                    detail: Some(detail),
                    request_id: None,
                });
                OpsCommandOutcome::from_parts(
                    true,
                    OpsCommand::Reload,
                    KernelStatusSnapshot {
                        generation: result.state.generation,
                        fingerprint: result.state.fingerprint().to_string(),
                        uptime_s: uptime_secs(),
                        active_connections: self.lifecycle.active_connections(),
                        draining: self.lifecycle.is_draining(),
                        version: None,
                        reload_in_progress: reload_in_progress(),
                        tls_cert_not_after_epoch: None,
                    },
                    note,
                    None,
                )
            }
            Err(err) => {
                // Prefer stable EXY-* prefix when the error already carries one; never invent.
                let message = err.to_string();
                crate::observability::emit_audit(crate::observability::AuditEvent {
                    action: "reload",
                    result: "failure",
                    detail: Some(&message),
                    request_id: None,
                });
                self.outcome(false, OpsCommand::Reload, Some(message), false)
            }
        }
    }

    fn request_drain(&self) -> OpsCommandOutcome {
        self.lifecycle.start_drain();
        // Audit only after drain flag is committed in LifecycleState.
        crate::observability::emit_audit(crate::observability::AuditEvent {
            action: "drain",
            result: "success",
            detail: Some("drain_flag_set"),
            request_id: None,
        });
        self.outcome(true, OpsCommand::Drain, None, true)
    }

    fn request_shutdown(&self) -> OpsCommandOutcome {
        self.lifecycle.request_shutdown();
        crate::observability::emit_audit(crate::observability::AuditEvent {
            action: "shutdown",
            result: "success",
            detail: Some("shutdown_requested"),
            request_id: None,
        });
        self.outcome(true, OpsCommand::Shutdown, None, true)
    }

    fn read_status(&self, include_version: bool) -> OpsCommandOutcome {
        self.outcome(true, OpsCommand::Status, None, include_version)
    }

    fn member_list(&self, upstream: Option<&str>) -> OpsCommandOutcome {
        let runtime = exyonq_mod_proxy::wire_conn::pinned_runtime();
        match runtime.member_list(upstream) {
            Ok(rows) => {
                let payload = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
                self.outcome(true, OpsCommand::MemberList, None, false)
                    .with_payload(payload)
            }
            Err(err) => self.outcome(false, OpsCommand::MemberList, Some(err), false),
        }
    }

    fn member_set(
        &self,
        req: &exyonq_module_api::kernel_control::MemberRequest,
    ) -> OpsCommandOutcome {
        let Some(endpoint_id) = req.endpoint_id.as_deref() else {
            return self.outcome(
                false,
                OpsCommand::MemberSet,
                Some("member set requires endpoint_id".into()),
                false,
            );
        };
        let runtime = exyonq_mod_proxy::wire_conn::pinned_runtime();
        match runtime.member_set(
            req.upstream.as_deref(),
            endpoint_id,
            req.enabled,
            req.weight,
        ) {
            Ok(row) => {
                crate::observability::emit_audit(crate::observability::AuditEvent {
                    action: "member_set",
                    result: "success",
                    detail: Some(&format!(
                        "upstream={} id={} enabled={} weight={}",
                        row.upstream, row.endpoint_id, row.enabled, row.weight
                    )),
                    request_id: None,
                });
                let payload = serde_json::to_string(&row).unwrap_or_else(|_| "{}".into());
                self.outcome(true, OpsCommand::MemberSet, None, false)
                    .with_payload(payload)
            }
            Err(err) => self.outcome(false, OpsCommand::MemberSet, Some(err), false),
        }
    }
}
