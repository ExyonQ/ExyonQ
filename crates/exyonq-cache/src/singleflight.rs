use crate::key::CacheKey;
use crate::metrics::{note_singleflight_follower, note_singleflight_leader};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

pub(crate) struct Flight {
    notify: Notify,
    done: AtomicBool,
}

pub(crate) enum JoinResult {
    Leader(Arc<Flight>),
    Follower(Arc<Flight>),
}

/// Per-key singleflight coordinator (lock held only for register/wait/retire).
pub struct Singleflight {
    flights: Mutex<HashMap<CacheKey, Arc<Flight>>>,
}

impl Default for Singleflight {
    fn default() -> Self {
        Self::new()
    }
}

impl Singleflight {
    pub fn new() -> Self {
        Self {
            flights: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn join(&self, key: &CacheKey) -> JoinResult {
        let mut map = self.flights.lock().expect("flights lock");
        if let Some(existing) = map.get(key) {
            note_singleflight_follower();
            return JoinResult::Follower(Arc::clone(existing));
        }
        let flight = Arc::new(Flight {
            notify: Notify::new(),
            done: AtomicBool::new(false),
        });
        map.insert(key.clone(), Arc::clone(&flight));
        note_singleflight_leader();
        JoinResult::Leader(flight)
    }

    pub(crate) fn finish(&self, key: &CacheKey) {
        let flight = {
            let mut map = self.flights.lock().expect("flights lock");
            map.remove(key)
        };
        if let Some(flight) = flight {
            flight.done.store(true, Ordering::Release);
            flight.notify.notify_waiters();
        }
    }

    pub fn clear(&self) {
        self.flights.lock().expect("flights lock").clear();
    }

    pub(crate) fn has_flight(&self, key: &CacheKey) -> bool {
        self.flights.lock().expect("flights lock").contains_key(key)
    }

    #[doc(hidden)]
    pub fn flights_lock_held(&self) -> bool {
        self.flights.try_lock().is_err()
    }
}

pub(crate) async fn wait_flight(flight: &Flight) {
    while !flight.done.load(Ordering::Acquire) {
        let notified = flight.notify.notified();
        if flight.done.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

#[cfg(any(debug_assertions, feature = "test-utils"))]
type SingleflightFollowerHook = dyn Fn() + Send + Sync;

#[cfg(any(debug_assertions, feature = "test-utils"))]
static SINGLEFLIGHT_FOLLOWER_HOOK: std::sync::LazyLock<
    std::sync::Mutex<Option<Arc<SingleflightFollowerHook>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

#[cfg(any(debug_assertions, feature = "test-utils"))]
pub(crate) fn singleflight_follower_hook() -> Option<Arc<SingleflightFollowerHook>> {
    SINGLEFLIGHT_FOLLOWER_HOOK
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(Arc::clone))
}

#[cfg(any(debug_assertions, feature = "test-utils"))]
#[doc(hidden)]
pub fn set_singleflight_follower_hook_for_tests(hook: Option<Arc<SingleflightFollowerHook>>) {
    if let Ok(mut guard) = SINGLEFLIGHT_FOLLOWER_HOOK.lock() {
        *guard = hook;
    }
}

#[cfg(not(any(debug_assertions, feature = "test-utils")))]
#[doc(hidden)]
pub fn set_singleflight_follower_hook_for_tests(_hook: Option<Arc<dyn Fn() + Send + Sync>>) {}
