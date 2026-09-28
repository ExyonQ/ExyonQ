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
//! ExyonQ HTTP Control API leaf crate.

pub mod auth;
pub mod error;
pub mod http;
pub mod mutate;
pub mod revision;
pub mod service;
pub mod types;

use crate::auth::validate_configured_token;
use crate::error::CtrlError;
use crate::http::{ControlApiBindConfig, ControlApiHandle};
use crate::service::ControlService;
use exyonq_module_api::kernel_control::KernelControlPort;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

pub use types::{
    CapabilitySet, ControlApiError, Health, Revision, RuntimeConfig, Stats, Vhost, VhostCreate,
};

#[derive(Debug, Clone)]
pub struct ControlApiSpawnConfig {
    pub config_path: PathBuf,
    pub unix_socket: Option<PathBuf>,
    pub tcp_addr: Option<SocketAddr>,
    pub token: String,
}

pub async fn spawn_control_api(
    port: Arc<dyn KernelControlPort>,
    config: ControlApiSpawnConfig,
) -> Result<ControlApiHandle, CtrlError> {
    if config.unix_socket.is_none() && config.tcp_addr.is_none() {
        return Err(CtrlError::BadRequest(
            "control api requires at least one bind".to_owned(),
        ));
    }
    let token = Arc::<[u8]>::from(validate_configured_token(config.token.as_bytes())?);
    let service = ControlService::new(port, config.config_path);
    http::spawn_http(
        service,
        ControlApiBindConfig {
            unix_socket: config.unix_socket,
            tcp_addr: config.tcp_addr,
            token,
        },
    )
    .await
}
