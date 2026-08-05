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

//! PS1A lifecycle correctness — portable integration tests.

use exyonq_core::lifecycle::{self, LifecycleState};

#[test]
fn accept_loop_drain_break_requires_zero_active() {
    let ops = LifecycleState::new();
    let token = ops.try_enter().unwrap();
    ops.start_drain();
    assert!(ops.is_draining());
    assert_ne!(ops.active_connections(), 0);
    drop(token);
    assert_eq!(ops.active_connections(), 0);
    assert!(ops.is_draining() && ops.active_connections() == 0);
}

#[tokio::test]
async fn hyper_handoff_reuses_task_local_without_second_enter() {
    let ops = LifecycleState::new();
    let token = ops.try_enter().unwrap();
    lifecycle::run_with_connection_token(token, async {
        assert_eq!(ops.active_connections(), 1);
        let reused = lifecycle::admit_connection(&ops).unwrap();
        drop(reused);
        assert_eq!(ops.active_connections(), 0);
    })
    .await;
    assert_eq!(ops.active_connections(), 0);
}

#[test]
fn spawn_failure_path_drops_token_when_scope_ends() {
    let ops = LifecycleState::new();
    {
        let _token = ops.try_enter().unwrap();
        assert_eq!(ops.active_connections(), 1);
    }
    assert_eq!(ops.active_connections(), 0);
}
