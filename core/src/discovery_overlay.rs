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
//! Mechanical discovery overlay adapter — delegates to registered discovery runtime.

use crate::config::AppConfig;
use exyonq_module_api::discovery_runtime::discovery_runtime_service;

/// Apply env-file discovery overlay when a runtime service is registered.
pub fn apply_env_discovery_overlay(config: AppConfig) -> AppConfig {
    discovery_runtime_service()
        .map(|svc| svc.apply_env_file_overlay(&config))
        .unwrap_or(config)
}
