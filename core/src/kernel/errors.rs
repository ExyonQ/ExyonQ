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
//! Contractual error surface — no libc errno, epoll, or worker internals.

pub use crate::lifecycle::DrainRejected;

/// Semantic wire planning rejected (empty headers or invalid input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WirePlanningError;

/// Handoff bundle could not be constructed or spawned (I/O setup failure).
#[derive(Debug)]
pub struct HandoffConstructionError(pub std::io::Error);

impl From<std::io::Error> for HandoffConstructionError {
    fn from(err: std::io::Error) -> Self {
        Self(err)
    }
}

impl std::fmt::Display for HandoffConstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for HandoffConstructionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}
