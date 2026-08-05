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
//! Connection admission seam — RAII token only; no manual counter access.

use crate::kernel::errors::DrainRejected;
use crate::lifecycle::{ConnectionLifecycleToken, LifecycleState};
use std::sync::Arc;

/// Platform/kernel admission facade (PS1C).
pub struct PlatformConnectionAdmission;

impl PlatformConnectionAdmission {
    /// Admit one connection; token drop is the sole decrement authority.
    pub fn try_admit(ops: &Arc<LifecycleState>) -> Result<ConnectionLifecycleToken, DrainRejected> {
        ops.try_enter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps1c_token_cannot_clone() {
        fn assert_not_clone<T>() {}
        assert_not_clone::<ConnectionLifecycleToken>();
    }

    #[test]
    fn ps1c_admission_rejects_during_drain() {
        let ops = LifecycleState::new();
        ops.start_drain();
        assert!(PlatformConnectionAdmission::try_admit(&ops).is_err());
    }

    #[test]
    fn ps1c_admission_token_drops_once() {
        let ops = LifecycleState::new();
        let token = PlatformConnectionAdmission::try_admit(&ops).expect("enter");
        assert_eq!(ops.active_connections(), 1);
        drop(token);
        assert_eq!(ops.active_connections(), 0);
    }
}
