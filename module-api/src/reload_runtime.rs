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
//! KD4.14 — config watcher and OS signal runtime contract (no notify/signals in core).

use crate::kernel_control::KernelControlPort;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

/// External reload/signal runtime — translates filesystem and OS events into kernel commands.
pub trait ReloadRuntimeService: Send + Sync {
    /// Start watcher and signal listeners; abort tasks when the supervisor is dropped.
    fn start(
        &self,
        config_path: Option<PathBuf>,
        port: Arc<dyn KernelControlPort>,
    ) -> ReloadRuntimeSupervisor;
}

/// Owns background watcher/signal tasks; cancels them on drop.
pub struct ReloadRuntimeSupervisor {
    shutdown: Option<Box<dyn Fn() + Send + Sync>>,
}

impl ReloadRuntimeSupervisor {
    pub fn new(shutdown: Option<Box<dyn Fn() + Send + Sync>>) -> Self {
        Self { shutdown }
    }

    pub fn cancel(&self) {
        if let Some(shutdown) = &self.shutdown {
            shutdown();
        }
    }
}

impl Drop for ReloadRuntimeSupervisor {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadRuntimeRegisterError {
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
    static RELOAD_RUNTIME_TLS: RefCell<TestOverride<dyn ReloadRuntimeService>> =
        const { RefCell::new(TestOverride::Inherit) };
}

static RELOAD_RUNTIME_SLOT: Mutex<Option<Arc<dyn ReloadRuntimeService>>> = Mutex::new(None);

pub fn register_reload_runtime_service(
    service: Arc<dyn ReloadRuntimeService>,
) -> Result<(), ReloadRuntimeRegisterError> {
    let mut slot = RELOAD_RUNTIME_SLOT
        .lock()
        .map_err(|_| ReloadRuntimeRegisterError::Poisoned)?;
    if slot.is_some() {
        return Err(ReloadRuntimeRegisterError::AlreadyRegistered);
    }
    *slot = Some(service);
    Ok(())
}

pub fn reload_runtime_service() -> Option<Arc<dyn ReloadRuntimeService>> {
    let override_state = RELOAD_RUNTIME_TLS.with(|c| c.borrow().clone());
    match override_state {
        TestOverride::Inherit => RELOAD_RUNTIME_SLOT.lock().ok()?.clone(),
        TestOverride::ForceAbsent => None,
        TestOverride::Override(service) => Some(service),
    }
}

static REGISTRATION_GATE: Mutex<()> = Mutex::new(());

#[doc(hidden)]
pub fn reload_runtime_registration_test_gate() -> MutexGuard<'static, ()> {
    REGISTRATION_GATE
        .lock()
        .expect("reload runtime registration test gate poisoned")
}

pub struct ReloadRuntimeTestGuard {
    previous: TestOverride<dyn ReloadRuntimeService>,
}

impl ReloadRuntimeTestGuard {
    pub fn install(service: Arc<dyn ReloadRuntimeService>) -> Self {
        let previous = RELOAD_RUNTIME_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::Override(service);
            prev
        });
        Self { previous }
    }

    pub fn force_absent() -> Self {
        let previous = RELOAD_RUNTIME_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::ForceAbsent;
            prev
        });
        Self { previous }
    }
}

impl Drop for ReloadRuntimeTestGuard {
    fn drop(&mut self) {
        RELOAD_RUNTIME_TLS.with(|cell| *cell.borrow_mut() = self.previous.clone());
    }
}

#[doc(hidden)]
pub fn clear_reload_runtime_for_tests() {
    if let Ok(mut slot) = RELOAD_RUNTIME_SLOT.lock() {
        *slot = None;
    }
}
