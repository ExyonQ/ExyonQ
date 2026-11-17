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
//! This module also owns the *fallback* resolution, so no consumer parses the
//! environment or repeats the defaults: [`header_read_deadline`] is
//! `[tuning]` -> `EXYONQ_READ_TIMEOUT_MS` (observed once per process) -> 30 s,
//! and [`max_header_bytes`] is `[tuning]` -> 8192 bytes. `0` in either atomic
//! means "not configured", which keeps the behaviour an absent `[tuning]`
//! always had.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// Keep-alive idle / header-read deadline when nothing configures it.
pub const DEFAULT_KEEPALIVE_TIMEOUT_MS: u64 = 30_000;
/// Header-block cap when nothing configures it.
pub const DEFAULT_MAX_HEADER_BYTES: usize = 8192;

static KEEPALIVE_TIMEOUT_MS: AtomicU64 = AtomicU64::new(0);
static MAX_HEADER_BYTES: AtomicUsize = AtomicUsize::new(0);
/// Process-lifetime observation of the fallback env (0 = unresolved; else ms + 1).
static ENV_READ_TIMEOUT_MS: AtomicU64 = AtomicU64::new(0);

/// Publish one committed generation's tuning.
///
/// `None` restores the fallback resolution, which is what an absent `[tuning]`
/// table or a key that the operator removed must do on the next reload.
pub fn publish(keepalive_timeout_ms: Option<u64>, max_header_bytes: Option<usize>) {
    KEEPALIVE_TIMEOUT_MS.store(keepalive_timeout_ms.unwrap_or(0), Ordering::Release);
    MAX_HEADER_BYTES.store(max_header_bytes.unwrap_or(0), Ordering::Release);
}

/// Header-read / keep-alive deadline the server applies.
///
/// `[tuning].keepalive_timeout_ms`, else `EXYONQ_READ_TIMEOUT_MS` resolved once
/// per process, else [`DEFAULT_KEEPALIVE_TIMEOUT_MS`]. Nothing outside this
/// module parses that env var or repeats the default.
pub fn header_read_deadline() -> Duration {
    if let Some(configured) = configured_keepalive_timeout() {
        return configured;
    }
    let cached = ENV_READ_TIMEOUT_MS.load(Ordering::Relaxed);
    if cached != 0 {
        return Duration::from_millis(cached - 1);
    }
    let resolved = std::env::var("EXYONQ_READ_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_KEEPALIVE_TIMEOUT_MS));
    let _ = ENV_READ_TIMEOUT_MS.compare_exchange(
        0,
        resolved.as_millis() as u64 + 1,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
    Duration::from_millis(ENV_READ_TIMEOUT_MS.load(Ordering::Relaxed) - 1)
}

/// Header-block cap the server applies: `[tuning].max_header_bytes`, else
/// [`DEFAULT_MAX_HEADER_BYTES`].
pub fn max_header_bytes() -> usize {
    configured_max_header_bytes().unwrap_or(DEFAULT_MAX_HEADER_BYTES)
}

/// Configured keep-alive / header-read deadline, or `None` to use the fallback.
fn configured_keepalive_timeout() -> Option<Duration> {
    match KEEPALIVE_TIMEOUT_MS.load(Ordering::Acquire) {
        0 => None,
        ms => Some(Duration::from_millis(ms)),
    }
}

/// Configured header-block cap, or `None` to use the fallback.
fn configured_max_header_bytes() -> Option<usize> {
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

/// Test-only: forget the observed `EXYONQ_READ_TIMEOUT_MS` (simulates a fresh
/// process, or lets a test change the env and observe it again).
#[doc(hidden)]
pub fn reset_env_cache_for_tests() {
    ENV_READ_TIMEOUT_MS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::{
        header_read_deadline, max_header_bytes, publish, reset_env_cache_for_tests,
        reset_for_tests, DEFAULT_KEEPALIVE_TIMEOUT_MS, DEFAULT_MAX_HEADER_BYTES,
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
        assert_eq!(max_header_bytes(), DEFAULT_MAX_HEADER_BYTES);
    }

    #[test]
    fn published_tuning_is_visible_and_removable() {
        let _guard = lock();
        publish(Some(400), Some(16_384));
        assert_eq!(header_read_deadline(), Duration::from_millis(400));
        assert_eq!(max_header_bytes(), 16_384);
        // A reload that drops the keys must restore the fallback, not keep them.
        publish(None, None);
        assert_eq!(max_header_bytes(), DEFAULT_MAX_HEADER_BYTES);
    }

    /// The env fallback is observed once and is always weaker than the config
    /// key: publishing `[tuning]` must win even after the env was resolved.
    #[test]
    fn env_fallback_is_resolved_once_and_yields_to_the_config_key() {
        let _guard = lock();
        let previous = std::env::var("EXYONQ_READ_TIMEOUT_MS").ok();
        std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "100");
        reset_for_tests();
        reset_env_cache_for_tests();
        assert_eq!(header_read_deadline(), Duration::from_millis(100));

        publish(Some(400), None);
        assert_eq!(header_read_deadline(), Duration::from_millis(400));

        publish(None, None);
        assert_eq!(
            header_read_deadline(),
            Duration::from_millis(100),
            "the observed env value is reused, not re-read"
        );

        // Non-u64 values (including `-1` and an empty string) mean "unset";
        // `0` is a literal zero-millisecond deadline, not a sentinel.
        for invalid in ["nonsense", "", "-1"] {
            std::env::set_var("EXYONQ_READ_TIMEOUT_MS", invalid);
            reset_env_cache_for_tests();
            assert_eq!(
                header_read_deadline(),
                Duration::from_millis(DEFAULT_KEEPALIVE_TIMEOUT_MS),
                "`{invalid}` falls back to the default deadline"
            );
        }
        std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "0");
        reset_env_cache_for_tests();
        assert_eq!(header_read_deadline(), Duration::from_millis(0));
        std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "125");
        reset_env_cache_for_tests();
        assert_eq!(header_read_deadline(), Duration::from_millis(125));

        match previous {
            Some(value) => std::env::set_var("EXYONQ_READ_TIMEOUT_MS", value),
            None => std::env::remove_var("EXYONQ_READ_TIMEOUT_MS"),
        }
        reset_env_cache_for_tests();
    }
}
