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
//! KD4.7 — FastCGI script resolution contract (core ↔ mod-fastcgi seam).
//!
//! Policy ownership: **module**. Core must treat outcomes mechanically.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

/// Register-once error for the FastCGI script resolver contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastcgiScriptResolverRegisterError {
    AlreadyRegistered,
    Poisoned,
}

/// Resolution purpose — the module may choose different probe budgets for `Probe` vs `Dispatch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastcgiScriptResolutionPurpose {
    /// Resolve script fields for an actual FastCGI dispatch.
    Dispatch,
    /// Resolve existence/shape as part of overlay/directory-index probing.
    Probe,
}

/// Inputs required to resolve a FastCGI script target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastcgiScriptResolutionRequest {
    /// Request path (URI path, no query), overlay-adjusted if applicable.
    pub uri_path: String,
    /// Config-compiled document root (may be missing in compat modes).
    pub pool_document_root: Option<PathBuf>,
    /// Current runtime generation (for metrics/debug; not required for correctness).
    pub generation: u64,
    pub purpose: FastcgiScriptResolutionPurpose,
}

/// Outcome of FastCGI script resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastcgiScriptResolutionOutcome {
    Resolved {
        script_filename: String,
        script_name: String,
        path_info: Option<String>,
        document_root: String,
    },
    NotFound,
    Forbidden,
    InvalidPath,
    ProbeError,
}

/// Module-owned resolver service. Must be cheap to call and thread-safe.
pub trait FastcgiScriptResolutionService: Send + Sync {
    fn resolve(&self, request: &FastcgiScriptResolutionRequest) -> FastcgiScriptResolutionOutcome;
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
    static RESOLVER_TLS: RefCell<TestOverride<dyn FastcgiScriptResolutionService>> =
        const { RefCell::new(TestOverride::Inherit) };
}

static RESOLVER_SLOT: Mutex<Option<Arc<dyn FastcgiScriptResolutionService>>> = Mutex::new(None);

/// Register the resolver exactly once (composition root).
pub fn register_fastcgi_script_resolver(
    service: Arc<dyn FastcgiScriptResolutionService>,
) -> Result<(), FastcgiScriptResolverRegisterError> {
    let mut slot = RESOLVER_SLOT
        .lock()
        .map_err(|_| FastcgiScriptResolverRegisterError::Poisoned)?;
    if slot.is_some() {
        return Err(FastcgiScriptResolverRegisterError::AlreadyRegistered);
    }
    *slot = Some(service);
    Ok(())
}

/// Resolve the registered service for the current thread (test overrides apply).
pub fn fastcgi_script_resolver() -> Option<Arc<dyn FastcgiScriptResolutionService>> {
    let override_state = RESOLVER_TLS.with(|c| c.borrow().clone());
    match override_state {
        TestOverride::Inherit => RESOLVER_SLOT.lock().ok()?.clone(),
        TestOverride::ForceAbsent => None,
        TestOverride::Override(service) => Some(service),
    }
}

/// Serializes global register-once mutation across tests.
static REGISTRATION_GATE: Mutex<()> = Mutex::new(());

#[doc(hidden)]
pub fn fastcgi_script_resolver_registration_test_gate() -> MutexGuard<'static, ()> {
    REGISTRATION_GATE
        .lock()
        .expect("fastcgi script resolver registration test gate poisoned")
}

/// Test-only per-thread override, restored on drop (parallel-safe).
pub struct FastcgiScriptResolverTestGuard {
    previous: TestOverride<dyn FastcgiScriptResolutionService>,
}

impl FastcgiScriptResolverTestGuard {
    pub fn install(service: Arc<dyn FastcgiScriptResolutionService>) -> Self {
        let previous = RESOLVER_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::Override(service);
            prev
        });
        Self { previous }
    }

    pub fn force_absent() -> Self {
        let previous = RESOLVER_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::ForceAbsent;
            prev
        });
        Self { previous }
    }
}

impl Drop for FastcgiScriptResolverTestGuard {
    fn drop(&mut self) {
        RESOLVER_TLS.with(|cell| *cell.borrow_mut() = self.previous.clone());
    }
}

#[doc(hidden)]
pub fn clear_fastcgi_script_resolver_for_tests() {
    if let Ok(mut slot) = RESOLVER_SLOT.lock() {
        *slot = None;
    }
}
