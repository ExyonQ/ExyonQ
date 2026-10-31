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
use exyonq_config_ir::AppConfig;
use exyonq_config_merge::load_with_includes;
use exyonq_config_surface::{compile_serverfile, CompileOptions};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    Toml,
    Exy,
}

pub fn detect_kind(path: &Path) -> ConfigKind {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("exy") => ConfigKind::Exy,
        _ => ConfigKind::Toml,
    }
}

pub fn load_app_config(path: &Path) -> Result<AppConfig, String> {
    match detect_kind(path) {
        ConfigKind::Exy => {
            let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let toml =
                compile_serverfile(&raw, CompileOptions::default()).map_err(|e| e.to_string())?;
            AppConfig::parse_str(&toml).map_err(|e| e.to_string())
        }
        ConfigKind::Toml => load_with_includes(path).map_err(|e| e.to_string()),
    }
}
