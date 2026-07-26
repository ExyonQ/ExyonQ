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
//! Composition-root registration for the unix control plane.

use crate::control_socket::UnixControlPlane;
use exyonq_module_api::kernel_control::{
    register_control_plane_service, ControlPlaneRegisterError,
};
use std::sync::Arc;

pub use crate::control_socket::UnixControlPlane as ControlPlane;

pub fn register_control_plane() -> Result<(), ControlPlaneRegisterError> {
    register_control_plane_service(Arc::new(UnixControlPlane))
}
