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
//! ExyonQ configuration IR (versioned TOML schema).

mod canonical;
mod diagnostic;
mod endpoint_set;
#[cfg(test)]
#[path = "endpoint_set_property_tests.rs"]
mod endpoint_set_property_tests;
mod error;
mod full_page_cache;
mod logging;
mod modules;
mod registry;
mod static_preload;
mod upstream_health;
mod waf;

pub use canonical::{canonical_json, fingerprint, IrFingerprint};
pub use diagnostic::{
    offset_to_position, redact_secrets, render_human, sort_diagnostics, span_from_byte_range,
    suggest_typo, Diagnostic, DiagnosticCode, DiagnosticDocument, Position, Severity, SourceSpan,
    SpanQuality, ROOT_FIELD_CATALOG, SERVER_FIELD_CATALOG,
};
pub use endpoint_set::{
    deterministic_endpoint_id, endpoint_from_discovery_hostport, endpoint_from_http_target,
    endpoint_from_raw, AdminEndpointState, Endpoint, EndpointAddress, EndpointFailoverPolicy,
    EndpointId, EndpointSelectionPolicy, EndpointSet, RawEndpointConfig, UserBackendId,
    MAX_BACKENDS_PROVISIONAL, MAX_ENDPOINTS_PER_BACKEND_PROVISIONAL, MAX_ID_LENGTH_PROVISIONAL,
};
pub use error::ConfigError;
pub use full_page_cache::{
    DistributedCacheConfig, DistributedCacheRedisConfig, DistributedCacheSecurityConfig,
    FullPageCacheConfig,
};
pub use logging::{
    validate_logging, AccessLoggingConfig, AuditLoggingConfig, ConsoleLoggingConfig, ConsoleStream,
    FileLoggingConfig, FileRotationConfig, JournaldLoggingConfig, LogFormat, LoggingConfig,
    OtelLoggingConfig, OtlpProtocol, SyslogLoggingConfig,
};
pub use modules::{CompressionConfig, MetricsConfig, ModulesConfig, RateLimitConfig};
pub use registry::{lookup_module, ModuleDirectiveSpec, MODULE_DIRECTIVES_V1};
pub use static_preload::{
    StaticEncodingCacheConfig, StaticPreloadConfig, StaticSectionConfig,
    DEFAULT_ENCODING_CACHE_DIR, DEFAULT_ENCODING_CACHE_LEVEL, DEFAULT_ENCODING_CACHE_MAX_BYTES,
    DEFAULT_ENCODING_CACHE_MAX_ENTRIES, DEFAULT_ENCODING_CACHE_MAX_TOTAL_BYTES,
    DEFAULT_ENCODING_CACHE_MIN_BYTES, DEFAULT_PRELOAD_MAX_ENTRIES, DEFAULT_PRELOAD_MAX_FILE_BYTES,
    DEFAULT_PRELOAD_TOTAL_BYTES, ENCODING_CACHE_LEVEL_CEILING, ENCODING_CACHE_MAX_BYTES_CEILING,
    ENCODING_CACHE_MAX_ENTRIES_CEILING, ENCODING_CACHE_MAX_TOTAL_BYTES_CEILING,
    PRELOAD_MAX_ENTRIES_CEILING, PRELOAD_MAX_FILE_BYTES_CEILING, PRELOAD_MAX_TOTAL_BYTES_CEILING,
};
pub use upstream_health::UpstreamHealthCheckConfig;
pub use waf::{
    WafAbuseConfig, WafAbuseMode, WafBuiltinToggles, WafConfig, WafCustomRuleConfig,
    WafDetectorModeIr, WafEngineIr, WafExclusionConfig, WafFailPolicyIr, WafIpFilterConfig,
    WafModeIr, WafOnInspectionLimitIr, WafPhaseIr, WafRuleActionIr, WafRulesetConfig,
};

use modules::ModulesConfig as RawModulesConfig;
use serde::{Deserialize, Serialize};
use static_preload::StaticSectionConfig as RawStaticSectionConfig;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

pub const CONFIG_VERSION_V1: u32 = 1;
pub const CONFIG_VERSION_V2: u32 = 2;
pub const DEFAULT_UPSTREAM_TIMEOUT_MS: u64 = 30_000;
/// Cap037: operator-facing ceiling; zero remains valid (immediate TimedOut).
pub const MAX_UPSTREAM_TIMEOUT_MS: u64 = 86_400_000;

/// Parsed and validated ExyonQ configuration IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub config_version: u32,
    pub includes: Vec<String>,
    pub servers: Vec<ServerConfig>,
    pub routes: Vec<RouteConfig>,
    pub upstreams: HashMap<String, UpstreamConfig>,
    pub pools_fcgi: HashMap<String, FcgiPoolConfig>,
    pub cache_policies: HashMap<String, CachePolicyConfig>,
    pub modules: ModulesConfig,
    /// `[static]` / `[static.preload]` — tree preload memory limits.
    pub static_section: StaticSectionConfig,
    /// `[full_page_cache]` — WC2B L1 lookup gate (default off).
    pub full_page_cache: FullPageCacheConfig,
    /// `[http3]` — provider-neutral HTTP/3 selection (P13D Phase 5).
    pub http3: Http3Config,
    /// `[waf]` — native WAF engine, rulesets, exclusions, abuse seam.
    pub waf: WafConfig,
    /// `[logging]` — Cap061 production observability (RD-005).
    pub logging: LoggingConfig,
}

/// Top-level `[http3]` IR (neutral; no provider crate types).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Http3Config {
    #[serde(default = "default_http3_enabled")]
    pub enabled: bool,
    /// Product provider: `"s2n"` | `"quiche"`. Absent → IR default `"s2n"` at runtime
    /// when a modern provider is compiled (Quinn-only builds ignore this field).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default = "default_http3_max_ack_delay_ms")]
    pub max_ack_delay_ms: u64,
    #[serde(default = "default_http3_drain_cap")]
    pub request_body_drain_cap_bytes: u64,
    #[serde(default)]
    pub qlog: bool,
}

impl Default for Http3Config {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: None,
            max_ack_delay_ms: default_http3_max_ack_delay_ms(),
            request_body_drain_cap_bytes: default_http3_drain_cap(),
            qlog: false,
        }
    }
}

fn default_http3_enabled() -> bool {
    true
}

fn default_http3_max_ack_delay_ms() -> u64 {
    1
}

fn default_http3_drain_cap() -> u64 {
    64 * 1024
}

/// One response header set on every response from the server that lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseHeaderConfig {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub listen: String,
    #[serde(default)]
    pub server_name: ServerNames,
    #[serde(default)]
    pub routes: Vec<String>,
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    #[serde(default)]
    pub http3_listen: Option<String>,
    /// Headers added to every response from this listener (HSTS, CSP, frame options).
    #[serde(default)]
    pub response_headers: Vec<ResponseHeaderConfig>,
}

impl ServerConfig {
    pub fn server_name_list(&self) -> Vec<String> {
        self.server_name.to_vec()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
    #[serde(default)]
    pub acme: Option<AcmeConfig>,
}

/// Automatic certificate management (ACME v2 / HTTP-01).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcmeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub email: String,
    pub domains: Vec<String>,
    #[serde(default)]
    pub staging: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(untagged)]
pub enum ServerNames {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl ServerNames {
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn to_vec(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::One(name) => vec![name.clone()],
            Self::Many(names) => names.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    pub name: String,
    pub r#match: RouteMatch,
    pub upstream: Option<String>,
    pub root: Option<PathBuf>,
    pub index: Option<String>,
    #[serde(default)]
    pub redirect: Option<RedirectConfig>,
    #[serde(default)]
    pub rewrite: Option<String>,
    #[serde(default)]
    pub fastcgi: Option<String>,
    #[serde(default)]
    pub htaccess: HtaccessMode,
    #[serde(default)]
    pub cache: Option<String>,
    /// When true, this static route may serve `.php`, `.phtml`, `.phar`, and dotfiles.
    /// Default is deny. `.well-known` stays reachable either way.
    #[serde(default)]
    pub allow_sensitive: bool,
}

/// Plan 12 v0 — response cache policy (compile-time only).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachePolicyConfig {
    pub name: String,
    #[serde(default = "default_cache_ttl_seconds")]
    pub ttl_seconds: u64,
    #[serde(default = "default_cache_max_object_bytes")]
    pub max_object_bytes: usize,
}

fn default_cache_ttl_seconds() -> u64 {
    30
}

fn default_cache_max_object_bytes() -> usize {
    1024 * 1024
}

/// Per-route `.htaccess` overlay capability (Plan 05B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HtaccessMode {
    #[default]
    Off,
    Overlay,
}

