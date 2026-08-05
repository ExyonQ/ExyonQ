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
//! Single-slot registry for composition-root contract services (KD1.1 / KD2D).
//!
//! Production: register-once at process startup (CLI). Reads clone an [`Arc`] under a mutex.
//! Tests: per-thread overrides via [`ContractServiceTestGuard`] — parallel-safe, restored on drop.

use exyonq_module_api::fcgi_dispatch::FcgiRegisterError;
use exyonq_module_api::htaccess_runtime::HtaccessRegisterError;
use exyonq_module_api::proxy_dispatch::ProxyRegisterError;
use exyonq_module_api::static_dispatch::StaticRegisterError;
use std::cell::RefCell;
use std::sync::{Arc, Mutex, MutexGuard};

/// Per-thread test override — `Inherit` uses the global composition-root slot.
pub(crate) enum TestServiceOverride<S: ?Sized> {
    Inherit,
    ForceAbsent,
    Override(Arc<S>),
}

impl<S: ?Sized> Clone for TestServiceOverride<S> {
    fn clone(&self) -> Self {
        match self {
            Self::Inherit => Self::Inherit,
            Self::ForceAbsent => Self::ForceAbsent,
            Self::Override(service) => Self::Override(Arc::clone(service)),
        }
    }
}

/// Register-once slot for a contract dispatch service (reusable for future module backends).
pub struct ContractServiceSlot<S: ?Sized> {
    inner: Mutex<Option<Arc<S>>>,
}

impl<S: ?Sized> Default for ContractServiceSlot<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: ?Sized> ContractServiceSlot<S> {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Register the service exactly once (composition root).
    pub fn register(&self, service: Arc<S>) -> Result<(), FcgiRegisterError> {
        let mut slot = self.inner.lock().map_err(|_| FcgiRegisterError::Poisoned)?;
        if slot.is_some() {
            return Err(FcgiRegisterError::AlreadyRegistered);
        }
        *slot = Some(service);
        Ok(())
    }

    /// Register the service exactly once — static contract error type (KD2.2).
    pub fn register_static(&self, service: Arc<S>) -> Result<(), StaticRegisterError> {
        let mut slot = self
            .inner
            .lock()
            .map_err(|_| StaticRegisterError::Poisoned)?;
        if slot.is_some() {
            return Err(StaticRegisterError::AlreadyRegistered);
        }
        *slot = Some(service);
        Ok(())
    }

    /// Register the service exactly once — proxy contract error type (KD3.2).
    pub fn register_proxy(&self, service: Arc<S>) -> Result<(), ProxyRegisterError> {
        let mut slot = self
            .inner
            .lock()
            .map_err(|_| ProxyRegisterError::Poisoned)?;
        if slot.is_some() {
            return Err(ProxyRegisterError::AlreadyRegistered);
        }
        *slot = Some(service);
        Ok(())
    }

    /// Register the service exactly once — htaccess runtime contract (KD4.2).
    pub fn register_htaccess(&self, service: Arc<S>) -> Result<(), HtaccessRegisterError> {
        let mut slot = self
            .inner
            .lock()
            .map_err(|_| HtaccessRegisterError::Poisoned)?;
        if slot.is_some() {
            return Err(HtaccessRegisterError::AlreadyRegistered);
        }
        *slot = Some(service);
        Ok(())
    }

    /// Clone the registered service handle, if any.
    pub fn get(&self) -> Option<Arc<S>> {
        self.inner.lock().ok()?.clone()
    }

    /// Test-only global reset — use only under [`fcgi_service_registration_test_gate`] for global register-once tests.
    #[doc(hidden)]
    pub fn clear_for_tests(&self) {
        if let Ok(mut slot) = self.inner.lock() {
            *slot = None;
        }
    }

    /// Resolve service for current thread: test override wins over global slot.
    pub(crate) fn resolve(&self, override_state: &TestServiceOverride<S>) -> Option<Arc<S>> {
        match override_state {
            TestServiceOverride::Inherit => self.get(),
            TestServiceOverride::ForceAbsent => None,
            TestServiceOverride::Override(service) => Some(Arc::clone(service)),
        }
    }
}

/// RAII per-thread contract service override (KD2D — parallel test safe).
pub(crate) struct ContractServiceTestGuard<S: ?Sized + 'static> {
    tls: &'static std::thread::LocalKey<RefCell<TestServiceOverride<S>>>,
    previous: TestServiceOverride<S>,
}

