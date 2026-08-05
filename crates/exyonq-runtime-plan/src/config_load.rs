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
//! Config load helpers for reload/watchers (compile-time boundary).

use exyonq_config_ir::AppConfig;
use std::path::Path;

pub use exyonq_config_merge::{apply_discovery, DiscoveryCluster, DiscoveryOverlay, Profile};

/// Load config IR from disk for reload: v2 includes via `exyonq-config-merge`, else direct parse.
pub fn load_config_for_reload(path: &Path) -> anyhow::Result<AppConfig> {
    if looks_like_v2_include(path) {
        exyonq_config_merge::load_with_includes(path).map_err(Into::into)
    } else {
        let raw = std::fs::read_to_string(path)
            .map_err(|err| anyhow::anyhow!("read config {}: {err}", path.display()))?;
        AppConfig::parse_str(&raw).map_err(Into::into)
    }
}

fn looks_like_v2_include(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    raw.contains("include =") && raw.contains("config_version = 2")
}
