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
//! KD4.14 — config filesystem watcher and OS signal listeners → `KernelControlPort`.

pub mod config_watcher;
pub mod service;
pub mod signal_runtime;

pub use config_watcher::{spawn_config_watcher, CONFIG_RELOAD_DEBOUNCE_MS};
pub use service::{register_reload_runtime, FileReloadRuntime};
pub use signal_runtime::spawn_shutdown_listener;
