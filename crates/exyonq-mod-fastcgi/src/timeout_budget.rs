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
//! Absolute FastCGI request timeout budget (P1.2).
//!
//! `deadline = start + total_timeout`. Every phase uses
//! `min(configured_phase_timeout, remaining)`. Partial phase caps must never
//! extend work past `deadline`.

use crate::wire::WireError;
use std::time::{Duration, Instant};

/// Default total FastCGI operation budget (includes pool wait).
pub const DEFAULT_FCGI_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Default max wait for a free pool connection.
pub const DEFAULT_FCGI_CHECKOUT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutPhase {
    Checkout,
    Connect,
    Write,
    Read,
    Total,
}

/// Absolute deadline for one FastCGI request attempt (including one safe retry).
#[derive(Debug, Clone)]
pub struct TimeoutBudget {
    start: Instant,
    deadline: Instant,
    pub checkout_cap: Duration,
    pub connect_cap: Duration,
    pub write_cap: Duration,
    pub read_cap: Duration,
}

impl TimeoutBudget {
    pub fn new(
        total: Duration,
        checkout_cap: Duration,
        connect_cap: Duration,
        write_cap: Duration,
        read_cap: Duration,
    ) -> Self {
        let start = Instant::now();
        let total = if total.is_zero() {
            Duration::from_millis(1)
        } else {
            total
        };
        Self {
            start,
            deadline: start + total,
            checkout_cap,
            connect_cap,
            write_cap,
            read_cap,
        }
    }

    pub fn start(&self) -> Instant {
        self.start
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Phase budget: `min(configured, remaining)`. Errors if no time left.
    pub fn phase(&self, configured: Duration) -> Result<Duration, WireError> {
        let rem = self.remaining();
        if rem.is_zero() {
            return Err(WireError::Timeout);
        }
        Ok(configured.min(rem))
    }

    pub fn checkout_budget(&self) -> Result<Duration, WireError> {
        self.phase(self.checkout_cap)
    }

    pub fn connect_budget(&self) -> Result<Duration, WireError> {
        self.phase(self.connect_cap)
    }

    pub fn write_budget(&self) -> Result<Duration, WireError> {
        self.phase(self.write_cap)
    }

    pub fn read_budget(&self) -> Result<Duration, WireError> {
        self.phase(self.read_cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_zero_when_total_tiny() {
        let budget = TimeoutBudget::new(
            Duration::from_millis(1),
            Duration::from_secs(10),
            Duration::from_secs(10),
            Duration::from_secs(10),
            Duration::from_secs(10),
        );
        std::thread::sleep(Duration::from_millis(5));
        assert!(budget.expired());
        assert!(budget.phase(Duration::from_secs(1)).is_err());
    }

    #[test]
    fn phase_never_exceeds_remaining() {
        let budget = TimeoutBudget::new(
            Duration::from_millis(50),
            Duration::from_secs(10),
            Duration::from_secs(10),
            Duration::from_secs(10),
            Duration::from_secs(10),
        );
        let w = budget.write_budget().expect("write");
        assert!(w <= Duration::from_millis(50));
        assert!(w <= budget.write_cap);
    }

    #[test]
    fn total_smaller_than_partial_caps_wins() {
        let budget = TimeoutBudget::new(
            Duration::from_millis(20),
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::from_secs(5),
        );
        let c = budget.connect_budget().expect("connect");
        assert!(c <= Duration::from_millis(20));
    }
}
