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

//! PS1B unified Hyper handoff — portable integration tests.
//!
//! Linux-native handoff gates run in `core/src/server/hyper_handoff.rs` and worker
//! `ps1a_linux_native_tests` / `ps1b_*` in-module tests.

use exyonq_core::lifecycle::{self, LifecycleState};

#[test]
fn handoff_path_reuses_task_local_without_second_enter() {
    let ops = LifecycleState::new();
    let token = ops.try_enter().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        lifecycle::run_with_connection_token(token, async {
            assert_eq!(ops.active_connections(), 1);
            let reused = lifecycle::admit_connection(&ops).unwrap();
            drop(reused);
            assert_eq!(ops.active_connections(), 0);
        })
        .await;
    });
    assert_eq!(ops.active_connections(), 0);
}
