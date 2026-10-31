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
//! Coordination provider health (independent of dataplane readiness).

use std::sync::atomic::{AtomicU8, Ordering};

/// Provider health FSM — never blocks ExyonQ traffic readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CoordinationProviderHealth {
    Disabled = 0,
    Connecting = 1,
    Healthy = 2,
    Degraded = 3,
    Reconciling = 4,
}

impl CoordinationProviderHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "DISABLED",
            Self::Connecting => "CONNECTING",
            Self::Healthy => "HEALTHY",
            Self::Degraded => "DEGRADED",
            Self::Reconciling => "RECONCILING",
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Connecting,
            2 => Self::Healthy,
            3 => Self::Degraded,
            4 => Self::Reconciling,
            _ => Self::Disabled,
        }
    }
}

/// Process-local health cell (no secrets / endpoints).
#[derive(Debug)]
pub struct HealthCell {
    state: AtomicU8,
}

impl HealthCell {
    pub fn new(initial: CoordinationProviderHealth) -> Self {
        Self {
            state: AtomicU8::new(initial as u8),
        }
    }

    pub fn get(&self) -> CoordinationProviderHealth {
        CoordinationProviderHealth::from_u8(self.state.load(Ordering::Relaxed))
    }

    pub fn set(&self, next: CoordinationProviderHealth) {
        self.state.store(next as u8, Ordering::Relaxed);
    }
}

/// Snapshot safe for health endpoints (no secrets, no full Redis URL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinationHealthSnapshot {
    pub state: CoordinationProviderHealth,
    pub degraded: bool,
    pub reconcile_needed: bool,
}

impl CoordinationHealthSnapshot {
    pub fn state_label(&self) -> &'static str {
        self.state.as_str()
    }
}
