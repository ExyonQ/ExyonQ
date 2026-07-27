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
//! JSON file discovery overlay — merge upstream targets into IR before compile.

use exyonq_config_ir::AppConfig;
use exyonq_module_api::discovery_runtime::{
    discovery_path_from_env, register_discovery_runtime_service, DiscoveryConfigOverlayService,
    DiscoveryRuntimeRegisterError,
};
use exyonq_runtime_plan::{apply_discovery, DiscoveryOverlay};
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

pub struct FileDiscoveryRuntime;

pub fn register_discovery_runtime() -> Result<(), DiscoveryRuntimeRegisterError> {
    register_discovery_runtime_service(Arc::new(FileDiscoveryRuntime))
}

/// Merge upstream targets from a JSON discovery file into a cloned config (fail-open).
pub fn apply_file_overlay(config: &AppConfig, path: &Path) -> AppConfig {
    FileDiscoveryRuntime.apply_file_overlay(config, path)
}

impl DiscoveryConfigOverlayService for FileDiscoveryRuntime {
    fn apply_file_overlay(&self, config: &AppConfig, path: &Path) -> AppConfig {
        let Ok(raw) = std::fs::read_to_string(path) else {
            warn!(path = %path.display(), "discovery file missing");
            return config.clone();
        };
        let Ok(overlay) = DiscoveryOverlay::from_json(&raw) else {
            warn!(path = %path.display(), "discovery file invalid JSON");
            return config.clone();
        };
        if overlay.upstreams.is_empty() && overlay.clusters.is_empty() {
            return config.clone();
        }
        let next = apply_discovery(config, &overlay);
        for (name, upstream) in &next.upstreams {
            if config
                .upstreams
                .get(name)
                .is_some_and(|prev| prev.target != upstream.target)
            {
                info!(%name, target = %upstream.target, "discovery updated upstream");
            }
        }
        next
    }

    fn apply_env_file_overlay(&self, config: &AppConfig) -> AppConfig {
        match discovery_path_from_env() {
            Some(path) => self.apply_file_overlay(config, &path),
            None => config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_config_ir::AppConfig;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn minimal_config() -> AppConfig {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        std::fs::write(
            &path,
            r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]

[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:9000"
"#,
        )
        .unwrap();
        AppConfig::from_file(&path).expect("config")
    }

    #[test]
    fn invalid_json_keeps_config() {
        let dir = TempDir::new().unwrap();
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{not json").unwrap();
        let config = minimal_config();
        let merged = apply_file_overlay(&config, &bad);
        assert_eq!(
            merged.upstreams.get("backend").unwrap().target,
            config.upstreams.get("backend").unwrap().target
        );
    }

    #[test]
    fn missing_file_keeps_config() {
        let config = minimal_config();
        let merged = apply_file_overlay(&config, Path::new("/tmp/exyonq-no-such-discovery.json"));
        assert_eq!(
            merged.upstreams.get("backend").unwrap().target,
            config.upstreams.get("backend").unwrap().target
        );
    }

    #[test]
    fn upstream_overlay_updates_target() {
        let dir = TempDir::new().unwrap();
        let overlay = dir.path().join("disc.json");
        std::fs::write(
            &overlay,
            r#"{"upstreams":{"backend":"http://127.0.0.1:9100"}}"#,
        )
        .unwrap();
        let config = minimal_config();
        let merged = apply_file_overlay(&config, &overlay);
        assert_eq!(
            merged.upstreams.get("backend").unwrap().target,
            "http://127.0.0.1:9100"
        );
    }

    #[test]
    fn cluster_first_endpoint_updates_target() {
        let dir = TempDir::new().unwrap();
        let overlay = dir.path().join("disc.json");
        std::fs::write(
            &overlay,
            r#"{"clusters":{"backend":{"endpoints":["127.0.0.1:9200"]}}}"#,
        )
        .unwrap();
        let config = minimal_config();
        let merged = apply_file_overlay(&config, &overlay);
        assert_eq!(
            merged.upstreams.get("backend").unwrap().target,
            "http://127.0.0.1:9200"
        );
    }

    #[test]
    fn env_overlay_applies_when_set() {
        let dir = TempDir::new().unwrap();
        let overlay = dir.path().join("disc.json");
        std::fs::write(
            &overlay,
            r#"{"upstreams":{"backend":"http://127.0.0.1:9300"}}"#,
        )
        .unwrap();
        std::env::set_var("EXYONQ_DISCOVERY_FILE", overlay.to_str().unwrap());
        let config = minimal_config();
        let merged = FileDiscoveryRuntime.apply_env_file_overlay(&config);
        assert_eq!(
            merged.upstreams.get("backend").unwrap().target,
            "http://127.0.0.1:9300"
        );
        std::env::remove_var("EXYONQ_DISCOVERY_FILE");
    }
}
