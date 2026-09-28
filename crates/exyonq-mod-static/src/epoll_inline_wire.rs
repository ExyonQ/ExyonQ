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
//! Epoll wire responses for product ops probes (`/health`).
//!
//! Cap061: site static files hand off to Hyper/static with full access/correlation.

use crate::epoll_session::runtime_for_hooks;
use std::os::unix::io::RawFd;

pub enum EpollInlineWireResult {
    Written,
    /// Caller should hand off to Tokio async path.
    Handoff,
    NoMatch,
}

pub fn try_write_inline_wire_response(
    site_slot: u32,
    fd: RawFd,
    head: &[u8],
    write_fn: fn(RawFd, &[u8]) -> std::io::Result<()>,
) -> EpollInlineWireResult {
    if head.starts_with(b"GET /health ") || head.starts_with(b"HEAD /health ") {
        if write_fn(fd, crate::wire::health_keep_alive_for_head(head)).is_ok() {
            let started = std::time::Instant::now();
            crate::wire_conn::notify_wire_access_for_hooks(head, 200, "health_probe", started);
            return EpollInlineWireResult::Written;
        }
        return EpollInlineWireResult::NoMatch;
    }
    // Ensure site slot is still live (fail closed → handoff).
    if runtime_for_hooks().root_for_slot_public(site_slot).is_err() {
        return EpollInlineWireResult::NoMatch;
    }
    EpollInlineWireResult::Handoff
}
