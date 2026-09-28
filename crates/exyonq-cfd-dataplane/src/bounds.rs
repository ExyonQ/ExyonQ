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

//! Fixed Phase 4 body relay bounds.
//!
//! Each active proxied connection owns one request-direction relay buffer and
//! one response-direction relay buffer. Both are capped at 16 KiB, so the
//! maximum body relay buffer residency per connection is:
//!
//! `REQUEST_RELAY_BUFFER_BYTES + RESPONSE_RELAY_BUFFER_BYTES = 32_768`.

pub const REQUEST_RELAY_BUFFER_BYTES: usize = 16_384;
pub const RESPONSE_RELAY_BUFFER_BYTES: usize = 16_384;

pub const MAX_PENDING_REQUEST_BYTES: usize = REQUEST_RELAY_BUFFER_BYTES;
pub const MAX_PENDING_RESPONSE_BYTES: usize = RESPONSE_RELAY_BUFFER_BYTES;
pub const MAX_BODY_BUFFER_BYTES_PER_CONNECTION: usize =
    REQUEST_RELAY_BUFFER_BYTES + RESPONSE_RELAY_BUFFER_BYTES;

pub const MAX_CHUNK_LINE_BYTES: usize = 64;
pub const MAX_TRAILER_BYTES: usize = 8_192;
pub const MAX_TRAILER_COUNT: usize = 16;
