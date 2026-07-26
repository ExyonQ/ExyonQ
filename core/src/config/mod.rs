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
//! Native TOML configuration IR (re-export from `exyonq-config-ir`).

pub use exyonq_config_ir::{
    canonical_json, fingerprint, AcmeConfig, AppConfig, CachePolicyConfig, CompressionConfig,
    ConfigError, FcgiPoolConfig, HtaccessMode, IrFingerprint, MetricsConfig, ModulesConfig,
    RateLimitConfig, RawConfigInput, RedirectConfig, RouteConfig, RouteMatch, ServerConfig,
    ServerNames, TlsConfig, UpstreamConfig, CONFIG_VERSION_V1, CONFIG_VERSION_V2,
    DEFAULT_UPSTREAM_TIMEOUT_MS,
};
