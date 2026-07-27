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
//! IR merge: includes, discovery overlays, profiles.

mod product_profile;

pub use product_profile::{
    expand_product_profile, render_product_profile_toml, ProductProfile, ProductProfileInputs,
    INTERNAL_PROFILE_DISPOSITION,
};

/// Maximum include nesting depth (root file = depth 0).
pub const MAX_INCLUDE_DEPTH: usize = 32;
/// Per-file read bound for root and include fragments (offline CLI / merge).
pub const MAX_INCLUDE_FILE_BYTES: u64 = 8 * 1024 * 1024;

use exyonq_config_ir::{
    AppConfig, CachePolicyConfig, ConfigError, FcgiPoolConfig, FullPageCacheConfig, Http3Config,
    RawConfigInput, RouteConfig, ServerConfig, UpstreamConfig, CONFIG_VERSION_V2,
    DEFAULT_UPSTREAM_TIMEOUT_MS,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFragment {
    #[serde(default)]
    route: Vec<RouteConfig>,
    #[serde(default)]
    upstream: Vec<UpstreamConfig>,
    #[serde(default)]
    fcgi_pool: Vec<FcgiPoolConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoot {
    config_version: u32,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    server: Vec<ServerConfig>,
    #[serde(default)]
    route: Vec<RouteConfig>,
    #[serde(default)]
    upstream: Vec<UpstreamConfig>,
    #[serde(default)]
    fcgi_pool: Vec<FcgiPoolConfig>,
    #[serde(default)]
    cache_policy: Vec<CachePolicyConfig>,
    #[serde(default)]
    modules: exyonq_config_ir::ModulesConfig,
    #[serde(default, rename = "static")]
    static_section: exyonq_config_ir::StaticSectionConfig,
    #[serde(default)]
    full_page_cache: FullPageCacheConfig,
    #[serde(default)]
    http3: Http3Config,
}

/// Load IR from path, resolving `include` fragments (config_version = 2).
pub fn load_with_includes(path: &Path) -> Result<AppConfig, ConfigError> {
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let raw: RawRoot = toml::from_str(&contents).map_err(ConfigError::parse)?;
    if raw.include.is_empty() {
        return AppConfig::from_raw_input(raw_to_ir(raw)).map_err(|e| e.with_path(path));
    }
    if raw.config_version != CONFIG_VERSION_V2 {
        return Err(
            ConfigError::Parse("include requires config_version = 2".into()).with_path(path),
        );
    }
    merge_includes(base_dir, raw).map_err(|e| e.with_path(path))
}

fn merge_includes(base_dir: &Path, mut root: RawRoot) -> Result<AppConfig, ConfigError> {
    let includes = std::mem::take(&mut root.include);
    for rel in includes {
        let fragment_path = base_dir.join(&rel);
        if !fragment_path.is_file() {
            return Err(ConfigError::IncludeNotFound {
                path: fragment_path,
            });
        }
        let contents =
            std::fs::read_to_string(&fragment_path).map_err(|source| ConfigError::Read {
                path: fragment_path.clone(),
                source,
            })?;
        let fragment: RawFragment = toml::from_str(&contents).map_err(ConfigError::parse)?;
        append_unique(&mut root.route, fragment.route, "route")?;
        append_unique(&mut root.upstream, fragment.upstream, "upstream")?;
        append_unique(&mut root.fcgi_pool, fragment.fcgi_pool, "fcgi_pool")?;
    }
    AppConfig::from_raw_input(raw_to_ir(root))
}

fn append_unique<T>(dest: &mut Vec<T>, src: Vec<T>, kind: &'static str) -> Result<(), ConfigError>
where
    T: Named,
{
    for item in src {
        if dest.iter().any(|existing| existing.name() == item.name()) {
            return Err(ConfigError::IncludeConflict {
                message: format!("duplicate {kind} name `{}`", item.name()),
            });
        }
        dest.push(item);
    }
    Ok(())
}

trait Named {
    fn name(&self) -> &str;
}

impl Named for RouteConfig {
    fn name(&self) -> &str {
        &self.name
    }
}

impl Named for UpstreamConfig {
    fn name(&self) -> &str {
        &self.name
    }
}

impl Named for FcgiPoolConfig {
    fn name(&self) -> &str {
        &self.name
    }
}

impl Named for CachePolicyConfig {
    fn name(&self) -> &str {
        &self.name
    }
}

fn raw_to_ir(raw: RawRoot) -> RawConfigInput {
    RawConfigInput {
        config_version: raw.config_version,
        include: raw.include,
        server: raw.server,
        route: raw.route,
        upstream: raw.upstream,
        fcgi_pool: raw.fcgi_pool,
        cache_policy: raw.cache_policy,
        modules: raw.modules,
        static_section: raw.static_section,
        full_page_cache: raw.full_page_cache,
        http3: raw.http3,
    }
}

/// Discovery overlay JSON (merge-spec: override upstream targets by name).
#[derive(Debug, Default, Deserialize)]
pub struct DiscoveryOverlay {
    #[serde(default)]
    pub upstreams: HashMap<String, String>,
    #[serde(default)]
    pub clusters: HashMap<String, DiscoveryCluster>,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveryCluster {
    #[serde(default)]
    pub endpoints: Vec<String>,
}

impl DiscoveryOverlay {
    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }
}

/// Apply discovery overlay per merge-spec (upstream target override).
pub fn apply_discovery(config: &AppConfig, overlay: &DiscoveryOverlay) -> AppConfig {
    let mut next = config.clone();
    for (name, target) in &overlay.upstreams {
        if let Some(upstream) = next.upstreams.get_mut(name) {
            upstream.target = target.clone();
        }
    }
    for (name, cluster) in &overlay.clusters {
        if let Some(endpoint) = cluster.endpoints.first() {
            if let Some(upstream) = next.upstreams.get_mut(name) {
                upstream.target = format!("http://{endpoint}");
            }
        }
    }
    next
}

/// Profile transforms (IR-level presets, not runtime branches).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Dev,
    Edge,
    Lb,
}

pub fn apply_profile(mut config: AppConfig, profile: Profile) -> AppConfig {
    match profile {
        Profile::Dev => {
            config.modules.metrics.enabled = true;
            for upstream in config.upstreams.values_mut() {
                upstream.timeout_ms = upstream.timeout_ms.max(60_000);
            }
        }
        Profile::Edge => {
            config.modules.compression.enabled = false;
        }
        Profile::Lb => {
            for upstream in config.upstreams.values_mut() {
                upstream.timeout_ms = upstream.timeout_ms.min(DEFAULT_UPSTREAM_TIMEOUT_MS);
            }
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn merge_includes_appends_routes() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("frag.toml"),
            r#"
[[route]]
name = "extra"
match = { path = "/extra" }
root = "/var/www"
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.toml"),
            r#"
config_version = 2
include = ["frag.toml"]

[[server]]
listen = "127.0.0.1:8080"
routes = ["api", "extra"]

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
        let config = load_with_includes(&dir.path().join("main.toml")).unwrap();
        assert_eq!(config.routes.len(), 2);
    }

    #[test]
    fn merge_rejects_duplicate_route_names() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("dup.toml"),
            r#"
[[route]]
name = "api"
match = { path = "/dup" }
root = "/tmp"
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.toml"),
            r#"
config_version = 2
include = ["dup.toml"]

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
        assert!(matches!(
            load_with_includes(&dir.path().join("main.toml")).unwrap_err(),
            ConfigError::IncludeConflict { .. }
        ));
    }
}
