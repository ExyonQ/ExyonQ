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
//! Owned / raw FD helpers (Linux).

use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

/// Duplicate an open fd. Caller owns the returned [`OwnedFd`].
pub fn dup_raw_fd(fd: RawFd) -> io::Result<OwnedFd> {
    let dup_fd = unsafe {
        // SAFETY: `fd` must be open; `dup` returns a new independent fd or -1.
        libc::dup(fd)
    };
    if dup_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe {
        // SAFETY: `dup_fd` is a freshly duplicated open fd exclusive to this OwnedFd.
        OwnedFd::from_raw_fd(dup_fd)
    })
}

/// Close a raw fd. Prefer [`OwnedFd`] drop when possible.
pub fn close_raw_fd(fd: RawFd) {
    unsafe {
        // SAFETY: caller guarantees `fd` is open and not used after this call.
        let _ = libc::close(fd);
    }
}