impl RouteConfig {
    /// Document root used for `.htaccess` discovery when `htaccess = overlay`.
    pub fn htaccess_document_root<'a>(
        &'a self,
        pools_fcgi: &'a HashMap<String, FcgiPoolConfig>,
    ) -> Option<&'a PathBuf> {
        if self.htaccess != HtaccessMode::Overlay {
            return None;
        }
        if let Some(root) = &self.root {
            return Some(root);
        }
        self.fastcgi
            .as_ref()
            .and_then(|name| pools_fcgi.get(name))
            .and_then(|pool| pool.document_root.as_ref())
    }
}

/// Default FastCGI in-flight capacity per pool (PR5-B2).
pub const DEFAULT_FCGI_MAX_CONCURRENCY: u32 = 16;

/// Maximum allowed FastCGI in-flight capacity per pool (PR5-B2).
pub const MAX_FCGI_MAX_CONCURRENCY: u32 = 4096;

/// Structural FastCGI pool declaration (compile-time only — no transport in PR1).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FcgiPoolConfig {
    pub name: String,
    pub address: String,
    #[serde(default)]
    pub document_root: Option<PathBuf>,
    #[serde(default = "default_fcgi_max_concurrency")]
    pub max_concurrency: u32,
    /// Module-path (`exyonq-mod-fastcgi`) open-socket pool bound when set.
    /// Defaults to `max_concurrency` when absent.
    ///
    /// **Not** the CFD project route `max_conn` contract: CFD requires `max_conn=1`
    /// (ADR-042) because CFD FastCGI `execute_get` is serial per shard; CFD idle
    /// retention beyond one socket is unsupported via published project config.
    /// Do not treat this TOML field as a CFD concurrent backend-socket budget.
    #[serde(default)]
    pub max_connections: Option<u32>,
    /// Transport: `unix` or `tcp` (default unix).
    #[serde(default = "default_fcgi_transport")]
    pub transport: String,
    /// Idle pooled connection lifetime in milliseconds (default 30000).
    #[serde(default = "default_fcgi_idle_timeout_ms")]
    pub idle_timeout_ms: u64,
    /// Total request budget in milliseconds (default 30000).
    #[serde(default = "default_fcgi_total_timeout_ms")]
    pub total_timeout_ms: u64,
    /// Pool checkout wait in milliseconds (default 5000).
    #[serde(default = "default_fcgi_checkout_timeout_ms")]
    pub checkout_timeout_ms: u64,
}

fn default_fcgi_max_concurrency() -> u32 {
    DEFAULT_FCGI_MAX_CONCURRENCY
}

fn default_fcgi_transport() -> String {
    "unix".to_string()
}

fn default_fcgi_idle_timeout_ms() -> u64 {
    30_000
}

fn default_fcgi_total_timeout_ms() -> u64 {
    30_000
}

fn default_fcgi_checkout_timeout_ms() -> u64 {
    5_000
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteMatch {
    pub path: String,
    #[serde(default)]
    pub host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedirectConfig {
    #[serde(default = "default_redirect_status")]
    pub status: u16,
    pub location: String,
}

fn default_redirect_status() -> u16 {
    302
}

/// Upstream / logical backend IR (K8S-P2A EndpointSet foundation).
///
/// Legacy `target` sugar remains for TOML compatibility. After normalize,
/// [`Self::endpoint_set`] is authoritative; [`Self::target`] holds the sole
/// executable URI when `len == 1`, else empty (empty set or multi deferred).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamConfig {
    pub name: String,
    /// Sole executable http URI when endpoint set has exactly one endpoint.
    pub target: String,
    pub endpoint_set: EndpointSet,
    pub selection_policy: EndpointSelectionPolicy,
    pub failover_policy: EndpointFailoverPolicy,
    pub timeout_ms: u64,
    /// Cap021: connect-only retries (`hyper` Connect errors). Default 1; Cap021 v1 max 1.
    pub max_connect_retries: u8,
    /// Cap024: opt-in active HTTP health checks (default disabled).
    pub health_check: UpstreamHealthCheckConfig,
}

impl UpstreamConfig {
    /// Construct from legacy single-target fields (tests / programmatic builders).
    pub fn legacy(name: impl Into<String>, target: impl Into<String>, timeout_ms: u64) -> Self {
        let name = name.into();
        let target = target.into();
        Self::try_from_parts(
            name,
            Some(target),
            Vec::new(),
            EndpointSelectionPolicy::default(),
            EndpointFailoverPolicy::default(),
            timeout_ms,
            default_max_connect_retries(),
            UpstreamHealthCheckConfig::default(),
        )
        .expect("valid legacy upstream")
    }

    // Cap021 added max_connect_retries as an 8th parameter; keep the flat
    // constructor for call-site stability (clippy 1.97 threshold is 7).
    #[allow(clippy::too_many_arguments)]
    pub fn try_from_parts(
        name: String,
        target: Option<String>,
        raw_endpoints: Vec<RawEndpointConfig>,
        selection_policy: EndpointSelectionPolicy,
        failover_policy: EndpointFailoverPolicy,
        timeout_ms: u64,
        max_connect_retries: u8,
        health_check: UpstreamHealthCheckConfig,
    ) -> Result<Self, ConfigError> {
        // Cap021 v1: only 0..=1; larger Envoy-style budgets rejected until later admission.
        if max_connect_retries > 1 {
            return Err(ConfigError::Parse(format!(
                "upstream `{name}`: max_connect_retries={max_connect_retries} exceeds Cap021 v1 max 1"
            )));
        }
        // Cap037: bound timeout_ms so Instant+Duration cannot panic; 0 = immediate TimedOut.
        if timeout_ms > MAX_UPSTREAM_TIMEOUT_MS {
            return Err(ConfigError::Parse(format!(
                "upstream `{name}`: timeout_ms={timeout_ms} exceeds max {MAX_UPSTREAM_TIMEOUT_MS}"
            )));
        }
        health_check.validate(&name)?;
        let target_trimmed = target
            .as_ref()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        let has_target = target_trimmed.is_some();
        let has_endpoints = !raw_endpoints.is_empty();
        if has_target && has_endpoints {
            return Err(ConfigError::AmbiguousUpstreamTargetAndEndpoints { upstream: name });
        }
        if raw_endpoints.len() > MAX_ENDPOINTS_PER_BACKEND_PROVISIONAL {
            return Err(ConfigError::EndpointSetLimitExceeded {
                limit: MAX_ENDPOINTS_PER_BACKEND_PROVISIONAL,
                found: raw_endpoints.len(),
            });
        }
        let endpoint_set = if let Some(t) = target_trimmed {
            let ep = endpoint_from_http_target(&t)?;
            EndpointSet::from_endpoints(vec![ep])?
        } else if has_endpoints {
            let mut eps = Vec::with_capacity(raw_endpoints.len());
            for raw in raw_endpoints {
                eps.push(endpoint_from_raw(raw)?);
            }
            EndpointSet::from_endpoints(eps)?
        } else {
            EndpointSet::empty()
        };
        let target = endpoint_set.sole_http_uri().unwrap_or_default();
        let _user_id = UserBackendId::new(&name)?;
        Ok(Self {
            name,
            target,
            endpoint_set,
            selection_policy,
            failover_policy,
            timeout_ms,
            max_connect_retries,
            health_check,
        })
    }

    /// Replace endpoint set from discovery/overlay (full set; no silent truncation).
    pub fn set_endpoint_set(&mut self, endpoint_set: EndpointSet) {
        self.target = endpoint_set.sole_http_uri().unwrap_or_default();
        self.endpoint_set = endpoint_set;
    }

    pub fn user_backend_id(&self) -> Result<UserBackendId, ConfigError> {
        UserBackendId::new(&self.name)
    }

    /// Productive single-endpoint execution is ready when exactly one endpoint
    /// is eligible (`Enabled` + `weight > 0`). Configured set size may be larger
    /// (OPEN-002); full set is retained without silent truncation.
    pub fn single_endpoint_executable(&self) -> bool {
        self.eligible_endpoint_count() == 1
    }

    /// Productive multi-endpoint WRR is ready when two or more endpoints are eligible.
    pub fn multi_endpoint_ready(&self) -> bool {
        self.eligible_endpoint_count() > 1
    }

    fn eligible_endpoint_count(&self) -> usize {
        self.endpoint_set
            .endpoints()
            .iter()
            .filter(|ep| matches!(ep.admin_state, AdminEndpointState::Enabled) && ep.weight > 0)
            .count()
    }

    /// Backward-compatible alias — historically meant "present but deferred".
    #[deprecated(note = "use multi_endpoint_ready")]
    pub fn multi_endpoint_deferred(&self) -> bool {
        self.multi_endpoint_ready()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUpstreamConfig {
    name: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    endpoints: Vec<RawEndpointConfig>,
    #[serde(default)]
    selection_policy: EndpointSelectionPolicy,
    #[serde(default)]
    failover_policy: EndpointFailoverPolicy,
    #[serde(default = "default_upstream_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_max_connect_retries")]
    max_connect_retries: u8,
    #[serde(default)]
    health_check: UpstreamHealthCheckConfig,
}

impl TryFrom<RawUpstreamConfig> for UpstreamConfig {
    type Error = ConfigError;

    fn try_from(raw: RawUpstreamConfig) -> Result<Self, Self::Error> {
        Self::try_from_parts(
            raw.name,
            raw.target,
            raw.endpoints,
            raw.selection_policy,
            raw.failover_policy,
            raw.timeout_ms,
            raw.max_connect_retries,
            raw.health_check,
        )
    }
}

impl<'de> Deserialize<'de> for UpstreamConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawUpstreamConfig::deserialize(deserializer)?;
        Self::try_from(raw).map_err(serde::de::Error::custom)
    }
}

fn default_max_connect_retries() -> u8 {
    1
}

fn default_upstream_timeout_ms() -> u64 {
    DEFAULT_UPSTREAM_TIMEOUT_MS
}

/// Parsed TOML fields before validation (used by merge/includes).
#[derive(Debug, Clone)]
pub struct RawConfigInput {
    pub config_version: u32,
    pub include: Vec<String>,
    pub server: Vec<ServerConfig>,
    pub route: Vec<RouteConfig>,
    pub upstream: Vec<UpstreamConfig>,
    pub fcgi_pool: Vec<FcgiPoolConfig>,
    pub cache_policy: Vec<CachePolicyConfig>,
    pub modules: ModulesConfig,
    pub static_section: StaticSectionConfig,
    pub full_page_cache: FullPageCacheConfig,
    pub http3: Http3Config,
    pub waf: WafConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
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
    modules: RawModulesConfig,
    #[serde(default, rename = "static")]
    static_section: RawStaticSectionConfig,
    #[serde(default)]
    full_page_cache: FullPageCacheConfig,
    #[serde(default)]
    http3: Http3Config,
    #[serde(default)]
    waf: WafConfig,
    #[serde(default)]
    logging: LoggingConfig,
}

impl std::str::FromStr for AppConfig {
    type Err = ConfigError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let raw: RawConfig = toml::from_str(input).map_err(ConfigError::parse)?;
        Self::from_raw(raw)
    }
}

