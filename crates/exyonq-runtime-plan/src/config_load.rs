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

/// Load config IR from disk for reload via include-aware merge (Cap047 LA-CAP047-002).
///
/// Same path as `exyonqctl config lint` / `exyonq serve` — no substring heuristic.
pub fn load_config_for_reload(path: &Path) -> anyhow::Result<AppConfig> {
    exyonq_config_merge::load_with_includes(path).map_err(Into::into)
}
