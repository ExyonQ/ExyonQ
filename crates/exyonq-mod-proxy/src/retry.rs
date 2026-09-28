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
//! Cap021 — conservative connect-only upstream retry policy.

use std::time::{Duration, Instant};

/// Cap021 v1 hard ceiling (config rejects higher).
pub const CAP021_MAX_CONNECT_RETRIES_V1: u8 = 1;

/// Default when `max_connect_retries` omitted from config.
pub const CAP021_DEFAULT_MAX_CONNECT_RETRIES: u8 = 1;

/// Result of one upstream Hyper attempt before status mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamAttemptClass {
    /// `hyper_util` Connect kind — proven no request bytes sent.
    ConnectFailed,
    /// Whole-attempt wall timeout (ambiguous) — do not retry.
    TimedOut,
    /// Send/protocol/other Hyper error — may have sent bytes — do not retry.
    UnretryableError,
}

/// True only for Hyper legacy client connect-class errors.
pub fn is_connect_error(err: &hyper_util::client::legacy::Error) -> bool {
    err.is_connect()
}

pub fn classify_hyper_error(err: &hyper_util::client::legacy::Error) -> UpstreamAttemptClass {
    if is_connect_error(err) {
        UpstreamAttemptClass::ConnectFailed
    } else {
        UpstreamAttemptClass::UnretryableError
    }
}

/// Remaining budget from a shared deadline. Zero ⇒ no further attempt.
pub fn remaining_budget(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

/// Cap037: build a shared wall deadline without panicking on Duration overflow.
/// Overflow ⇒ deadline already elapsed (immediate TimedOut path).
pub fn shared_deadline(timeout: Duration) -> Instant {
    Instant::now().checked_add(timeout).unwrap_or_else(|| {
        Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now)
    })
}

/// Whether another connect-only attempt is allowed.
pub fn may_retry_connect(
    max_connect_retries: u8,
    retries_used: u8,
    class: UpstreamAttemptClass,
    remaining: Duration,
) -> bool {
    class == UpstreamAttemptClass::ConnectFailed
        && retries_used < max_connect_retries
        && !remaining.is_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_connect_class_retries_within_budget() {
        assert!(may_retry_connect(
            1,
            0,
            UpstreamAttemptClass::ConnectFailed,
            Duration::from_millis(10)
        ));
        assert!(!may_retry_connect(
            1,
            1,
            UpstreamAttemptClass::ConnectFailed,
            Duration::from_millis(10)
        ));
        assert!(!may_retry_connect(
            1,
            0,
            UpstreamAttemptClass::TimedOut,
            Duration::from_millis(10)
        ));
        assert!(!may_retry_connect(
            1,
            0,
            UpstreamAttemptClass::UnretryableError,
            Duration::from_millis(10)
        ));
        assert!(!may_retry_connect(
            1,
            0,
            UpstreamAttemptClass::ConnectFailed,
            Duration::ZERO
        ));
        assert!(!may_retry_connect(
            0,
            0,
            UpstreamAttemptClass::ConnectFailed,
            Duration::from_millis(10)
        ));
    }

    #[test]
    fn shared_deadline_overflow_is_elapsed_not_panic() {
        let d = shared_deadline(Duration::MAX);
        assert!(remaining_budget(d).is_zero());
    }
}