impl AppConfig {
    pub fn parse_str(input: &str) -> Result<Self, ConfigError> {
        input.parse()
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse_str(&contents).map_err(|err| err.with_path(path))
    }

    pub fn from_raw_input(raw: RawConfigInput) -> Result<Self, ConfigError> {
        Self::from_raw(RawConfig {
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
            waf: raw.waf,
            logging: raw.logging,
        })
    }

    pub(crate) fn from_raw(raw: RawConfig) -> Result<Self, ConfigError> {
        let expected = match raw.config_version {
            CONFIG_VERSION_V1 | CONFIG_VERSION_V2 => raw.config_version,
            found => {
                return Err(ConfigError::UnsupportedVersion {
                    found,
                    expected: CONFIG_VERSION_V2,
                });
            }
        };

        if raw.config_version == CONFIG_VERSION_V1 && !raw.include.is_empty() {
            return Err(ConfigError::Parse(
                "include requires config_version = 2".into(),
            ));
        }

        if raw.server.is_empty() {
            return Err(ConfigError::MissingServers);
        }

        let routes = raw.route;
        let route_names = unique_names(routes.iter().map(|route| route.name.as_str()), "route")?;

        let mut upstreams = HashMap::new();
        for upstream in raw.upstream {
            let name = upstream.name.clone();
            if upstreams.insert(name.clone(), upstream).is_some() {
                return Err(ConfigError::DuplicateName {
                    kind: "upstream",
                    name,
                });
            }
        }
        if upstreams.len() > MAX_BACKENDS_PROVISIONAL {
            return Err(ConfigError::BackendLimitExceeded {
                limit: MAX_BACKENDS_PROVISIONAL,
                found: upstreams.len(),
            });
        }

        let mut pools_fcgi = HashMap::new();
        for pool in raw.fcgi_pool {
            let name = pool.name.clone();
            if pools_fcgi.insert(name.clone(), pool).is_some() {
                return Err(ConfigError::DuplicateName {
                    kind: "fcgi_pool",
                    name,
                });
            }
        }

        let mut cache_policies = HashMap::new();
        for policy in raw.cache_policy {
            validate_cache_policy(&policy)?;
            let name = policy.name.clone();
            if cache_policies.insert(name.clone(), policy).is_some() {
                return Err(ConfigError::DuplicateName {
                    kind: "cache_policy",
                    name,
                });
            }
        }

        for server in &raw.server {
            validate_listen(&server.listen)?;
            for route_name in &server.routes {
                if !route_names.contains(route_name) {
                    return Err(ConfigError::UnknownRoute {
                        route: route_name.clone(),
                    });
                }
            }
            if let Some(tls) = &server.tls {
                validate_tls_acme(tls)?;
            }
            for header in &server.response_headers {
                validate_response_header(header)?;
            }
        }

        for route in &routes {
            validate_match_path(&route.r#match.path)?;
            validate_route_cache(route, &cache_policies)?;
            validate_route_actions(route)?;
            validate_htaccess_overlay(route, &pools_fcgi)?;
            if let Some(upstream) = &route.upstream {
                if !upstreams.contains_key(upstream) {
                    return Err(ConfigError::UnknownUpstream {
                        upstream: upstream.clone(),
                        route: route.name.clone(),
                    });
                }
            }
            if let Some(redirect) = &route.redirect {
                validate_redirect_status(redirect.status)?;
                validate_redirect_location(&redirect.location)?;
            }
            if let Some(rewrite) = &route.rewrite {
                validate_rewrite_target(rewrite)?;
            }
            if let Some(pool) = &route.fastcgi {
                if !pools_fcgi.contains_key(pool) {
                    return Err(ConfigError::UnknownFcgiPool {
                        route: route.name.clone(),
                        pool: pool.clone(),
                    });
                }
            }
        }

        for upstream in upstreams.values() {
            // Empty target is valid: empty EndpointSet or multi-endpoint deferred (P2A).
            if !upstream.target.is_empty() {
                validate_upstream_target(&upstream.target)?;
            }
            // Endpoint set already validated in UpstreamConfig::try_from_parts.
            let _ = upstream.user_backend_id()?;
        }

        for pool in pools_fcgi.values() {
            validate_fcgi_pool_address(&pool.name, &pool.address)?;
            validate_fcgi_max_concurrency(&pool.name, pool.max_concurrency)?;
            validate_fcgi_max_connections(&pool.name, pool.max_concurrency, pool.max_connections)?;
            validate_fcgi_transport(&pool.name, &pool.transport)?;
            validate_fcgi_idle_timeout_ms(&pool.name, pool.idle_timeout_ms)?;
            validate_fcgi_total_timeout_ms(&pool.name, pool.total_timeout_ms)?;
            validate_fcgi_checkout_timeout_ms(&pool.name, pool.checkout_timeout_ms)?;
        }

        validate_static_preload(&raw.static_section.preload)?;
        validate_static_encoding_cache(&raw.static_section.encoding_cache)?;
        validate_full_page_cache(&raw.full_page_cache)?;
        validate_http3_config(&raw.http3)?;
        waf::validate_waf(&raw.waf)?;
        validate_metrics_config(&raw.modules.metrics, &raw.server)?;
        validate_ratelimit_config(&raw.modules.ratelimit)?;
        if let Err(message) = validate_logging(&raw.logging) {
            return Err(ConfigError::Parse(message));
        }

        Ok(Self {
            config_version: expected,
            includes: raw.include,
            servers: raw.server,
            routes,
            upstreams,
            pools_fcgi,
            cache_policies,
            modules: raw.modules,
            static_section: raw.static_section,
            full_page_cache: raw.full_page_cache,
            http3: raw.http3,
            waf: raw.waf,
            logging: raw.logging,
        })
    }

    pub fn primary_server(&self) -> &ServerConfig {
        &self.servers[0]
    }

    pub fn primary_listen_addr(&self) -> Result<SocketAddr, ConfigError> {
        let listen = self
            .servers
            .first()
            .map(|server| server.listen.as_str())
            .ok_or(ConfigError::MissingServers)?;
        parse_listen(listen)
    }

    /// One socket per distinct `listen`. The bool is true when that socket speaks TLS.
    pub fn listen_endpoints(&self) -> Result<Vec<(SocketAddr, bool)>, ConfigError> {
        let mut out: Vec<(SocketAddr, bool)> = Vec::new();
        for server in &self.servers {
            let addr = parse_listen(&server.listen)?;
            let tls = server.tls.is_some();
            if let Some(slot) = out.iter_mut().find(|(existing, _)| *existing == addr) {
                slot.1 |= tls;
            } else {
                out.push((addr, tls));
            }
        }
        Ok(out)
    }

    /// Migrate v1 IR to v2 (adds empty includes, preserves semantics).
    pub fn migrate_v1_to_v2(mut self) -> Self {
        if self.config_version == CONFIG_VERSION_V1 {
            self.config_version = CONFIG_VERSION_V2;
        }
        self
    }
}

fn validate_response_header(header: &ResponseHeaderConfig) -> Result<(), ConfigError> {
    let name = header.name.as_str();
    let name_ok = !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !name.eq_ignore_ascii_case("content-length")
        && !name.eq_ignore_ascii_case("transfer-encoding")
        && !name.eq_ignore_ascii_case("connection")
        && !name.eq_ignore_ascii_case("host");
    if !name_ok {
        return Err(ConfigError::Parse(format!(
            "response header name `{name}` is not allowed"
        )));
    }
    let value_ok = !header.value.is_empty()
        && header.value.len() <= 1024
        && !header.value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0);
    if !value_ok {
        return Err(ConfigError::Parse(format!(
            "response header `{name}` has an empty or illegal value"
        )));
    }
    Ok(())
}