impl<S: ?Sized + 'static> ContractServiceTestGuard<S> {
    pub fn install(
        tls: &'static std::thread::LocalKey<RefCell<TestServiceOverride<S>>>,
        service: Arc<S>,
    ) -> Self {
        let previous = tls.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestServiceOverride::Override(service);
            prev
        });
        Self { tls, previous }
    }

    pub fn force_absent(
        tls: &'static std::thread::LocalKey<RefCell<TestServiceOverride<S>>>,
    ) -> Self {
        let previous = tls.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = TestServiceOverride::ForceAbsent;
            prev
        });
        Self { tls, previous }
    }
}

impl<S: ?Sized + 'static> Drop for ContractServiceTestGuard<S> {
    fn drop(&mut self) {
        self.tls
            .with(|cell| *cell.borrow_mut() = self.previous.clone());
    }
}

/// Serializes global contract service mutation across register-once tests (KD1.1/KD2.2).
static CONTRACT_SERVICE_REGISTRATION_GATE: Mutex<()> = Mutex::new(());

#[doc(hidden)]
pub fn fcgi_service_registration_test_gate() -> MutexGuard<'static, ()> {
    CONTRACT_SERVICE_REGISTRATION_GATE
        .lock()
        .expect("contract service registration test gate poisoned")
}

/// Alias for static + FastCGI global register-once tests.
#[doc(hidden)]
pub fn contract_service_registration_test_gate() -> MutexGuard<'static, ()> {
    fcgi_service_registration_test_gate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    thread_local! {
        static DUMMY_TLS: RefCell<TestServiceOverride<Dummy>> =
            const { RefCell::new(TestServiceOverride::Inherit) };
    }

    struct Dummy(u8);
    impl Dummy {
        fn arc(v: u8) -> Arc<Dummy> {
            Arc::new(Dummy(v))
        }
    }

    static SLOT: ContractServiceSlot<Dummy> = ContractServiceSlot::new();

    #[test]
    fn register_once_rejects_second_register() {
        let _gate = fcgi_service_registration_test_gate();
        SLOT.clear_for_tests();
        SLOT.register(Dummy::arc(1)).expect("first");
        assert_eq!(
            SLOT.register(Dummy::arc(2)),
            Err(FcgiRegisterError::AlreadyRegistered)
        );
        SLOT.clear_for_tests();
    }

    #[test]
    fn parallel_thread_local_overrides_do_not_cross_threads() {
        static STARTED: AtomicUsize = AtomicUsize::new(0);
        let barrier = Arc::new(Barrier::new(2));

        let t1 = {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let _guard = ContractServiceTestGuard::install(&DUMMY_TLS, Dummy::arc(10));
                STARTED.fetch_add(1, Ordering::SeqCst);
                barrier.wait();
                assert_eq!(
                    SLOT.resolve(&DUMMY_TLS.with(|c| c.borrow().clone()))
                        .map(|d| d.0),
                    Some(10)
                );
            })
        };
        let t2 = {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let _guard = ContractServiceTestGuard::force_absent(&DUMMY_TLS);
                STARTED.fetch_add(1, Ordering::SeqCst);
                barrier.wait();
                assert!(SLOT
                    .resolve(&DUMMY_TLS.with(|c| c.borrow().clone()))
                    .is_none());
            })
        };
        t1.join().unwrap();
        t2.join().unwrap();
        assert_eq!(STARTED.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn guard_restores_previous_override_on_drop() {
        std::thread::spawn(|| {
            let outer = ContractServiceTestGuard::install(&DUMMY_TLS, Dummy::arc(1));
            {
                let _inner = ContractServiceTestGuard::force_absent(&DUMMY_TLS);
                assert!(SLOT
                    .resolve(&DUMMY_TLS.with(|c| c.borrow().clone()))
                    .is_none());
            }
            assert_eq!(
                SLOT.resolve(&DUMMY_TLS.with(|c| c.borrow().clone()))
                    .map(|d| d.0),
                Some(1)
            );
            drop(outer);
            assert!(matches!(
                DUMMY_TLS.with(|c| c.borrow().clone()),
                TestServiceOverride::Inherit
            ));
        })
        .join()
        .expect("join");
    }

    #[test]
    fn guard_restores_on_panic() {
        let result = std::panic::catch_unwind(|| {
            let _guard = ContractServiceTestGuard::install(&DUMMY_TLS, Dummy::arc(99));
            panic!("simulated test failure");
        });
        assert!(result.is_err());
        assert!(matches!(
            DUMMY_TLS.with(|c| c.borrow().clone()),
            TestServiceOverride::Inherit
        ));
    }
}
