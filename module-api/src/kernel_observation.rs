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
//! KD4.12 — kernel shell HTTP observation (501 on contract routes).

use std::cell::RefCell;
use std::sync::{Arc, Mutex, MutexGuard};

/// Shell-level HTTP 501 counters emitted before/alongside module runtime metrics.
pub trait KernelObservationService: Send + Sync {
    fn note_fastcgi_http_501(&self);
    fn note_static_http_501(&self);
    fn note_proxy_http_501(&self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelObservationRegisterError {
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
    static KERNEL_OBSERVATION_TLS: RefCell<TestOverride<dyn KernelObservationService>> =
        const { RefCell::new(TestOverride::Inherit) };
}

static KERNEL_OBSERVATION_SLOT: Mutex<Option<Arc<dyn KernelObservationService>>> = Mutex::new(None);

pub fn register_kernel_observation_service(
    service: Arc<dyn KernelObservationService>,
) -> Result<(), KernelObservationRegisterError> {
    let mut slot = KERNEL_OBSERVATION_SLOT
        .lock()
        .map_err(|_| KernelObservationRegisterError::Poisoned)?;
    if slot.is_some() {
        return Err(KernelObservationRegisterError::AlreadyRegistered);
    }
    *slot = Some(service);
    Ok(())
}

pub fn kernel_observation_service() -> Option<Arc<dyn KernelObservationService>> {
    let override_state = KERNEL_OBSERVATION_TLS.with(|c| c.borrow().clone());
    match override_state {
        TestOverride::Inherit => KERNEL_OBSERVATION_SLOT.lock().ok()?.clone(),
        TestOverride::ForceAbsent => None,
        TestOverride::Override(service) => Some(service),
    }
}

#[inline]
pub fn note_fastcgi_http_501() {
    if let Some(service) = kernel_observation_service() {
        service.note_fastcgi_http_501();
    }
}

#[inline]
pub fn note_static_http_501() {
    if let Some(service) = kernel_observation_service() {
        service.note_static_http_501();
    }
}

#[inline]
pub fn note_proxy_http_501() {
    if let Some(service) = kernel_observation_service() {
        service.note_proxy_http_501();
    }
}

static REGISTRATION_GATE: Mutex<()> = Mutex::new(());

#[doc(hidden)]
pub fn kernel_observation_registration_test_gate() -> MutexGuard<'static, ()> {
    REGISTRATION_GATE
        .lock()
        .expect("kernel observation registration test gate poisoned")
}

pub struct KernelObservationTestGuard {
    previous: TestOverride<dyn KernelObservationService>,
}

impl KernelObservationTestGuard {
    pub fn install(service: Arc<dyn KernelObservationService>) -> Self {
        let previous = KERNEL_OBSERVATION_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::Override(service);
            prev
        });
        Self { previous }
    }

    pub fn force_absent() -> Self {
        let previous = KERNEL_OBSERVATION_TLS.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestOverride::ForceAbsent;
            prev
        });
        Self { previous }
    }
}

impl Drop for KernelObservationTestGuard {
    fn drop(&mut self) {
        KERNEL_OBSERVATION_TLS.with(|cell| *cell.borrow_mut() = self.previous.clone());
    }
}

#[doc(hidden)]
pub fn clear_kernel_observation_for_tests() {
    if let Ok(mut slot) = KERNEL_OBSERVATION_SLOT.lock() {
        *slot = None;
    }
}
