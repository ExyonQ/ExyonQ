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
//! KD4.9 — kernel control port and ops runtime control-plane contract.

use async_trait::async_trait;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::cache_purge::{CachePurgePort, CachePurgeSocketConfig};

/// Administrative command surface (lifecycle only — purge lives on CachePurgePort).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpsCommand {
    Reload,
    Status,
    Drain,
    Shutdown,
}

/// Neutral kernel status snapshot (no secrets, no full config).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelStatusSnapshot {
    pub generation: u64,
    pub fingerprint: String,
    pub uptime_s: u64,
    pub active_connections: u64,
    pub draining: bool,
    pub version: Option<String>,
    /// True while a structural reload is in PREPARE/COMMIT (P1.4-WS5).
    pub reload_in_progress: bool,
}

/// Outcome of an administrative command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpsCommandOutcome {
    pub ok: bool,
    pub command: OpsCommand,
    pub snapshot: KernelStatusSnapshot,
    pub error: Option<String>,
    /// Stable diagnostic code when present (e.g. EXY-RELOAD-0008).
    pub code: Option<String>,
}

impl OpsCommandOutcome {
    /// Build an outcome, deriving `code` from a leading `EXY-*-NNNN` token in `error` when
    /// `code` is not supplied explicitly.
    pub fn from_parts(
        ok: bool,
        command: OpsCommand,
        snapshot: KernelStatusSnapshot,
        error: Option<String>,
        code: Option<String>,
    ) -> Self {
        let code = code.or_else(|| error.as_deref().and_then(Self::stable_code_from_message));
        Self {
            ok,
            command,
            snapshot,
            error,
            code,
        }
    }

    /// Parse a leading stable diagnostic id (`EXY-FAMILY-NNNN`) from a human/JSON message.
    /// Does not invent codes; returns `None` when no stable token is present.
    pub fn stable_code_from_message(message: &str) -> Option<String> {
        let token = message
            .trim()
            .split(|c: char| c == ':' || c.is_whitespace())
            .next()?;
        if !token.starts_with("EXY-") {
            return None;
        }
        // EXY-<FAMILY>-<4 digits> — refuse free-form suffixes as codes.
        let mut parts = token.split('-');
        let (Some("EXY"), Some(_family), Some(num), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return None;
        };
        if num.len() == 4 && num.chars().all(|c| c.is_ascii_digit()) {
            Some(token.to_string())
        } else {
            None
        }
    }

    /// `ok` / `code` pairing rules for reload diagnostics (product contract).
    ///
    /// - `EXY-RELOAD-0008` (identical NO_OP) requires `ok == true`
    /// - other `EXY-RELOAD-*` codes require `ok == false`
    /// - missing code is always consistent (caller may not know a stable id yet)
    pub fn code_result_consistent(&self) -> bool {
        match self.code.as_deref() {
            None => true,
            Some("EXY-RELOAD-0008") => self.ok,
            Some(code) if code.starts_with("EXY-RELOAD-") => !self.ok,
            Some(code) if code.starts_with("EXY-") => true,
            Some(_) => false,
        }
    }
}

/// Narrow capability port implemented by the kernel — no worker/fd/plan internals exposed.
#[async_trait]
pub trait KernelControlPort: Send + Sync {
    async fn request_reload(&self, config_path: &Path) -> OpsCommandOutcome;
    fn request_drain(&self) -> OpsCommandOutcome;
    fn request_shutdown(&self) -> OpsCommandOutcome;
    fn read_status(&self, include_version: bool) -> OpsCommandOutcome;
}

/// Module-owned control plane (unix socket, command parsing, JSON formatting).
pub trait ControlPlaneService: Send + Sync {
    fn spawn_unix_control_socket(
        &self,
        socket_path: PathBuf,
        config_path: PathBuf,
        port: Arc<dyn KernelControlPort>,
    );

