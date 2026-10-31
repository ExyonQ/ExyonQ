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
//! `flock(2)` exclusive lock helper (CFD single-process gate).

use std::io;
use std::os::fd::RawFd;

/// Non-blocking exclusive flock. Returns `Ok(true)` if locked, `Ok(false)` if would block.
pub fn try_flock_exclusive(fd: RawFd) -> io::Result<bool> {
    let rc = unsafe {
        // SAFETY: `fd` must remain open for the lock lifetime owned by the caller.
        libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB)
    };
    if rc == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) || err.kind() == io::ErrorKind::WouldBlock {
        return Ok(false);
    }
    Err(err)
}
