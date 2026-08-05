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
//! KD4.10 — discovery config overlay contract (file/env overlay only; no endpoint runtime in v0).

use exyonq_config_ir::AppConfig;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// Applies discovery overlays to IR config before compile/reload (background/cold path).
pub trait DiscoveryConfigOverlayService: Send + Sync {
    /// Merge upstream targets from a JSON discovery file (fail-open on missing/invalid).
    fn apply_file_overlay(&self, config: &AppConfig, path: &Path) -> AppConfig;

    /// Apply overlay from env-configured discovery file, if any.
    fn apply_env_file_overlay(&self, config: &AppConfig) -> AppConfig;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryRuntimeRegisterError {
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
    static DISCOVERY_RUNTIME_TLS: RefCell<TestOverride<dyn DiscoveryConfigOverlayService>> =
        const { RefCell::new(TestOverride::Inherit) };
}

static DISCOVERY_RUNTIME_SLOT: Mutex<Option<Arc<dyn DiscoveryConfigOverlayService>>> =
    Mutex::new(None);

pub fn register_discovery_runtime_service(
    service: Arc<dyn DiscoveryConfigOverlayService>,
) -> Result<(), DiscoveryRuntimeRegisterError> {
    let mut slot = DISCOVERY_RUNTIME_SLOT
        .lock()
        .map_err(|_| DiscoveryRuntimeRegisterError::Poisoned)?;
    if slot.is_some() {
        return Err(DiscoveryRuntimeRegisterError::AlreadyRegistered);
    }
    *slot = Some(service);
    Ok(())
}

pub fn discovery_runtime_service() -> Option<Arc<dyn DiscoveryConfigOverlayService>> {
    let override_state = DISCOVERY_RUNTIME_TLS.with(|c| c.borrow().clone());
    match override_state {
        TestOverride::Inherit => DISCOVERY_RUNTIME_SLOT.lock().ok()?.clone(),
        TestOverride::ForceAbsent => None,
        TestOverride::Override(service) => Some(service),
    }
}

static REGISTRATION_GATE: Mutex<()> = Mutex::new(());

#[doc(hidden)]
pub fn discovery_runtime_registration_test_gate() -> MutexGuard<'static, ()> {
    REGISTRATION_GATE
        .lock()
        .expect("discovery runtime registration test gate poisoned")
}

pub struct DiscoveryRuntimeTestGuard {
    previous: TestOverride<dyn DiscoveryConfigOverlayService>,
}

impl DiscoveryRuntimeTestGuard {
    pub fn install(service: Arc<dyn DiscoveryConfigOverlayService>) -> Self {
        let previous = DISCOVERY_RUNTIME_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::Override(service);
            prev
        });
        Self { previous }
    }

    pub fn force_absent() -> Self {
        let previous = DISCOVERY_RUNTIME_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::ForceAbsent;
            prev
        });
        Self { previous }
    }
}

impl Drop for DiscoveryRuntimeTestGuard {
    fn drop(&mut self) {
        DISCOVERY_RUNTIME_TLS.with(|cell| *cell.borrow_mut() = self.previous.clone());
    }
}

#[doc(hidden)]
pub fn clear_discovery_runtime_for_tests() {
    if let Ok(mut slot) = DISCOVERY_RUNTIME_SLOT.lock() {
        *slot = None;
    }
}

/// Env keys checked in order (existing product behavior).
pub fn discovery_path_from_env() -> Option<PathBuf> {
    [
        "EXYONQ_DISCOVERY_FILE",
        "EXYONQ_DISCOVERY_K8S",
        "EXYONQ_DISCOVERY_DOCKER",
    ]
    .into_iter()
    .find_map(|key| std::env::var(key).ok())
    .map(PathBuf::from)
}