    /// Dedicated FPC purge socket (WC3) — not the lifecycle socket.
    fn spawn_unix_cache_purge_socket(
        &self,
        config: CachePurgeSocketConfig,
        port: Arc<dyn CachePurgePort>,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneRegisterError {
    AlreadyRegistered,
    Poisoned,
}

enum TestOverride<S: ?Sized> {
    Inherit,
    ForceAbsent,
    Override(Arc<S>),
}

impl<S: ?Sized> Clone for TestOverride<S> {
    fn clone(&self) -> Self {
        match self {
            Self::Inherit => Self::Inherit,
            Self::ForceAbsent => Self::ForceAbsent,
            Self::Override(svc) => Self::Override(Arc::clone(svc)),
        }
    }
}

thread_local! {
    static CONTROL_PLANE_TLS: RefCell<TestOverride<dyn ControlPlaneService>> =
        const { RefCell::new(TestOverride::Inherit) };
}

static CONTROL_PLANE_SLOT: Mutex<Option<Arc<dyn ControlPlaneService>>> = Mutex::new(None);

pub fn register_control_plane_service(
    service: Arc<dyn ControlPlaneService>,
) -> Result<(), ControlPlaneRegisterError> {
    let mut slot = CONTROL_PLANE_SLOT
        .lock()
        .map_err(|_| ControlPlaneRegisterError::Poisoned)?;
    if slot.is_some() {
        return Err(ControlPlaneRegisterError::AlreadyRegistered);
    }
    *slot = Some(service);
    Ok(())
}

pub fn control_plane_service() -> Option<Arc<dyn ControlPlaneService>> {
    let override_state = CONTROL_PLANE_TLS.with(|c| c.borrow().clone());
    match override_state {
        TestOverride::Inherit => CONTROL_PLANE_SLOT.lock().ok()?.clone(),
        TestOverride::ForceAbsent => None,
        TestOverride::Override(service) => Some(service),
    }
}

static REGISTRATION_GATE: Mutex<()> = Mutex::new(());

#[doc(hidden)]
pub fn control_plane_registration_test_gate() -> MutexGuard<'static, ()> {
    REGISTRATION_GATE
        .lock()
        .expect("control plane registration test gate poisoned")
}

pub struct ControlPlaneTestGuard {
    previous: TestOverride<dyn ControlPlaneService>,
}

impl ControlPlaneTestGuard {
    pub fn install(service: Arc<dyn ControlPlaneService>) -> Self {
        let previous = CONTROL_PLANE_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::Override(service);
            prev
        });
        Self { previous }
    }

    pub fn force_absent() -> Self {
        let previous = CONTROL_PLANE_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::ForceAbsent;
            prev
        });
        Self { previous }
    }
}

impl Drop for ControlPlaneTestGuard {
    fn drop(&mut self) {
        CONTROL_PLANE_TLS.with(|cell| *cell.borrow_mut() = self.previous.clone());
    }
}

#[doc(hidden)]
pub fn clear_control_plane_for_tests() {
    if let Ok(mut slot) = CONTROL_PLANE_SLOT.lock() {
        *slot = None;
    }
}

#[cfg(test)]
mod outcome_code_tests {
    use super::*;

    fn snap() -> KernelStatusSnapshot {
        KernelStatusSnapshot {
            generation: 1,
            fingerprint: "fp".into(),
            uptime_s: 0,
            active_connections: 0,
            draining: false,
            version: None,
            reload_in_progress: false,
        }
    }

    #[test]
    fn stable_code_from_message_extracts_prefix() {
        assert_eq!(
            OpsCommandOutcome::stable_code_from_message("EXY-RELOAD-0008: identical"),
            Some("EXY-RELOAD-0008".into())
        );
        assert_eq!(
            OpsCommandOutcome::stable_code_from_message("shutdown in progress"),
            None
        );
    }

    #[test]
    fn success_has_no_code() {
        let o = OpsCommandOutcome::from_parts(true, OpsCommand::Reload, snap(), None, None);
        assert!(o.ok);
        assert_eq!(o.code, None);
        assert!(o.code_result_consistent());
    }

    #[test]
    fn no_op_requires_ok_true() {
        let o = OpsCommandOutcome::from_parts(
            true,
            OpsCommand::Reload,
            snap(),
            Some("EXY-RELOAD-0008: identical config — NO_OP".into()),
            None,
        );
        assert_eq!(o.code.as_deref(), Some("EXY-RELOAD-0008"));
        assert!(o.code_result_consistent());
        let bad = OpsCommandOutcome {
            ok: false,
            code: Some("EXY-RELOAD-0008".into()),
            ..o
        };
        assert!(!bad.code_result_consistent());
    }

    #[test]
    fn rejected_and_failed_codes_require_ok_false() {
        for code in ["EXY-RELOAD-0002", "EXY-RELOAD-0003", "EXY-RELOAD-0009", "EXY-RELOAD-0010"] {
            let o = OpsCommandOutcome::from_parts(
                false,
                OpsCommand::Reload,
                snap(),
                Some(format!("{code}: detail")),
                None,
            );
            assert_eq!(o.code.as_deref(), Some(code));
            assert!(o.code_result_consistent());
        }
    }

    #[test]
    fn restart_required_is_failure_shaped() {
        let o = OpsCommandOutcome::from_parts(
            false,
            OpsCommand::Reload,
            snap(),
            Some("EXY-RELOAD-0005: listener change requires restart".into()),
            None,
        );
        assert_eq!(o.code.as_deref(), Some("EXY-RELOAD-0005"));
        assert!(!o.ok);
        assert!(o.code_result_consistent());
    }
}
