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
//! KD4.9 — control plane and operations runtime.

#![forbid(unsafe_code)]

mod control_socket;
mod purge_socket;
mod service;

pub use purge_socket::{constant_time_eq, parse_purge_line, ParsedPurge};
pub use service::{register_control_plane, ControlPlane};
