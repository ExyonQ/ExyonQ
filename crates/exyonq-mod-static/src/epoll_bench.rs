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
//! Epoll bench wire responses (P1/P7) — module-owned, core supplies fd only.

use crate::epoll_session::runtime_for_hooks;
use std::os::unix::io::RawFd;

pub enum EpollBenchWriteResult {
    Written,
    /// Caller should hand off to Tokio async path.
    Handoff,
    NoMatch,
}

pub fn try_write_bench_response(
    site_slot: u32,
    fd: RawFd,
    head: &[u8],
    p1_wire: &[u8],
    write_fn: fn(RawFd, &[u8]) -> std::io::Result<()>,
) -> EpollBenchWriteResult {
    if head.starts_with(b"GET /health ") {
        if write_fn(fd, &crate::wire::HEALTH_KEEP_ALIVE).is_ok() {
            return EpollBenchWriteResult::Written;
        }
        return EpollBenchWriteResult::NoMatch;
    }
    let Ok(root) = runtime_for_hooks().root_for_slot_public(site_slot) else {
        return EpollBenchWriteResult::NoMatch;
    };
    if root.is_bench_one_k_head(head) {
        if write_fn(fd, p1_wire).is_ok() {
            return EpollBenchWriteResult::Written;
        }
        return EpollBenchWriteResult::NoMatch;
    }
    if root.is_bench_route_head(head) {
        if let Some(route_wire) = root.bench_route_wire_bytes() {
            if write_fn(fd, route_wire).is_ok() {
                return EpollBenchWriteResult::Written;
            }
            return EpollBenchWriteResult::NoMatch;
        }
        return EpollBenchWriteResult::Handoff;
    }
    if let Some(wire_bytes) = root.match_bench_head(head) {
        if write_fn(fd, wire_bytes.as_ref()).is_ok() {
            return EpollBenchWriteResult::Written;
        }
        return EpollBenchWriteResult::NoMatch;
    }
    EpollBenchWriteResult::Handoff
}