fn validate_ratelimit_config(rl: &RateLimitConfig) -> Result<(), ConfigError> {
    if !rl.enabled {
        return Ok(());
    }
    if rl.requests_per_second == 0 {
        return Err(ConfigError::Parse(
            "modules.ratelimit.requests_per_second must be >= 1 when enabled".into(),
        ));
    }
    if rl.burst == 0 {
        return Err(ConfigError::Parse(
            "modules.ratelimit.burst must be >= 1 when enabled".into(),
        ));
    }
    if let Some(path) = &rl.path {
        if !path.starts_with('/') || path.contains('?') || path.contains(' ') {
            return Err(ConfigError::Parse(
                "modules.ratelimit.path must be an absolute path without a query".into(),
            ));
        }
    }
    Ok(())
}

fn listen_is_loopback(listen: &str) -> bool {
    listen
        .parse::<SocketAddr>()
        .map(|addr| addr.ip().is_loopback())
        .unwrap_or(false)
}

/// True when any TCP or HTTP/3 data-plane bind is non-loopback (LA-CAP054-008).
fn metrics_binds_public_listener(servers: &[ServerConfig]) -> bool {
    servers.iter().any(|s| {
        !listen_is_loopback(&s.listen)
            || s.http3_listen
                .as_deref()
                .is_some_and(|h3| !listen_is_loopback(h3))
    })
}

