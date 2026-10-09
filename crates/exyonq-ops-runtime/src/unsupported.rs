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
//! Hosts without Unix sockets have no control-plane listener.
//! Registration is a no-op so the process can still start. The server only
//! binds `EXYONQ_CONTROL_SOCKET` and the purge socket on Unix.

use exyonq_module_api::kernel_control::ControlPlaneRegisterError;

pub fn register_control_plane() -> Result<(), ControlPlaneRegisterError> {
    Ok(())
}
