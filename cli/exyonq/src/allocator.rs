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

//! Compile-time global allocator selection for the `exyonq` binary.
//!
//! - No Cargo feature → Rust default system allocator (glibc `malloc` on Linux GNU).
//! - `allocator-jemalloc` → `tikv-jemallocator` as Rust `#[global_allocator]`
//!   (glibc remains the process libc; jemalloc is not a glibc substitute).
//!
//! Selection is compile-time only. There is no runtime switch and no official
//! `LD_PRELOAD` configuration. Mimalloc is not a product feature
//! (`MIMALLOC_OFFICIAL_OPTION=NO`; P14ALLOC-MI closed REJECT on RSS/p99).

/// Compile-time allocator identity for diagnostics (`exyonq --version`).
#[cfg(feature = "allocator-jemalloc")]
pub const ALLOCATOR_NAME: &str = "jemalloc";

#[cfg(not(feature = "allocator-jemalloc"))]
pub const ALLOCATOR_NAME: &str = "system";

#[cfg(feature = "allocator-jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