fn validate_metrics_config(
    metrics: &MetricsConfig,
    servers: &[ServerConfig],
) -> Result<(), ConfigError> {
    // Always validate paths so defaults stay coherent even when disabled.
    let reserved = ["/health", "/live", "/ready"];
    for (field, path) in [
        ("path", metrics.path.as_str()),
        ("health_path", metrics.health_path.as_str()),
    ] {
        if path.is_empty() || !path.starts_with('/') {
            return Err(ConfigError::Parse(format!(
                "modules.metrics.{field} must be an absolute path starting with '/'"
            )));
        }
        if path.contains('?') {
            return Err(ConfigError::Parse(format!(
                "modules.metrics.{field} must not contain a query string"
            )));
        }
        if reserved.contains(&path) {
            return Err(ConfigError::Parse(format!(
                "modules.metrics.{field}={path} collides with core liveness/readiness probe; use a distinct path (default health_path is /exyonq-metrics-health)"
            )));
        }
    }
    if metrics.path == metrics.health_path {
        return Err(ConfigError::Parse(
            "modules.metrics.path and modules.metrics.health_path must differ".into(),
        ));
    }
    if let Some(token) = metrics.scrape_bearer_token.as_deref() {
        if token.is_empty() {
            return Err(ConfigError::Parse(
                "modules.metrics.scrape_bearer_token must not be empty when set".into(),
            ));
        }
    }
    // LA-CAP054-008: fail closed — public (non-loopback) metrics scrape requires a bearer.
    if metrics.enabled && metrics_binds_public_listener(servers) {
        match metrics.scrape_bearer_token.as_deref() {
            Some(t) if !t.is_empty() => {}
            _ => {
                return Err(ConfigError::Parse(
                    "modules.metrics.scrape_bearer_token is required when metrics is enabled and any server.listen or server.http3_listen is non-loopback (LA-CAP054-008)".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_route_actions(route: &RouteConfig) -> Result<(), ConfigError> {
    let mut actions = 0u8;
    if route.upstream.is_some() {
        actions += 1;
    }
    if route.root.is_some() {
        actions += 1;
    }
    if route.redirect.is_some() {
        actions += 1;
    }
    if route.rewrite.is_some() {
        actions += 1;
    }
    if route.fastcgi.is_some() {
        actions += 1;
    }
    if actions == 0 {
        return Err(ConfigError::RouteWithoutAction {
            route: route.name.clone(),
        });
    }
    if actions > 1 {
        return Err(ConfigError::RouteConflictingActions {
            route: route.name.clone(),
        });
    }
    Ok(())
}

fn validate_route_cache(
    route: &RouteConfig,
    cache_policies: &HashMap<String, CachePolicyConfig>,
) -> Result<(), ConfigError> {
    let Some(policy_name) = &route.cache else {
        return Ok(());
    };
    if route.redirect.is_some() || route.rewrite.is_some() {
        return Err(ConfigError::CacheInvalidRouteAction {
            route: route.name.clone(),
        });
    }
    let has_root = route.root.is_some();
    let has_upstream = route.upstream.is_some();
    let has_fastcgi = route.fastcgi.is_some();
    if !has_root && !has_upstream && !has_fastcgi {
        return Err(ConfigError::CacheRequiresStaticOrProxyRoute {
            route: route.name.clone(),
        });
    }
    if !cache_policies.contains_key(policy_name) {
        return Err(ConfigError::UnknownCachePolicy {
            route: route.name.clone(),
            policy: policy_name.clone(),
        });
    }
    Ok(())
}

fn validate_cache_policy(policy: &CachePolicyConfig) -> Result<(), ConfigError> {
    if policy.name.trim().is_empty() {
        return Err(ConfigError::Parse(
            "cache_policy name must not be empty".into(),
        ));
    }
    if policy.ttl_seconds == 0 {
        return Err(ConfigError::InvalidCacheTtl {
            policy: policy.name.clone(),
        });
    }
    if policy.max_object_bytes == 0 {
        return Err(ConfigError::InvalidCacheMaxObject {
            policy: policy.name.clone(),
        });
    }
    Ok(())
}

fn validate_full_page_cache(cfg: &FullPageCacheConfig) -> Result<(), ConfigError> {
    if cfg.enabled && cfg.namespace == 0 {
        return Err(ConfigError::Parse(
            "full_page_cache.namespace must be non-zero when enabled".into(),
        ));
    }
    if cfg.enabled && (cfg.max_entries == 0 || cfg.max_total_bytes == 0) {
        return Err(ConfigError::Parse(
            "full_page_cache capacity must be non-zero when enabled".into(),
        ));
    }
    if cfg.enabled && cfg.max_object_bytes == 0 {
        return Err(ConfigError::Parse(
            "full_page_cache.max_object_bytes must be non-zero when enabled".into(),
        ));
    }
    if cfg.enabled && (cfg.default_ttl_seconds == 0 || cfg.max_ttl_seconds == 0) {
        return Err(ConfigError::Parse(
            "full_page_cache TTL bounds must be non-zero when enabled".into(),
        ));
    }
    if cfg.enabled && cfg.default_ttl_seconds > cfg.max_ttl_seconds {
        return Err(ConfigError::Parse(
            "full_page_cache.default_ttl_seconds must be <= max_ttl_seconds".into(),
        ));
    }
    if cfg.enabled && cfg.max_object_bytes > cfg.max_total_bytes {
        return Err(ConfigError::Parse(
            "full_page_cache.max_object_bytes must be <= max_total_bytes".into(),
        ));
    }
    validate_distributed_cache(&cfg.distributed_cache)?;
    Ok(())
}

fn validate_distributed_cache(cfg: &DistributedCacheConfig) -> Result<(), ConfigError> {
    if !cfg.enabled {
        return Ok(());
    }
    if cfg.max_pending_events == 0 || cfg.max_pending_events > 65_536 {
        return Err(ConfigError::Parse(
            "full_page_cache.distributed_cache.max_pending_events must be 1..=65536 when enabled"
                .into(),
        ));
    }
    if cfg.max_event_bytes < 256 || cfg.max_event_bytes > 1024 * 1024 {
        return Err(ConfigError::Parse(
            "full_page_cache.distributed_cache.max_event_bytes must be 256..=1048576 when enabled"
                .into(),
        ));
    }
    if cfg.dedup_capacity == 0 || cfg.dedup_capacity > 1_000_000 {
        return Err(ConfigError::Parse(
            "full_page_cache.distributed_cache.dedup_capacity must be 1..=1000000 when enabled"
                .into(),
        ));
    }
    if cfg.dedup_ttl_ms == 0 {
        return Err(ConfigError::Parse(
            "full_page_cache.distributed_cache.dedup_ttl_ms must be non-zero when enabled".into(),
        ));
    }
    if !cfg.provider.is_empty() && cfg.provider != "local" && cfg.provider != "redis" {
        return Err(ConfigError::Parse(
            "full_page_cache.distributed_cache.provider must be empty, \"local\", or \"redis\""
                .into(),
        ));
    }
    if cfg.provider == "redis" {
        if cfg.redis.endpoint.is_empty() {
            return Err(ConfigError::Parse(
                "full_page_cache.distributed_cache.redis.endpoint required when provider=redis"
                    .into(),
            ));
        }
        // Credentials must not appear in IR — use EXYONQ_REDIS_COORD_PASSWORD.
        if cfg.redis.endpoint.contains('@') {
            return Err(ConfigError::Parse(
                "full_page_cache.distributed_cache.redis.endpoint must not contain userinfo; use EXYONQ_REDIS_COORD_PASSWORD"
                    .into(),
            ));
        }
        if !(cfg.redis.endpoint.starts_with("redis://")
            || cfg.redis.endpoint.starts_with("rediss://"))
        {
            return Err(ConfigError::Parse(
                "full_page_cache.distributed_cache.redis.endpoint must be redis:// or rediss://"
                    .into(),
            ));
        }
        if cfg.redis.stream_maxlen == 0 || cfg.redis.stream_maxlen > 10_000_000 {
            return Err(ConfigError::Parse(
                "full_page_cache.distributed_cache.redis.stream_maxlen must be 1..=10000000".into(),
            ));
        }
        if cfg.redis.namespace.is_empty() || cfg.redis.namespace.len() > 128 {
            return Err(ConfigError::Parse(
                "full_page_cache.distributed_cache.redis.namespace must be 1..=128 bytes".into(),
            ));
        }
        if cfg.redis.connect_timeout_ms == 0 || cfg.redis.command_timeout_ms == 0 {
            return Err(ConfigError::Parse(
                "full_page_cache.distributed_cache.redis timeouts must be non-zero".into(),
            ));
        }
        if cfg.security.replay_window_ms == 0 || cfg.security.max_clock_skew_ms == 0 {
            return Err(ConfigError::Parse(
                "full_page_cache.distributed_cache.security replay_window_ms and max_clock_skew_ms must be non-zero"
                    .into(),
            ));
        }
        // Fail-closed: Redis provider requires HMAC path material (no inline secrets).
        if cfg.security.active_key_id.is_empty() || cfg.security.active_key_file.is_empty() {
            return Err(ConfigError::Parse(
                "full_page_cache.distributed_cache.security.active_key_id and active_key_file required when provider=redis"
                    .into(),
            ));
        }
        if cfg.security.previous_key_file.is_empty() != cfg.security.previous_key_id.is_empty() {
            return Err(ConfigError::Parse(
                "full_page_cache.distributed_cache.security previous_key_id and previous_key_file must be set together"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn validate_htaccess_overlay(
    route: &RouteConfig,
    pools_fcgi: &HashMap<String, FcgiPoolConfig>,
) -> Result<(), ConfigError> {
    if route.htaccess != HtaccessMode::Overlay {
        return Ok(());
    }
    if route.redirect.is_some() {
        return Err(ConfigError::HtaccessConflictsWithRedirect {
            route: route.name.clone(),
        });
    }
    if route.upstream.is_some() {
        return Err(ConfigError::HtaccessConflictsWithUpstream {
            route: route.name.clone(),
        });
    }
    if route.root.is_some() {
        return Ok(());
    }
    if let Some(pool_name) = &route.fastcgi {
        let pool = pools_fcgi
            .get(pool_name)
            .ok_or_else(|| ConfigError::UnknownFcgiPool {
                route: route.name.clone(),
                pool: pool_name.clone(),
            })?;
        if pool
            .document_root
            .as_ref()
            .is_some_and(|root| !root.as_os_str().is_empty())
        {
            return Ok(());
        }
        return Err(ConfigError::HtaccessFcgiPoolMissingDocumentRoot {
            route: route.name.clone(),
            pool: pool_name.clone(),
        });
    }
    Err(ConfigError::HtaccessRequiresDocumentRoot {
        route: route.name.clone(),
    })
}

fn validate_http3_config(http3: &Http3Config) -> Result<(), ConfigError> {
    if let Some(provider) = http3.provider.as_deref() {
        match provider {
            "s2n" | "s2n-quic" | "quiche" => {}
            other => {
                return Err(ConfigError::InvalidHttp3Provider {
                    value: other.to_string(),
                });
            }
        }
    }
    if http3.max_ack_delay_ms > 25_000 {
        return Err(ConfigError::InvalidHttp3MaxAckDelay {
            value: http3.max_ack_delay_ms,
        });
    }
    if http3.request_body_drain_cap_bytes == 0 {
        return Err(ConfigError::InvalidHttp3DrainCap);
    }
    Ok(())
}

fn validate_redirect_status(status: u16) -> Result<(), ConfigError> {
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        Ok(())
    } else {
        Err(ConfigError::InvalidRedirectStatus { value: status })
    }
}

/// Cap036: Location is an opaque configured redirect target (relative or absolute).
/// Reject empty / control bytes (CRLF injection). Absolute `http(s)://` and
/// scheme-relative `//` are intentionally allowed as administrator-configured destinations.
fn validate_redirect_location(location: &str) -> Result<(), ConfigError> {
    let invalid =
        location.is_empty() || location.as_bytes().iter().any(|&b| b <= 0x20 || b == 0x7f);
    if invalid {
        Err(ConfigError::InvalidRedirectLocation {
            value: location.to_string(),
        })
    } else {
        Ok(())
    }
}

fn validate_rewrite_target(target: &str) -> Result<(), ConfigError> {
    // Cap035: absolute path only — no scheme/authority, no query/fragment in replacement.
    // `//…` is rejected so rewrite cannot become protocol-relative / host mutation.
    // Reject SPACE and other bytes that cannot form a valid http::Uri path (LA-CAP035-001/002).
    let invalid = !target.starts_with('/')
        || target.starts_with("//")
        || target.contains('?')
        || target.contains('#')
        || target.as_bytes().iter().any(|&b| b <= 0x20 || b == 0x7f)
        || target.parse::<hyper::Uri>().is_err();
    if invalid {
        Err(ConfigError::InvalidRewriteTarget {
            value: target.to_string(),
        })
    } else {
        Ok(())
    }
}

fn unique_names<'a, I>(names: I, kind: &'static str) -> Result<HashSet<String>, ConfigError>
where
    I: Iterator<Item = &'a str>,
{
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name.to_string()) {
            return Err(ConfigError::DuplicateName {
                kind,
                name: name.to_string(),
            });
        }
    }
    Ok(seen)
}

fn validate_listen(listen: &str) -> Result<(), ConfigError> {
    parse_listen(listen).map(|_| ())
}

fn parse_listen(listen: &str) -> Result<SocketAddr, ConfigError> {
    listen.parse().map_err(|_| ConfigError::InvalidListen {
        value: listen.to_string(),
    })
}

fn validate_match_path(path: &str) -> Result<(), ConfigError> {
    if !path.starts_with('/') {
        return Err(ConfigError::InvalidMatchPath {
            value: path.to_string(),
        });
    }
    Ok(())
}

fn validate_tls_acme(tls: &TlsConfig) -> Result<(), ConfigError> {
    if let Some(acme) = &tls.acme {
        if !acme.enabled {
            return Ok(());
        }
        if acme.email.trim().is_empty() {
            return Err(ConfigError::Parse(
                "tls.acme.email is required when acme is enabled".into(),
            ));
        }
        if acme.domains.is_empty() {
            return Err(ConfigError::Parse(
                "tls.acme.domains must not be empty when acme is enabled".into(),
            ));
        }
    }
    Ok(())
}

fn validate_fcgi_pool_address(pool: &str, address: &str) -> Result<(), ConfigError> {
    if address.trim().is_empty() {
        return Err(ConfigError::EmptyFcgiPoolAddress {
            pool: pool.to_string(),
        });
    }
    Ok(())
}

fn validate_static_preload(preload: &StaticPreloadConfig) -> Result<(), ConfigError> {
    let file = preload.max_file_bytes;
    let total = preload.max_total_bytes;
    let entries = preload.max_entries;

    if file != 0 && file > PRELOAD_MAX_FILE_BYTES_CEILING {
        return Err(ConfigError::InvalidStaticPreload {
            message: format!(
                "static.preload.max_file_bytes must be 0 or 1..={PRELOAD_MAX_FILE_BYTES_CEILING}, got {file}"
            ),
        });
    }
    if total != 0 && total > PRELOAD_MAX_TOTAL_BYTES_CEILING {
        return Err(ConfigError::InvalidStaticPreload {
            message: format!(
                "static.preload.max_total_bytes must be 0 or 1..={PRELOAD_MAX_TOTAL_BYTES_CEILING}, got {total}"
            ),
        });
    }
    if entries != 0 && entries > PRELOAD_MAX_ENTRIES_CEILING {
        return Err(ConfigError::InvalidStaticPreload {
            message: format!(
                "static.preload.max_entries must be 0 or 1..={PRELOAD_MAX_ENTRIES_CEILING}, got {entries}"
            ),
        });
    }
    // When preload is enabled, total must be able to hold at least one max-sized file.
    if !preload.is_disabled() && total < file {
        return Err(ConfigError::InvalidStaticPreload {
            message: format!(
                "static.preload.max_total_bytes ({total}) must be >= max_file_bytes ({file})"
            ),
        });
    }
    Ok(())
}

fn validate_static_encoding_cache(cfg: &StaticEncodingCacheConfig) -> Result<(), ConfigError> {
    if cfg.level > ENCODING_CACHE_LEVEL_CEILING {
        return Err(ConfigError::InvalidStaticEncodingCache {
            message: format!(
                "static.encoding_cache.level must be 0..={ENCODING_CACHE_LEVEL_CEILING}, got {}",
                cfg.level
            ),
        });
    }
    if cfg.max_bytes == 0 || cfg.max_bytes > ENCODING_CACHE_MAX_BYTES_CEILING {
        return Err(ConfigError::InvalidStaticEncodingCache {
            message: format!(
                "static.encoding_cache.max_bytes must be 1..={ENCODING_CACHE_MAX_BYTES_CEILING}, got {}",
                cfg.max_bytes
            ),
        });
    }
    if cfg.min_bytes > cfg.max_bytes {
        return Err(ConfigError::InvalidStaticEncodingCache {
            message: format!(
                "static.encoding_cache.min_bytes ({}) must be <= max_bytes ({})",
                cfg.min_bytes, cfg.max_bytes
            ),
        });
    }
    if cfg.max_entries == 0 || cfg.max_entries > ENCODING_CACHE_MAX_ENTRIES_CEILING {
        return Err(ConfigError::InvalidStaticEncodingCache {
            message: format!(
                "static.encoding_cache.max_entries must be 1..={ENCODING_CACHE_MAX_ENTRIES_CEILING}, got {}",
                cfg.max_entries
            ),
        });
    }
    if cfg.max_total_bytes == 0 || cfg.max_total_bytes > ENCODING_CACHE_MAX_TOTAL_BYTES_CEILING {
        return Err(ConfigError::InvalidStaticEncodingCache {
            message: format!(
                "static.encoding_cache.max_total_bytes must be 1..={ENCODING_CACHE_MAX_TOTAL_BYTES_CEILING}, got {}",
                cfg.max_total_bytes
            ),
        });
    }
    if cfg.cache_dir.as_os_str().is_empty() {
        return Err(ConfigError::InvalidStaticEncodingCache {
            message: "static.encoding_cache.cache_dir must not be empty".into(),
        });
    }
    // Refuse relative `..` components so IR cannot point the managed cache outside
    // an operator-intended tree via traversal tokens (absolute paths remain OK).
    if cfg
        .cache_dir
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ConfigError::InvalidStaticEncodingCache {
            message: "static.encoding_cache.cache_dir must not contain '..'".into(),
        });
    }
    if cfg.enabled && !cfg.gzip && !cfg.brotli {
        return Err(ConfigError::InvalidStaticEncodingCache {
            message: "static.encoding_cache.enabled requires gzip and/or brotli = true".into(),
        });
    }
    Ok(())
}

fn validate_fcgi_max_concurrency(pool: &str, value: u32) -> Result<(), ConfigError> {
    if (1..=MAX_FCGI_MAX_CONCURRENCY).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::InvalidFcgiMaxConcurrency {
            pool: pool.to_string(),
            value,
        })
    }
}

fn validate_fcgi_max_connections(
    pool: &str,
    max_concurrency: u32,
    max_connections: Option<u32>,
) -> Result<(), ConfigError> {
    let Some(value) = max_connections else {
        return Ok(());
    };
    if value >= 1 && value <= max_concurrency {
        Ok(())
    } else {
        Err(ConfigError::InvalidFcgiMaxConnections {
            pool: pool.to_string(),
            value,
            max_concurrency,
        })
    }
}

fn validate_fcgi_transport(pool: &str, transport: &str) -> Result<(), ConfigError> {
    match transport.trim().to_ascii_lowercase().as_str() {
        "unix" | "tcp" | "auto" => Ok(()),
        other => Err(ConfigError::InvalidFcgiTransport {
            pool: pool.to_string(),
            detail: format!("expected unix|tcp|auto, got {other}"),
        }),
    }
}

fn validate_fcgi_idle_timeout_ms(pool: &str, value: u64) -> Result<(), ConfigError> {
    if (1..=3_600_000).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::InvalidFcgiIdleTimeout {
            pool: pool.to_string(),
            value,
        })
    }
}

fn validate_fcgi_total_timeout_ms(pool: &str, value: u64) -> Result<(), ConfigError> {
    if (1..=3_600_000).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::InvalidFcgiTotalTimeout {
            pool: pool.to_string(),
            value,
        })
    }
}

