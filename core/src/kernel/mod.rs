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
//! **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API**
//!
//! PS1C minimal kernel/platform seam inside `exyonq-core`.
//! `exyonq-platform-linux` depends on this surface for D1 direction.
//!
//! Productive platform→core serve uses [`PlatformConnectionEntry`] only.
//! Do not reassemble admit/plan/Hyper in platform for productive traffic.

pub mod admission;
pub mod epoll_attach;
pub mod errors;
pub mod generation;
pub mod handoff;
pub mod platform_entry;
pub mod wire;

#[cfg(test)]
pub(crate) mod test_hooks;

pub use crate::server::accepted_connection::{AcceptedConnection, TransportKind};
pub use crate::server::connection_errors::{
    ConnectionError, ConnectionServeOutcome, CorePolicyError,
};
#[cfg(target_os = "linux")]
pub use crate::server::{spawn_hyper_handoff, xff_from_peer};
pub use admission::PlatformConnectionAdmission;
pub use epoll_attach::{
    EpollAttachDecision, EpollAttachRejectReason, EpollConnectionAttachment, EpollHyperCapability,
    EpollKeepaliveTransfer,
};
pub use errors::{DrainRejected, HandoffConstructionError, WirePlanningError};
pub use generation::GenerationView;
pub use handoff::HyperHandoff;
pub use platform_entry::PlatformConnectionEntry;
pub use wire::{plan_wire_decision, WirePlanDecision};
