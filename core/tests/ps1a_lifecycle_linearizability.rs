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

//! PS1A-LINUX: concurrent linearizability tests for `try_enter` / `start_drain`.

use exyonq_core::lifecycle::{self, LifecycleState};
use std::sync::{Arc, Barrier};
use std::thread;

#[test]
fn n_threads_try_enter_at_drain_boundary_leaves_zero_active() {
    let ops = LifecycleState::new();
    let n = 64;
    let start = Arc::new(Barrier::new(n + 1));
    let mut handles = Vec::new();

    for _ in 0..n {
        let ops = Arc::clone(&ops);
        let start = Arc::clone(&start);
        handles.push(thread::spawn(move || {
            start.wait();
            drop(ops.try_enter());
        }));
    }
    start.wait();
    ops.start_drain();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(ops.active_connections(), 0);
    assert!(ops.drain_complete());
}

#[test]
fn tokens_dropped_in_arbitrary_order_reach_zero() {
    let ops = LifecycleState::new();
    let tokens: Vec<_> = (0..8).map(|_| ops.try_enter().unwrap()).collect();
    ops.start_drain();
    assert!(!ops.drain_complete());
    for token in tokens.into_iter().rev() {
        drop(token);
    }
    assert!(ops.drain_complete());
}

#[tokio::test]
async fn hyper_path_single_token_via_task_local() {
    let ops = LifecycleState::new();
    let token = ops.try_enter().unwrap();
    lifecycle::run_with_connection_token(token, async {
        assert_eq!(ops.active_connections(), 1);
        let hyper = lifecycle::admit_connection(&ops).unwrap();
        assert_eq!(ops.active_connections(), 1);
        drop(hyper);
        assert_eq!(ops.active_connections(), 0);
    })
    .await;
    assert_eq!(ops.active_connections(), 0);
}

#[test]
fn shutdown_with_active_token_cleared_on_drop() {
    let ops = LifecycleState::new();
    let token = ops.try_enter().unwrap();
    ops.request_shutdown();
    assert!(ops.shutdown_requested());
    assert!(!ops.drain_complete());
    drop(token);
    assert!(ops.drain_complete());
}
