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
//! Config-driven accept-worker geometry (NG-21): an explicit `[tuning]` value
//! wins over the environment and is clamped by the resolver.

#[test]
fn explicit_tuning_value_wins_and_clamps() {
    const EXPLICIT_WORKERS: usize = 3;
    const INVALID_ZERO: usize = 0;
    const INVALID_ABOVE_MAX: usize = 99;
    const LOWER_CLAMP: usize = 1;
    const UPPER_CLAMP: usize = 16;

    assert_eq!(
        exyonq_core::server::resolve_accept_workers_with(Some(EXPLICIT_WORKERS)),
        EXPLICIT_WORKERS
    );
    // The IR rejects 0/17; the resolver still clamps defensively.
    assert_eq!(
        exyonq_core::server::resolve_accept_workers_with(Some(INVALID_ZERO)),
        LOWER_CLAMP
    );
    assert_eq!(
        exyonq_core::server::resolve_accept_workers_with(Some(INVALID_ABOVE_MAX)),
        UPPER_CLAMP
    );
}
