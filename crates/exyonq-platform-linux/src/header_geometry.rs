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
//! Header geometry of the epoll keep-alive path.
//!
//! Both values come from the shared `exyonq_module_api::runtime_tuning` cell
//! (published when a generation commits), never from a worker-local cache or a
//! direct `EXYONQ_READ_TIMEOUT_MS` read, so the epoll `ConnState` guard and its
//! header buffer cannot disagree with the H1 wire that hands them connections.
//!
//! The module is compiled on every host — not only on Linux — precisely so this
//! delegation is testable where the epoll worker itself cannot run.

use std::time::{Duration, Instant};

use exyonq_module_api::runtime_tuning;

/// Header-read / keep-alive deadline: `[tuning].keepalive_timeout_ms`, else
/// `EXYONQ_READ_TIMEOUT_MS` observed once per process, else 30 s.
pub(crate) fn header_read_deadline() -> Duration {
    runtime_tuning::header_read_deadline()
}

/// Header-block cap: `[tuning].max_header_bytes`, else the historical 8192.
/// Bounds the per-connection `ConnState` buffer.
pub(crate) fn header_block_limit() -> usize {
    runtime_tuning::max_header_bytes()
}

/// Deadline a connection earns for its next request head.
pub(crate) fn renew_read_deadline(now: Instant) -> Instant {
    now + header_read_deadline()
}

/// Slowloris guard: only while reading a head that is still incomplete.
pub(crate) fn header_read_expired(
    reading: bool,
    headers_complete: bool,
    now: Instant,
    deadline: Instant,
) -> bool {
    reading && !headers_complete && now >= deadline
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    use exyonq_module_api::runtime_tuning::{
        publish, reset_env_cache_for_tests, reset_for_tests, DEFAULT_KEEPALIVE_TIMEOUT_MS,
        DEFAULT_MAX_HEADER_BYTES,
    };

    /// The cell and the env are process-global; serialise the tests that move them.
    static TUNING_LOCK: Mutex<()> = Mutex::new(());

    fn locked_tuning() -> MutexGuard<'static, ()> {
        let guard = TUNING_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        reset_for_tests();
        reset_env_cache_for_tests();
        guard
    }

    /// The behaviour the epoll path observes: a 250 ms keep-alive closes an
    /// incomplete head exactly at 250 ms and a 16 KiB cap resizes the buffer.
    #[test]
    fn deadline_and_cap_come_from_the_committed_cell() {
        let _guard = locked_tuning();
        publish(Some(250), Some(16_384));

        assert_eq!(header_block_limit(), 16_384);

        let issued = Instant::now();
        let deadline = renew_read_deadline(issued);
        assert_eq!(deadline, issued + Duration::from_millis(250));

        // Closed at the committed deadline, never before it.
        assert!(!header_read_expired(
            true,
            false,
            issued + Duration::from_millis(249),
            deadline
        ));
        assert!(header_read_expired(
            true,
            false,
            issued + Duration::from_millis(250),
            deadline
        ));
        // A complete head and a sending connection are not slowloris.
        assert!(!header_read_expired(
            true,
            true,
            issued + Duration::from_secs(5),
            deadline
        ));
        assert!(!header_read_expired(
            false,
            false,
            issued + Duration::from_secs(5),
            deadline
        ));
    }

    /// A reload that drops the keys must restore the historical geometry, not
    /// keep the last committed one.
    #[test]
    fn dropping_the_keys_restores_the_historical_geometry() {
        let _guard = locked_tuning();
        publish(Some(250), Some(16_384));
        publish(None, None);

        assert_eq!(header_block_limit(), DEFAULT_MAX_HEADER_BYTES);
        let issued = Instant::now();
        assert_eq!(
            renew_read_deadline(issued),
            issued + Duration::from_millis(DEFAULT_KEEPALIVE_TIMEOUT_MS)
        );
    }

    /// With no `[tuning]`, the env fallback still reaches the epoll guard, and
    /// the committed key keeps winning over it.
    #[test]
    fn the_env_fallback_reaches_the_guard_and_the_config_key_wins() {
        let _guard = locked_tuning();
        let previous = std::env::var("EXYONQ_READ_TIMEOUT_MS").ok();
        std::env::set_var("EXYONQ_READ_TIMEOUT_MS", "125");
        reset_env_cache_for_tests();

        let issued = Instant::now();
        assert_eq!(
            renew_read_deadline(issued),
            issued + Duration::from_millis(125)
        );

        publish(Some(400), None);
        assert_eq!(
            renew_read_deadline(issued),
            issued + Duration::from_millis(400)
        );

        match previous {
            Some(value) => std::env::set_var("EXYONQ_READ_TIMEOUT_MS", value),
            None => std::env::remove_var("EXYONQ_READ_TIMEOUT_MS"),
        }
        reset_for_tests();
        reset_env_cache_for_tests();
    }
}
