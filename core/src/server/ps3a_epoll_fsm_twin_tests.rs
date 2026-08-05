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
//! PS3A-PM3-R2: core twin of pure epoll header-timeout predicates (FSM moved to platform).
#![cfg(all(test, target_os = "linux"))]

use crate::server::io::find_header_end;
use std::time::{Duration, Instant};

fn header_read_expired(len: usize, buf: &[u8], deadline: Instant, now: Instant) -> bool {
    find_header_end(&buf[..len]).is_none() && now >= deadline
}

#[test]
fn twin_expired_when_reading_incomplete_headers_past_deadline() {
    let past = Instant::now() - Duration::from_secs(1);
    let partial = b"GET / HTTP/1.1\r\nHost:";
    assert!(header_read_expired(
        partial.len(),
        partial,
        past,
        Instant::now()
    ));
}

#[test]
fn twin_not_expired_when_headers_complete() {
    let past = Instant::now() - Duration::from_secs(1);
    let complete = b"GET / HTTP/1.1\r\nHost: x\r\n\r\n";
    assert!(!header_read_expired(
        complete.len(),
        complete,
        past,
        Instant::now()
    ));
}
