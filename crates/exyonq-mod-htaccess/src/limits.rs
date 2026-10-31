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
//! Plan 05B limits — fail closed.

pub const MAX_DIRECTORY_DEPTH: usize = 32;
pub const MAX_HTACCESS_FILES_PER_VHOST: usize = 4096;
pub const MAX_BYTES_PER_FILE: usize = 1024 * 1024;
pub const MAX_BYTES_TOTAL_PER_VHOST: usize = 16 * 1024 * 1024;
pub const MAX_DIRECTIVES_PER_FILE: usize = 8192;
pub const WATCHER_DEBOUNCE_MS: u64 = 150;