fn validate_fcgi_checkout_timeout_ms(pool: &str, value: u64) -> Result<(), ConfigError> {
    if (1..=600_000).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::InvalidFcgiCheckoutTimeout {
            pool: pool.to_string(),
            value,
        })
    }
}

fn validate_upstream_target(target: &str) -> Result<(), ConfigError> {
    let uri: hyper::Uri = target
        .parse()
        .map_err(|_| ConfigError::InvalidUpstreamTarget {
            value: target.to_string(),
        })?;
    if uri.scheme_str() != Some("http") {
        return Err(ConfigError::InvalidUpstreamTarget {
            value: target.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
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
"#;

    #[test]
    fn parses_valid_config() {
        let config: AppConfig = VALID.parse().expect("valid config");
        assert_eq!(config.config_version, 1);
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.upstreams.len(), 1);
    }

    #[test]
    fn rejects_unsupported_version() {
        let input = VALID.replace("config_version = 1", "config_version = 99");
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::UnsupportedVersion { found: 99, .. }
        ));
    }

    #[test]
    fn rejects_conflicting_route_actions() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["bad"]

[[route]]
name = "bad"
match = { path = "/bad" }
upstream = "backend"
root = "/tmp"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:9000"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::RouteConflictingActions { .. }));
    }

    #[test]
    fn accepts_static_route_with_cache_policy() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["assets"]

[[cache_policy]]
name = "default"
ttl_seconds = 30

[[route]]
name = "assets"
match = { path = "/assets" }
root = "/srv/static"
cache = "default"
"#;
        let config: AppConfig = input.parse().expect("cache config");
        assert_eq!(config.cache_policies.len(), 1);
        assert_eq!(config.routes[0].cache.as_deref(), Some("default"));
    }

    #[test]
    fn rejects_cache_on_non_static_route() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]

[[cache_policy]]
name = "default"

[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"
cache = "default"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:9000"
"#;
        let config: AppConfig = input.parse().expect("proxy cache config");
        assert_eq!(config.routes[0].cache.as_deref(), Some("default"));
        assert_eq!(config.routes[0].upstream.as_deref(), Some("backend"));
    }

    #[test]
    fn accepts_cache_with_fastcgi() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[cache_policy]]
name = "default"
ttl_seconds = 10
max_object_bytes = 1048576

[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"
cache = "default"

[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
"#;
        let config: AppConfig = input.parse().expect("fastcgi cache config");
        assert_eq!(config.routes[0].cache.as_deref(), Some("default"));
        assert_eq!(config.routes[0].fastcgi.as_deref(), Some("php"));
    }

    #[test]
    fn rejects_cache_without_backend() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["nope"]

[[cache_policy]]
name = "default"

[[route]]
name = "nope"
match = { path = "/nope" }
cache = "default"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::CacheRequiresStaticOrProxyRoute { .. }
        ));
    }

    #[test]
    fn rejects_cache_on_redirect_route() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["redir"]

[[cache_policy]]
name = "default"

[[route]]
name = "redir"
match = { path = "/old" }
cache = "default"
redirect = { status = 301, location = "/new" }
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::CacheInvalidRouteAction { .. }));
    }

    #[test]
    fn rejects_redirect_location_with_crlf() {
        let input = "config_version = 1\n\
[[server]]\n\
listen = \"127.0.0.1:8080\"\n\
routes = [\"redir\"]\n\
\n\
[[route]]\n\
name = \"redir\"\n\
match = { path = \"/old\" }\n\
redirect = { status = 302, location = \"/ok\\r\\nSet-Cookie: x=1\" }\n";
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidRedirectLocation { .. }));
    }

    #[test]
    fn rejects_empty_redirect_location() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["redir"]

