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
//! Tokio `ReadBuf` helper — optional feature `tokio-io` (not used by CFD).

use tokio::io::ReadBuf;

/// Mark `n` bytes of `buf`'s unfilled region as initialized and advance the cursor.
///
/// # Preconditions
/// The previous `poll_read` (or equivalent) must have written exactly `n` bytes into
/// the slice returned by `buf.initialize_unfilled()` / equivalent unfilled region.
pub fn assume_init_advance(buf: &mut ReadBuf<'_>, n: usize) {
    unsafe {
        // SAFETY: caller guarantees `n` bytes were initialized in the unfilled region.
        buf.assume_init(n);
    }
    buf.advance(n);
}
