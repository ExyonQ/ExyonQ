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
//! INTERNAL — Linux FFI capsule (ADR-045). Tokio-free by default.
//! NOT STABLE PUBLIC API. Sole intended home for product-tree `unsafe` syscall wrappers
//! behind Safe APIs (CFD + mod-static + core share this crate).

// ADR-045 capsule: this crate is the allowlisted home for Linux syscall `unsafe`.

#[cfg(target_os = "linux")]
mod fd;
#[cfg(unix)]
mod flock;
#[cfg(target_os = "linux")]
mod net;
#[cfg(target_os = "linux")]
mod openat2;
#[cfg(target_os = "linux")]
mod sendfile;
#[cfg(unix)]
mod umask;

#[cfg(feature = "tokio-io")]
mod read_buf;

#[cfg(target_os = "linux")]
pub use fd::{close_raw_fd, dup_raw_fd};
#[cfg(unix)]
pub use flock::try_flock_exclusive;
#[cfg(target_os = "linux")]
pub use net::{send_all_nosignal, send_vectored_nosignal, set_abortive_linger, write_all_fd};
#[cfg(target_os = "linux")]
pub use openat2::{openat2_beneath, openat2_rdonly_beneath, Openat2Error};
#[cfg(target_os = "linux")]
pub use sendfile::{sendfile64_step, SendfileStep};
#[cfg(unix)]
pub use umask::with_umask;

#[cfg(feature = "tokio-io")]
pub use read_buf::assume_init_advance;

/// Capsule identity for composition / gates.
pub const CAPSULE_ID: &str = "exyonq-linux-ffi:adr-045";