[[route]]
name = "redir"
match = { path = "/old" }
redirect = { status = 302, location = "" }
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidRedirectLocation { .. }));
    }

    #[test]
    fn accepts_absolute_and_scheme_relative_redirect_location() {
        for loc in [
            "https://example.com/out",
            "http://example.com/out",
            "//cdn.example/path",
            "/rel?q=1",
            "/rel#frag",
        ] {
            let input = format!(
                r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["redir"]

[[route]]
name = "redir"
match = {{ path = "/old" }}
redirect = {{ status = 302, location = "{loc}" }}
"#
            );
            input
                .parse::<AppConfig>()
                .unwrap_or_else(|e| panic!("location {loc:?} should parse: {e}"));
        }
    }

    #[test]
    fn accepts_structural_fcgi_pool_and_fastcgi_route() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[route]]
name = "php"
match = { path = "/index.php" }
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
"#;
        let config: AppConfig = input.parse().expect("structural fcgi config");
        assert_eq!(config.pools_fcgi.len(), 1);
        assert_eq!(config.routes[0].fastcgi.as_deref(), Some("php"));
    }

    #[test]
    fn rejects_invalid_fcgi_max_concurrency() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[route]]
name = "php"
match = { path = "/index.php" }
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
max_concurrency = 0
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidFcgiMaxConcurrency { .. }));
    }

    #[test]
    fn accepts_fcgi_max_concurrency_default() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[route]]
name = "php"
match = { path = "/index.php" }
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
"#;
        let config: AppConfig = input.parse().expect("fcgi pool default concurrency");
        assert_eq!(
            config.pools_fcgi.get("php").expect("php").max_concurrency,
            DEFAULT_FCGI_MAX_CONCURRENCY
        );
    }

    #[test]
    fn accepts_fcgi_pool_document_root() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
document_root = "/srv/php"
"#;
        let config: AppConfig = input.parse().expect("fcgi pool document_root");
        assert_eq!(
            config
                .pools_fcgi
                .get("php")
                .expect("php")
                .document_root
                .as_ref()
                .map(|p| p.display().to_string()),
            Some("/srv/php".into())
        );
    }

    #[test]
    fn fcgi_pool_document_root_toml_roundtrip() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
document_root = "/srv/php"
max_concurrency = 16
"#;
        let config: AppConfig = input.parse().expect("parse");
        let pool = config.pools_fcgi.get("php").expect("php");
        assert_eq!(
            pool.document_root.as_ref().map(|p| p.display().to_string()),
            Some("/srv/php".into())
        );
        let canonical = crate::fingerprint(&config);
        let reparsed: AppConfig = input.parse().expect("roundtrip");
        assert_eq!(crate::fingerprint(&reparsed), canonical);
    }

    #[test]
    fn rejects_unknown_fcgi_pool_reference() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]

[[route]]
name = "php"
match = { path = "/index.php" }
fastcgi = "missing"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::UnknownFcgiPool { .. }));
    }

    #[test]
    fn golden_fixtures_parse() {
        for name in ["minimal.toml", "static.toml", "modules.toml"] {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures")
                .join(name);
            AppConfig::from_file(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn golden_fingerprints() {
        let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/golden/ir-fingerprints.json");
        let raw = std::fs::read_to_string(&golden_path).expect("golden file");
        let map: HashMap<String, String> = serde_json::from_str(&raw).unwrap();
        let mut mismatches = Vec::new();
        for (name, expected) in &map {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures")
                .join(name);
            let config = AppConfig::from_file(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            let got = fingerprint(&config);
            if got.as_str() != expected {
                mismatches.push(format!("{name} {}", got.as_str()));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    #[test]
    fn static_plus_overlay_valid() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "/srv/site"
htaccess = "overlay"
"#;
        let config: AppConfig = input.parse().expect("static overlay");
        assert_eq!(config.routes[0].htaccess, HtaccessMode::Overlay);
    }

    #[test]
    fn fastcgi_plus_overlay_valid_with_pool_document_root() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"
htaccess = "overlay"
[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
document_root = "/srv/php"
"#;
        input.parse::<AppConfig>().expect("fcgi overlay");
    }

    #[test]
    fn root_plus_fastcgi_still_invalid() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["bad"]
[[route]]
name = "bad"
match = { path = "/" }
root = "/srv"
fastcgi = "php"
[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
document_root = "/srv/php"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::RouteConflictingActions { .. }));
    }

    #[test]
    fn upstream_plus_overlay_invalid() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"
htaccess = "overlay"
[[upstream]]
name = "backend"
target = "http://127.0.0.1:9000"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::HtaccessConflictsWithUpstream { .. }
        ));
    }

    #[test]
    fn redirect_plus_overlay_invalid() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
redirect = { status = 301, location = "/x" }
htaccess = "overlay"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::HtaccessConflictsWithRedirect { .. }
        ));
    }

    #[test]
    fn fastcgi_overlay_without_document_root_invalid() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"
htaccess = "overlay"
[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::HtaccessFcgiPoolMissingDocumentRoot { .. }
        ));
    }

    #[test]
    fn htaccess_overlay_toml_json_roundtrip() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"
htaccess = "overlay"
[[fcgi_pool]]
name = "php"
address = "/run/php.sock"
document_root = "/srv/php"
"#;
        let config: AppConfig = input.parse().expect("parse");
        let json = canonical_json(&config);
        assert!(json.contains("\"htaccess\":\"overlay\""));
        let reparsed: AppConfig = input.parse().expect("roundtrip");
        assert_eq!(reparsed.routes[0].htaccess, HtaccessMode::Overlay);
    }

    #[test]
    fn static_preload_defaults_applied_when_absent() {
        let config: AppConfig = VALID.parse().expect("valid");
        assert_eq!(
            config.static_section.preload.max_file_bytes,
            DEFAULT_PRELOAD_MAX_FILE_BYTES
        );
        assert_eq!(
            config.static_section.preload.max_total_bytes,
            DEFAULT_PRELOAD_TOTAL_BYTES
        );
        assert_eq!(
            config.static_section.preload.max_entries,
            DEFAULT_PRELOAD_MAX_ENTRIES
        );
        assert!(!config.static_section.preload.is_disabled());
    }

    #[test]
    fn static_preload_zero_disables() {
        let input = r#"
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

[static.preload]
max_file_bytes = 0
max_total_bytes = 268435456
max_entries = 4096
"#;
        let config: AppConfig = input.parse().expect("zero file disables");
        assert!(config.static_section.preload.is_disabled());
    }

    #[test]
    fn static_preload_total_less_than_file_rejected() {
        let input = r#"
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

[static.preload]
max_file_bytes = 33554432
max_total_bytes = 1048576
max_entries = 4096
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidStaticPreload { .. }));
    }

    #[test]
    fn static_preload_above_ceiling_rejected() {
        let input = r#"
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

[static.preload]
max_file_bytes = 536870912
max_total_bytes = 1073741824
max_entries = 4096
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidStaticPreload { .. }));
    }

    #[test]
    fn static_preload_explicit_values_accepted() {
        let input = r#"
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

[static.preload]
max_file_bytes = 1048576
max_total_bytes = 4194304
max_entries = 16
"#;
        let config: AppConfig = input.parse().expect("explicit");
        assert_eq!(config.static_section.preload.max_file_bytes, 1_048_576);
        assert_eq!(config.static_section.preload.max_total_bytes, 4_194_304);
        assert_eq!(config.static_section.preload.max_entries, 16);
    }

    #[test]
    fn static_encoding_cache_defaults_off() {
        let input = r#"
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
"#;
        let config: AppConfig = input.parse().expect("defaults");
        assert!(!config.static_section.encoding_cache.enabled);
        assert!(config.static_section.encoding_cache.gzip);
        assert!(!config.static_section.encoding_cache.brotli);
        assert_eq!(
            config.static_section.encoding_cache.level,
            DEFAULT_ENCODING_CACHE_LEVEL
        );
        assert_eq!(
            config.static_section.encoding_cache.min_bytes,
            DEFAULT_ENCODING_CACHE_MIN_BYTES
        );
        assert_eq!(
            config.static_section.encoding_cache.max_bytes,
            DEFAULT_ENCODING_CACHE_MAX_BYTES
        );
        assert_eq!(
            config.static_section.encoding_cache.max_entries,
            DEFAULT_ENCODING_CACHE_MAX_ENTRIES
        );
        assert_eq!(
            config.static_section.encoding_cache.max_total_bytes,
            DEFAULT_ENCODING_CACHE_MAX_TOTAL_BYTES
        );
    }

    #[test]
    fn static_encoding_cache_enabled_accepted() {
        let input = r#"
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

[static.encoding_cache]
enabled = true
cache_dir = "/tmp/exyonq-static-encoding"
level = 6
gzip = true
brotli = false
"#;
        let config: AppConfig = input.parse().expect("encoding_cache");
        assert!(config.static_section.encoding_cache.enabled);
        assert_eq!(
            config.static_section.encoding_cache.cache_dir.as_os_str(),
            "/tmp/exyonq-static-encoding"
        );
    }

    #[test]
    fn static_encoding_cache_enabled_without_codings_rejected() {
        let input = r#"
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

[static.encoding_cache]
enabled = true
gzip = false
brotli = false
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidStaticEncodingCache { .. }
        ));
    }

    #[test]
    fn logging_section_parses_and_defaults() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/site" }
