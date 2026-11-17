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
//! NG-21: the `[tuning]` geometry that the last committed generation published.
//!
//! The header-read deadline and the header-block cap are read by core I/O, the
//! static wire and the proxy wire, which are separate crates and separate
//! process-lifetime caches. They all resolve through this cell so a single
//! config commit names one value: core publishes it from
//! `reload::publish_runtime_observe_authority`, i.e. only when a generation
//! actually commits, so a rejected reload never changes what the wire reads.
//!
//! `0` means "not configured"; the consumers then fall back to their own
//! environment/default resolution (`EXYONQ_READ_TIMEOUT_MS`, 30 s, 8192 bytes),
//! so an absent `[tuning]` keeps the behaviour the server had before these keys.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// Keep-alive idle / header-read deadline when nothing configures it.
pub const DEFAULT_KEEPALIVE_TIMEOUT_MS: u64 = 30_000;
/// Header-block cap when nothing configures it.
pub const DEFAULT_MAX_HEADER_BYTES: usize = 8192;

static KEEPALIVE_TIMEOUT_MS: AtomicU64 = AtomicU64::new(0);
static MAX_HEADER_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Publish one committed generation's tuning.
///
/// `None` restores the fallback resolution, which is what an absent `[tuning]`
/// table or a key that the operator removed must do on the next reload.
pub fn publish(keepalive_timeout_ms: Option<u64>, max_header_bytes: Option<usize>) {
    KEEPALIVE_TIMEOUT_MS.store(keepalive_timeout_ms.unwrap_or(0), Ordering::Release);
    MAX_HEADER_BYTES.store(max_header_bytes.unwrap_or(0), Ordering::Release);
}

/// Configured keep-alive / header-read deadline, or `None` to use the fallback.
pub fn configured_keepalive_timeout() -> Option<Duration> {
    match KEEPALIVE_TIMEOUT_MS.load(Ordering::Acquire) {
        0 => None,
        ms => Some(Duration::from_millis(ms)),
    }
}

/// Configured header-block cap, or `None` to use the fallback.
pub fn configured_max_header_bytes() -> Option<usize> {
    match MAX_HEADER_BYTES.load(Ordering::Acquire) {
        0 => None,
        bytes => Some(bytes),
    }
}

/// Test-only: forget the published generation (simulates a fresh process).
#[doc(hidden)]
pub fn reset_for_tests() {
    publish(None, None);
}

#[cfg(test)]
mod tests {
    use super::{
        configured_keepalive_timeout, configured_max_header_bytes, publish, reset_for_tests,
    };
    use std::time::Duration;

    /// The published geometry is process-global: two tests that mutate it must
    /// not interleave, or one observes the other's value.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    #[test]
    fn unpublished_tuning_falls_back() {
        let _guard = lock();
        reset_for_tests();
        assert_eq!(configured_keepalive_timeout(), None);
        assert_eq!(configured_max_header_bytes(), None);
    }

    #[test]
    fn published_tuning_is_visible_and_removable() {
        let _guard = lock();
        publish(Some(400), Some(16_384));
        assert_eq!(
            configured_keepalive_timeout(),
            Some(Duration::from_millis(400))
        );
        assert_eq!(configured_max_header_bytes(), Some(16_384));
        // A reload that drops the keys must restore the fallback, not keep them.
        publish(None, None);
        assert_eq!(configured_keepalive_timeout(), None);
        assert_eq!(configured_max_header_bytes(), None);
    }
}