root = "/tmp"

[logging]
format = "json"
level = "debug"

[logging.file]
enabled = true
path = "/tmp/exyonq-test.log"

[logging.file.rotation]
enabled = true
max_bytes = 1048576
keep = 3
"#;
        let config: AppConfig = input.parse().expect("logging ir");
        assert_eq!(config.logging.format, crate::LogFormat::Json);
        assert_eq!(config.logging.level, "debug");
        assert!(config.logging.file.enabled);
        assert_eq!(
            config.logging.file.path.as_deref(),
            Some("/tmp/exyonq-test.log")
        );
        assert!(config.logging.file.rotation.enabled);
        assert_eq!(config.logging.file.rotation.keep, 3);
    }

    #[test]
    fn logging_unknown_field_rejected() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/site" }
root = "/tmp"
[logging]
format = "text"
not_a_real_field = true
"#;
        assert!(input.parse::<AppConfig>().is_err());
    }

    #[test]
    fn distributed_cache_redis_rejects_userinfo_in_endpoint() {
        let input = r#"
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

[full_page_cache]
enabled = true
namespace = 4

[full_page_cache.distributed_cache]
enabled = true
provider = "redis"
invalidation_enabled = true
generation_enabled = true

[full_page_cache.distributed_cache.redis]
endpoint = "redis://:secret@127.0.0.1:6379/"

[full_page_cache.distributed_cache.security]
active_key_id = "k1"
active_key_file = "/run/secrets/l2_hmac_active"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("userinfo") || msg.contains("EXYONQ_REDIS_COORD_PASSWORD"),
            "unexpected err: {msg}"
        );
    }

    #[test]
    fn distributed_cache_redis_requires_hmac_security_paths() {
        let input = r#"
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

[full_page_cache]
enabled = true
namespace = 4

[full_page_cache.distributed_cache]
enabled = true
provider = "redis"
invalidation_enabled = true
generation_enabled = true

[full_page_cache.distributed_cache.redis]
endpoint = "redis://127.0.0.1:6379/"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(
            err.to_string().contains("active_key_file")
                || err.to_string().contains("active_key_id"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn distributed_cache_security_requires_key_file_not_inline() {
        let input = r#"
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

[full_page_cache]
enabled = true
namespace = 4

[full_page_cache.distributed_cache]
enabled = true
provider = "redis"
invalidation_enabled = true
generation_enabled = true

[full_page_cache.distributed_cache.redis]
endpoint = "redis://127.0.0.1:6379/"

[full_page_cache.distributed_cache.security]
active_key_id = "k1"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(
            err.to_string().contains("active_key_file"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn distributed_cache_security_paths_accepted() {
        let input = r#"
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

[full_page_cache]
enabled = true
namespace = 4

[full_page_cache.distributed_cache]
enabled = true
provider = "redis"
invalidation_enabled = true
generation_enabled = true
dedup_capacity = 2048

[full_page_cache.distributed_cache.redis]
endpoint = "redis://127.0.0.1:6379/"
namespace = "exyonq:fpc:v1:prod"
stream_maxlen = 10000
connect_timeout_ms = 500
command_timeout_ms = 500

[full_page_cache.distributed_cache.security]
active_key_id = "k2"
active_key_file = "/run/secrets/l2_hmac_active"
previous_key_id = "k1"
previous_key_file = "/run/secrets/l2_hmac_previous"
max_clock_skew_ms = 60000
replay_window_ms = 300000
"#;
        let cfg: AppConfig = input.parse().expect("security paths ok");
        assert_eq!(
            cfg.full_page_cache
                .distributed_cache
                .security
                .active_key_file,
            "/run/secrets/l2_hmac_active"
        );
        assert_eq!(
            cfg.full_page_cache
                .distributed_cache
                .security
                .replay_window_ms,
            300_000
        );
    }

    #[test]
    fn http3_provider_s2n_and_quiche_ok() {
        for provider in ["s2n", "quiche"] {
            let input = format!(
                r#"
config_version = 1
[http3]
provider = "{provider}"
max_ack_delay_ms = 1
request_body_drain_cap_bytes = 65536
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = {{ path = "/" }}
root = "/tmp"
"#
            );
            let cfg: AppConfig = input.parse().expect(provider);
            assert_eq!(cfg.http3.provider.as_deref(), Some(provider));
            assert_eq!(cfg.http3.max_ack_delay_ms, 1);
        }
    }

    #[test]
    fn http3_invalid_provider_rejected() {
        let input = r#"
config_version = 1
[http3]
provider = "nginx"
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(err.to_string().contains("http3.provider"), "{err}");
    }

    #[test]
    fn http3_quinn_not_public_ir() {
        let input = r#"
config_version = 1
[http3]
provider = "quinn-legacy"
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#;
        assert!(input.parse::<AppConfig>().is_err());
    }

    #[test]
    fn http3_max_ack_delay_validation() {
        let input = r#"
config_version = 1
[http3]
provider = "s2n"
max_ack_delay_ms = 30000
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#;
        assert!(input.parse::<AppConfig>().is_err());
    }

    #[test]
    fn http3_missing_provider_defaults_none() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#;
        let cfg: AppConfig = input.parse().unwrap();
        assert_eq!(cfg.http3.provider, None);
        assert!(cfg.http3.enabled);
    }

    #[test]
    fn timeout_ms_ceiling_rejects_overflow() {
        let err = UpstreamConfig::try_from_parts(
            "u".into(),
            Some("http://127.0.0.1:9".into()),
            Vec::new(),
            EndpointSelectionPolicy::default(),
            EndpointFailoverPolicy::default(),
            MAX_UPSTREAM_TIMEOUT_MS + 1,
            1,
            UpstreamHealthCheckConfig::default(),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("exceeds max"), "{msg}");
    }

    #[test]
    fn timeout_ms_zero_and_max_ok() {
        for ms in [0u64, MAX_UPSTREAM_TIMEOUT_MS] {
            UpstreamConfig::try_from_parts(
                "u".into(),
                Some("http://127.0.0.1:9".into()),
                Vec::new(),
                EndpointSelectionPolicy::default(),
                EndpointFailoverPolicy::default(),
                ms,
                1,
                UpstreamHealthCheckConfig::default(),
            )
            .expect("bound ok");
        }
    }

    /// LA-CAP047-001: unknown fields must fail closed on the serve/parse_str path
    /// (not only on merge RawRoot).
    #[test]
    fn unknown_fields_rejected_on_parse_str() {
        let base = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:1"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/api/" }
upstream = "u"
[[upstream]]
name = "u"
[[upstream.endpoints]]
address = "127.0.0.1"
port = 9
"#;
        let top = format!("totally_unknown_typo = true\n{base}");
        let err = AppConfig::parse_str(&top).unwrap_err().to_string();
        assert!(
            err.contains("unknown field") || err.contains("totally_unknown_typo"),
            "{err}"
        );

        let nested = base.replace("routes = [\"r\"]", "routes = [\"r\"]\nbogus_server_key = 1");
        let err = AppConfig::parse_str(&nested).unwrap_err().to_string();
        assert!(
            err.contains("unknown field") || err.contains("bogus_server_key"),
            "{err}"
        );
    }

    #[test]
    fn la_cap054_008_metrics_non_loopback_requires_scrape_bearer() {
        let toml = r#"
config_version = 1
[[server]]
listen = "0.0.0.0:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
[modules.metrics]
enabled = true
"#;
        let err = AppConfig::parse_str(toml).unwrap_err().to_string();
        assert!(
            err.contains("scrape_bearer_token") && err.contains("LA-CAP054-008"),
            "{err}"
        );
    }

    #[test]
    fn la_cap054_008_metrics_loopback_ok_without_scrape_bearer() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
[modules.metrics]
enabled = true
"#;
        AppConfig::parse_str(toml).expect("loopback metrics without bearer");
    }

    #[test]
    fn la_cap054_008_metrics_non_loopback_ok_with_scrape_bearer() {
        let toml = r#"
config_version = 1
[[server]]
listen = "0.0.0.0:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
[modules.metrics]
enabled = true
scrape_bearer_token = "phase1-la008-token"
"#;
        AppConfig::parse_str(toml).expect("non-loopback with bearer");
    }

    #[test]
    fn la_cap054_008_metrics_public_http3_listen_requires_scrape_bearer() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
http3_listen = "0.0.0.0:8443"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
[modules.metrics]
enabled = true
"#;
        let err = AppConfig::parse_str(toml).unwrap_err().to_string();
        assert!(
            err.contains("scrape_bearer_token") && err.contains("LA-CAP054-008"),
            "{err}"
        );
    }
}
