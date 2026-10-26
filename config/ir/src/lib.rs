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
pub mod modules;
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
    FpcBypassConfig, FpcKeyConfig, FullPageCacheConfig,
};
pub use logging::{
    validate_logging, AccessLoggingConfig, AuditLoggingConfig, ConsoleLoggingConfig, ConsoleStream,
    FileLoggingConfig, FileRotationConfig, JournaldLoggingConfig, LogFormat, LoggingConfig,
    OtelLoggingConfig, OtlpProtocol, SyslogLoggingConfig,
};
pub use modules::{CompressionConfig, MetricsConfig, ModulesConfig, RateLimitConfig, RealIpConfig};
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
    /// NG-05 / ADR-051: top-level exact equivalence tables (`[[map]]`).
    pub maps: HashMap<String, MapConfig>,
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
    /// NG-04 / ADR-050: status → absolute URI path (custom error body; status preserved).
    /// TOML keys are strings (`"404"`) for stable serde.
    #[serde(default)]
    pub error_pages: HashMap<String, String>,
    /// NG-06 / F3: PROXY protocol v1/v2 for real client IP behind LBs.
    /// If enabled, the listener expects a PROXY header before any TLS/HTTP bytes.
    #[serde(default)]
    pub proxy_protocol: Option<ProxyProtocolConfig>,
}

impl ServerConfig {
    pub fn server_name_list(&self) -> Vec<String> {
        self.server_name.to_vec()
    }
}

/// NG-06 / F3: PROXY protocol configuration.
/// If enabled, the listener expects a PROXY v1/v2 header before any TLS/HTTP bytes.
/// If `trusted` is set, only connections from those CIDRs may send PROXY headers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyProtocolConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Trusted proxy CIDRs. If empty, accept PROXY from any peer (unsafe).
    /// Example: `trusted = ["10.0.0.0/8", "192.168.0.0/16"]`
    #[serde(default)]
    pub trusted: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
    #[serde(default)]
    pub acme: Option<AcmeConfig>,
    /// AP-02: reject handshakes whose SNI matches no `server_name` on the
    /// listener (`SSLStrictSNIVHostCheck` equivalent). Only the first block's
    /// value applies — it governs the shared socket.
    #[serde(default)]
    pub strict_sni: bool,
    /// NG-12: enabled TLS protocol versions (`TLSv1.2`, `TLSv1.3`). Empty =
    /// rustls default (both). Only the first block's value applies.
    #[serde(default)]
    pub protocols: Vec<String>,
    /// NG-12: enabled cipher suites (IANA/rustls or OpenSSL AEAD names, or
    /// `:`-separated list). Empty = rustls default. First block only.
    #[serde(default)]
    pub ciphers: Vec<String>,
    /// NG-12: enabled ECDHE curves (`X25519`, `secp256r1`/`prime256v1`/`P-256`,
    /// `secp384r1`/`P-384`, or `:`-separated). Empty = rustls default. First block only.
    #[serde(default)]
    pub curves: Vec<String>,
    /// NG-13: DER-encoded OCSP response stapled to this certificate (nginx
    /// `ssl_stapling_file` equivalent — ExyonQ does not fetch from the
    /// responder; point this at a file refreshed by your PKI tooling).
    /// Per-`[[server]]` block, like `cert`/`key`.
    #[serde(default)]
    pub ocsp_stapling: Option<PathBuf>,
    /// NG-13: require `ocsp_stapling` for this block (fail-closed validation
    /// for must-staple certificates). Rejected without `ocsp_stapling`.
    #[serde(default)]
    pub must_staple: bool,
}

/// Automatic certificate management (ACME v2; HTTP-01 default, DNS-01 opt-in).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcmeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub email: String,
    pub domains: Vec<String>,
    #[serde(default)]
    pub staging: bool,
    /// Q-09: `"http-01"` (default) or `"dns-01"`. DNS-01 is required for
    /// wildcard domains.
    #[serde(default)]
    pub challenge: Option<String>,
    /// Q-09: command invoked to publish/remove the DNS-01 TXT record when
    /// `challenge = "dns-01"`. Environment: `EXYONQ_ACME_ACTION` =
    /// `present`|`cleanup`, `EXYONQ_ACME_DOMAIN` (record owner is
    /// `_acme-challenge.<EXYONQ_ACME_DOMAIN>`, never a `*.` prefix),
    /// `EXYONQ_ACME_TOKEN`, `EXYONQ_ACME_TXT_VALUE`. `present` must ADD the
    /// TXT value to the RRset (not replace it): wildcard + apex share one
    /// `_acme-challenge` name.
    #[serde(default)]
    pub dns_hook: Option<String>,
    /// Q-09: seconds to wait after the hook publishes TXT before telling the
    /// CA to validate (default 15). Only valid with `challenge = "dns-01"`.
    #[serde(default)]
    pub dns_propagation_secs: Option<u64>,
    /// Path to the persisted ACME account credentials (JSON). Defaults to
    /// `<cert>.acme-account.json` next to `tls.cert`.
    #[serde(default)]
    pub account_path: Option<PathBuf>,
}

impl AcmeConfig {
    /// Effective challenge type (`"http-01"` unless configured otherwise).
    pub fn challenge_type(&self) -> &str {
        self.challenge.as_deref().unwrap_or("http-01")
    }
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
    /// Cap035 literal path or NG-02 patterned rewrite (ADR-048).
    #[serde(default)]
    pub rewrite: Option<RewriteConfig>,
    #[serde(default)]
    pub fastcgi: Option<String>,
    #[serde(default)]
    pub htaccess: HtaccessMode,
    #[serde(default)]
    pub cache: Option<String>,
    /// NG-16: invalidation tags for entries served by this route
    /// (`exyonqctl purge tag <site_id> <tag>`).
    #[serde(default)]
    pub cache_tags: Vec<String>,
    /// NG-03 / ADR-049: after static miss under `root`, rematch once to this path.
    #[serde(default)]
    pub fallback_rewrite: Option<String>,
    /// Probe request URI as a file before fallback (default true when fallback set).
    #[serde(default)]
    pub try_uri: Option<bool>,
    /// Probe URI as directory (+ index) before fallback (default false when fallback set).
    #[serde(default)]
    pub try_uri_dir: Option<bool>,
    /// NG-05 / ADR-051: outbound upstream header replacements (templates + allowlisted vars).
    #[serde(default)]
    pub proxy_set_headers: Vec<ProxySetHeaderConfig>,
    /// F2 / ADR-061: outbound Cache-Control by response MIME (ExpiresByType adelanto).
    #[serde(default)]
    pub cache_headers: Option<CacheHeadersConfig>,
    /// ADR-062 / F1-R19: outbound client response headers (route-scoped `add_header` / `Header set`).
    #[serde(default)]
    pub response_headers: Vec<ResponseHeaderConfig>,
    /// F3-R20: response `Location`/`Content-Location`/`URI` prefix rewrite
    /// (`ProxyPassReverse` / `proxy_redirect`).
    #[serde(default)]
    pub location_rewrite: Vec<LocationRewriteConfig>,
}

/// F3-R20: one response-location rewrite rule (`ProxyPassReverse` /
/// `proxy_redirect`). When an upstream response `Location`,
/// `Content-Location` or `URI` header starts with `from` (scheme+authority
/// compared case-insensitively, path case-sensitively), that prefix is
/// replaced by `to`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocationRewriteConfig {
    /// Upstream URL prefix as seen by the backend (e.g. `http://backend/`).
    pub from: String,
    /// Public prefix the client must see (`/app` path or absolute URL).
    pub to: String,
}

/// ADR-062 / F1-R19: one outbound client response header.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseHeaderConfig {
    pub name: String,
    pub value: String,
    /// When false (default): apply on 2xx/3xx only. When true: all statuses.
    #[serde(default)]
    pub always: bool,
}

/// F2 / ADR-061: MIME → Cache-Control max-age map on a route.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheHeadersConfig {
    #[serde(default)]
    pub by_mime: std::collections::BTreeMap<String, MimeCacheHeader>,
}

/// Per-MIME cache header policy.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MimeCacheHeader {
    pub max_age: u64,
    #[serde(default)]
    pub immutable: bool,
}

impl CacheHeadersConfig {
    pub const MAX_AGE_LIMIT: u64 = 315_360_000; // ~10 years

    /// Lookup exact mime then `"*"`. `content_type` may include `; charset=…`.
    pub fn lookup(&self, content_type: &str) -> Option<&MimeCacheHeader> {
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or(content_type)
            .trim();
        if mime.is_empty() {
            return None;
        }
        self.by_mime.get(mime).or_else(|| self.by_mime.get("*"))
    }

    /// `Cache-Control` value for a content-type, if configured.
    pub fn cache_control_for(&self, content_type: &str) -> Option<String> {
        let h = self.lookup(content_type)?;
        let mut v = format!("public, max-age={}", h.max_age);
        if h.immutable {
            v.push_str(", immutable");
        }
        Some(v)
    }
}

/// NG-05 / ADR-051: one outbound upstream header set.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxySetHeaderConfig {
    pub name: String,
    pub value: String,
}

/// NG-05 / ADR-051: top-level exact `map` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapConfig {
    pub name: String,
    pub input: MapInputKind,
    /// Required when `input = "header"`.
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub values: HashMap<String, String>,
}

/// Map input source (ADR-051).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapInputKind {
    Header,
    Host,
    RemoteAddr,
    RequestUri,
    Scheme,
}

/// Cap035 literal or NG-02 patterned rewrite (ADR-048).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum RewriteConfig {
    /// Absolute path only; rematch once (`last` semantics).
    Literal(String),
    Rule(RewriteRuleConfig),
}

impl RewriteConfig {
    pub fn literal(path: impl Into<String>) -> Self {
        Self::Literal(path.into())
    }

    pub fn literal_path(&self) -> Option<&str> {
        match self {
            Self::Literal(p) => Some(p.as_str()),
            Self::Rule(_) => None,
        }
    }

    pub fn as_rule(&self) -> Option<&RewriteRuleConfig> {
        match self {
            Self::Rule(r) => Some(r),
            Self::Literal(_) => None,
        }
    }
}

impl From<&str> for RewriteConfig {
    fn from(value: &str) -> Self {
        Self::Literal(value.to_string())
    }
}

impl From<String> for RewriteConfig {
    fn from(value: String) -> Self {
        Self::Literal(value)
    }
}

/// Patterned rewrite rule (ADR-048 / NG-02).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewriteRuleConfig {
    pub pattern: String,
    pub replacement: String,
    #[serde(default)]
    pub flag: RewriteFlagConfig,
    #[serde(default)]
    pub when: Vec<RewriteCondConfig>,
}

/// Rewrite flags aligned with nginx (`last`/`break`/`redirect`/`permanent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RewriteFlagConfig {
    #[default]
    Last,
    Break,
    Redirect,
    Permanent,
}

/// Minimal RewriteCond equivalents (AND). ADR-048.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RewriteCondConfig {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub header_value: Option<String>,
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

/// Maximum compiled `match.regex` pattern length (bytes). ADR-047 / NG-01.
pub const MAX_ROUTE_REGEX_BYTES: usize = 256;

/// Which exclusive match arm is active (ADR-047 / NG-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteMatchKind {
    Prefix,
    Suffix,
    Regex,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RouteMatch {
    /// Directory-prefix path (Cap033). Empty when `suffix` or `regex` is set.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub host: Option<String>,
    /// Literal path suffix (`path.ends_with`). XOR with `path` / `regex`.
    #[serde(default)]
    pub suffix: Option<String>,
    /// Rust `regex` pattern (case-sensitive). XOR with `path` / `suffix`. Compiled at plan build.
    #[serde(default)]
    pub regex: Option<String>,
    /// NG-05 / ADR-051: optional request-header AND filter (not a XOR arm).
    #[serde(default)]
    pub header: Option<String>,
    /// Exact value when set; if omitted with `header`, require present non-empty.
    #[serde(default)]
    pub header_value: Option<String>,
}

impl RouteMatch {
    /// Prefix match helper for tests and programmatic builders.
    pub fn prefix(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            ..Default::default()
        }
    }

    /// Suffix match helper.
    pub fn with_suffix(suffix: impl Into<String>) -> Self {
        Self {
            suffix: Some(suffix.into()),
            ..Default::default()
        }
    }

    /// Regex match helper.
    pub fn with_regex(regex: impl Into<String>) -> Self {
        Self {
            regex: Some(regex.into()),
            ..Default::default()
        }
    }

    /// Active exclusive kind, if exactly one arm is set.
    pub fn kind(&self) -> Option<RouteMatchKind> {
        let has_path = !self.path.is_empty();
        let has_suffix = self.suffix.as_ref().is_some_and(|s| !s.is_empty());
        let has_regex = self.regex.as_ref().is_some_and(|s| !s.is_empty());
        match (has_path, has_suffix, has_regex) {
            (true, false, false) => Some(RouteMatchKind::Prefix),
            (false, true, false) => Some(RouteMatchKind::Suffix),
            (false, false, true) => Some(RouteMatchKind::Regex),
            _ => None,
        }
    }

    /// Stable identity key for duplicate detection (pattern string of the active arm).
    pub fn pattern_key(&self) -> Option<&str> {
        match self.kind()? {
            RouteMatchKind::Prefix => Some(self.path.trim_end_matches('/')),
            RouteMatchKind::Suffix => self.suffix.as_deref(),
            RouteMatchKind::Regex => self.regex.as_deref(),
        }
    }

    /// Ranking length for Cap033+NG-01 (longer wins within kind).
    pub fn pattern_len(&self) -> usize {
        match self.kind() {
            Some(RouteMatchKind::Prefix) => self.path.len(),
            Some(RouteMatchKind::Suffix) => self.suffix.as_ref().map_or(0, String::len),
            Some(RouteMatchKind::Regex) => self.regex.as_ref().map_or(0, String::len),
            None => 0,
        }
    }

    /// kind_rank: regex/suffix (3) > prefix (1) > catch-all `/` (0). ADR-047.
    pub fn kind_rank(&self) -> u8 {
        match self.kind() {
            Some(RouteMatchKind::Regex) | Some(RouteMatchKind::Suffix) => 3,
            Some(RouteMatchKind::Prefix) if self.path == "/" => 0,
            Some(RouteMatchKind::Prefix) => 1,
            None => 0,
        }
    }
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
    /// NG-07: cookie name minted/read by `CookieAffinity`. Normalized to
    /// [`DEFAULT_STICKY_COOKIE`] when that policy is configured without a name.
    pub sticky_cookie: Option<String>,
    /// NG-07: request header name whose value keys `HeaderAffinity`
    /// (required for that policy).
    pub sticky_header: Option<String>,
    /// NG-09: periodic DNS re-resolve interval for hostname endpoints.
    /// 0 = off (nginx parity: no `resolver` configured → no periodic refresh).
    /// Per-dial resolution already picks up IP changes for new connections;
    /// this adds scheduled re-resolution + `exyonq_upstream_dns_*` observability.
    pub dns_refresh_ms: u64,
}

/// NG-09: upper bound for `dns_refresh_ms` (24h).
pub const MAX_DNS_REFRESH_MS: u64 = 86_400_000;

/// NG-07: default route-cookie name for `cookie_affinity`.
pub const DEFAULT_STICKY_COOKIE: &str = "EXYONQ_ROUTE";

/// RFC 9110 token check for header names and cookie names used as affinity keys.
fn is_http_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
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
            None,
            None,
            0,
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
        sticky_cookie: Option<String>,
        sticky_header: Option<String>,
        dns_refresh_ms: u64,
    ) -> Result<Self, ConfigError> {
        if dns_refresh_ms > MAX_DNS_REFRESH_MS {
            return Err(ConfigError::Parse(format!(
                "upstream `{name}`: dns_refresh_ms={dns_refresh_ms} exceeds max {MAX_DNS_REFRESH_MS}"
            )));
        }
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
        // NG-07: header affinity is meaningless without a key source — fail
        // closed at config load (same class as other upstream parse errors).
        let sticky_header = match sticky_header {
            Some(h)
                if h.trim().is_empty()
                    && selection_policy != EndpointSelectionPolicy::HeaderAffinity =>
            {
                None
            }
            Some(h) => {
                let h = h.trim();
                if !is_http_token(h) {
                    return Err(ConfigError::Parse(format!(
                        "upstream `{name}`: sticky_header `{h}` is not a valid HTTP token"
                    )));
                }
                Some(h.to_string())
            }
            None => None,
        };
        if matches!(selection_policy, EndpointSelectionPolicy::HeaderAffinity)
            && sticky_header.is_none()
        {
            return Err(ConfigError::Parse(format!(
                "upstream `{name}`: selection_policy=header_affinity requires non-empty sticky_header"
            )));
        }
        let sticky_cookie = match sticky_cookie {
            Some(c) if c.trim().is_empty() => {
                return Err(ConfigError::Parse(format!(
                    "upstream `{name}`: sticky_cookie must be non-empty when set"
                )))
            }
            Some(c) => {
                let c = c.trim();
                if !is_http_token(c) {
                    return Err(ConfigError::Parse(format!(
                        "upstream `{name}`: sticky_cookie `{c}` is not a valid HTTP token"
                    )));
                }
                Some(c.to_string())
            }
            None if matches!(selection_policy, EndpointSelectionPolicy::CookieAffinity) => {
                Some(DEFAULT_STICKY_COOKIE.to_string())
            }
            None => None,
        };
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
            sticky_cookie,
            sticky_header,
            dns_refresh_ms,
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
    #[serde(default)]
    sticky_cookie: Option<String>,
    #[serde(default)]
    sticky_header: Option<String>,
    /// NG-09: 0 = periodic re-resolve disabled.
    #[serde(default)]
    dns_refresh_ms: u64,
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
            raw.sticky_cookie,
            raw.sticky_header,
            raw.dns_refresh_ms,
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
    pub map: Vec<MapConfig>,
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
    map: Vec<MapConfig>,
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
            map: raw.map,
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

        if raw.map.len() > MAX_MAPS {
            return Err(ConfigError::Parse(format!(
                "at most {MAX_MAPS} [[map]] tables allowed (ADR-051)"
            )));
        }
        let mut maps = HashMap::new();
        for map in raw.map {
            validate_map_config(&map)?;
            let name = map.name.clone();
            if maps.insert(name.clone(), map).is_some() {
                return Err(ConfigError::DuplicateName { kind: "map", name });
            }
        }

        for server in &raw.server {
            validate_listen(&server.listen)?;
            validate_error_pages(&server.error_pages)?;
            for route_name in &server.routes {
                if !route_names.contains(route_name) {
                    return Err(ConfigError::UnknownRoute {
                        route: route_name.clone(),
                    });
                }
            }
            if let Some(tls) = &server.tls {
                validate_tls_acme(tls)?;
                if tls.strict_sni && server.server_name_list().is_empty() {
                    return Err(ConfigError::Parse(
                        "tls.strict_sni requires at least one server_name \
                         (otherwise every TLS handshake is rejected)"
                            .into(),
                    ));
                }
            }
        }
        validate_shared_listeners(&raw.server)?;

        for route in &routes {
            validate_route_match(&route.r#match)?;
            validate_route_cache(route, &cache_policies)?;
            validate_route_cache_headers(route)?;
            validate_route_actions(route)?;
            validate_htaccess_overlay(route, &pools_fcgi)?;
            validate_proxy_set_headers(route, &maps)?;
            validate_response_headers(route)?;
            validate_location_rewrites(route)?;
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
                validate_rewrite_config(rewrite)?;
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
        raw.modules.real_ip.validate()?;
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
            maps,
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

    /// Every distinct `listen` address, primary first. Servers sharing one
    /// address are SNI/certificate variants of the same listener (NG-24);
    /// a server with a *different* address gets its own socket.
    pub fn listen_addrs(&self) -> Result<Vec<SocketAddr>, ConfigError> {
        let mut addrs: Vec<SocketAddr> = Vec::new();
        for server in &self.servers {
            let addr = parse_listen(&server.listen)?;
            if !addrs.contains(&addr) {
                addrs.push(addr);
            }
        }
        if addrs.is_empty() {
            return Err(ConfigError::MissingServers);
        }
        Ok(addrs)
    }

    /// `true` when any server bound to `addr` terminates TLS. A listener
    /// address whose servers are all plaintext stays plaintext (HTTP-01
    /// challenge listener, redirect-only :80).
    pub fn listen_uses_tls(&self, addr: SocketAddr) -> bool {
        self.servers.iter().any(|server| {
            server.tls.is_some() && parse_listen(&server.listen).is_ok_and(|a| a == addr)
        })
    }

    /// Migrate v1 IR to v2 (adds empty includes, preserves semantics).
    pub fn migrate_v1_to_v2(mut self) -> Self {
        if self.config_version == CONFIG_VERSION_V1 {
            self.config_version = CONFIG_VERSION_V2;
        }
        self
    }
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

fn validate_route_cache_headers(route: &RouteConfig) -> Result<(), ConfigError> {
    let Some(ch) = &route.cache_headers else {
        return Ok(());
    };
    if ch.by_mime.is_empty() {
        return Err(ConfigError::Parse(format!(
            "route `{}`: cache_headers.by_mime must not be empty when cache_headers is set",
            route.name
        )));
    }
    for (mime, hdr) in &ch.by_mime {
        if mime.is_empty() {
            return Err(ConfigError::Parse(format!(
                "route `{}`: cache_headers.by_mime key must not be empty",
                route.name
            )));
        }
        if hdr.max_age == 0 || hdr.max_age > CacheHeadersConfig::MAX_AGE_LIMIT {
            return Err(ConfigError::Parse(format!(
                "route `{}`: cache_headers.by_mime[`{mime}`].max_age must be 1..={}, got {}",
                route.name,
                CacheHeadersConfig::MAX_AGE_LIMIT,
                hdr.max_age
            )));
        }
    }
    Ok(())
}

fn validate_route_actions(route: &RouteConfig) -> Result<(), ConfigError> {
    // ADR-048: `flag = break` co-locates with exactly one of root|upstream|fastcgi
    // (nginx "stay in location"). Other rewrite forms remain exclusive terminals.
    let break_colocated = matches!(
        route.rewrite.as_ref().and_then(RewriteConfig::as_rule),
        Some(rule) if matches!(rule.flag, RewriteFlagConfig::Break)
    );

    validate_fallback_rewrite(route)?;

    let mut terminals = 0u8;
    if route.upstream.is_some() {
        terminals += 1;
    }
    if route.root.is_some() {
        terminals += 1;
    }
    if route.redirect.is_some() {
        terminals += 1;
    }
    if route.fastcgi.is_some() {
        terminals += 1;
    }

    if break_colocated {
        if route.redirect.is_some() || terminals != 1 {
            return Err(ConfigError::RouteConflictingActions {
                route: route.name.clone(),
            });
        }
        return Ok(());
    }

    let mut actions = terminals;
    if route.rewrite.is_some() {
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

/// NG-03 / ADR-049: `fallback_rewrite` is a root modifier, not a 5th exclusive terminal.
/// NG-04 / ADR-050: validate server `error_pages` map.
fn validate_error_pages(pages: &HashMap<String, String>) -> Result<(), ConfigError> {
    for (key, path) in pages {
        let status: u16 = key.parse().map_err(|_| {
            ConfigError::Parse(format!(
                "error_pages key `{key}` must be an HTTP status integer"
            ))
        })?;
        if !(400..=599).contains(&status) {
            return Err(ConfigError::Parse(format!(
                "error_pages status {status} must be in 400..=599"
            )));
        }
        if path.starts_with('@')
            || path.starts_with("http://")
            || path.starts_with("https://")
            || path.starts_with("//")
        {
            return Err(ConfigError::Parse(format!(
                "error_pages target `{path}` must be an absolute path (no @named / URL redirect)"
            )));
        }
        validate_rewrite_target(path)?;
    }
    Ok(())
}

fn validate_fallback_rewrite(route: &RouteConfig) -> Result<(), ConfigError> {
    let has_flags = route.try_uri.is_some() || route.try_uri_dir.is_some();
    let Some(target) = route.fallback_rewrite.as_deref() else {
        if has_flags {
            return Err(ConfigError::Parse(format!(
                "route `{}`: try_uri/try_uri_dir require fallback_rewrite",
                route.name
            )));
        }
        return Ok(());
    };

    if route.root.is_none() {
        return Err(ConfigError::Parse(format!(
            "route `{}`: fallback_rewrite requires root",
            route.name
        )));
    }
    if route.rewrite.is_some()
        || route.redirect.is_some()
        || route.upstream.is_some()
        || route.fastcgi.is_some()
    {
        return Err(ConfigError::RouteConflictingActions {
            route: route.name.clone(),
        });
    }
    if route.htaccess == HtaccessMode::Overlay {
        return Err(ConfigError::Parse(format!(
            "route `{}`: fallback_rewrite cannot combine with htaccess=overlay (ADR-049)",
            route.name
        )));
    }
    validate_rewrite_target(target)?;

    let try_uri = route.try_uri.unwrap_or(true);
    let try_uri_dir = route.try_uri_dir.unwrap_or(false);
    if !try_uri && !try_uri_dir {
        return Err(ConfigError::Parse(format!(
            "route `{}`: fallback_rewrite requires try_uri and/or try_uri_dir",
            route.name
        )));
    }
    Ok(())
}

fn validate_route_cache(
    route: &RouteConfig,
    cache_policies: &HashMap<String, CachePolicyConfig>,
) -> Result<(), ConfigError> {
    // NG-16: tag names must be usable as purge selectors.
    for tag in &route.cache_tags {
        if tag.trim().is_empty()
            || tag.contains(|c: char| c.is_whitespace() || c == ',' || c == ';')
        {
            return Err(ConfigError::Parse(format!(
                "route {:?} has invalid cache_tags entry {tag:?}",
                route.name
            )));
        }
    }
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
    if cfg.enabled {
        cfg.validate().map_err(ConfigError::Parse)?;
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

fn validate_rewrite_config(rewrite: &RewriteConfig) -> Result<(), ConfigError> {
    match rewrite {
        RewriteConfig::Literal(target) => validate_rewrite_target(target),
        RewriteConfig::Rule(rule) => validate_rewrite_rule(rule),
    }
}

fn validate_rewrite_rule(rule: &RewriteRuleConfig) -> Result<(), ConfigError> {
    if rule.pattern.is_empty() {
        return Err(ConfigError::InvalidRouteRegex {
            detail: "rewrite.pattern empty".into(),
        });
    }
    if rule.pattern.len() > MAX_ROUTE_REGEX_BYTES {
        return Err(ConfigError::RouteRegexTooLong {
            limit: MAX_ROUTE_REGEX_BYTES,
            found: rule.pattern.len(),
        });
    }
    regex::Regex::new(&rule.pattern).map_err(|e| ConfigError::InvalidRouteRegex {
        detail: format!("rewrite.pattern: {e}"),
    })?;
    validate_rewrite_replacement_template(&rule.replacement)?;
    // Static expansion check for literal replacements (no captures).
    if !rule.replacement.contains('$') {
        match rule.flag {
            RewriteFlagConfig::Last | RewriteFlagConfig::Break => {
                validate_rewrite_target(&rule.replacement)?;
            }
            RewriteFlagConfig::Redirect | RewriteFlagConfig::Permanent => {
                validate_redirect_location(&rule.replacement)?;
            }
        }
    }
    for cond in &rule.when {
        validate_rewrite_cond(cond)?;
    }
    Ok(())
}

fn validate_rewrite_replacement_template(template: &str) -> Result<(), ConfigError> {
    // ADR-048 + ADR-062 (R07): `$1`..`$9`, `$$`, `$uri`, `$host`.
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            if i + 1 >= bytes.len() {
                return Err(ConfigError::InvalidRewriteTarget {
                    value: template.to_string(),
                });
            }
            let n = bytes[i + 1];
            if n == b'$' {
                i += 2;
                continue;
            }
            if (b'1'..=b'9').contains(&n) {
                i += 2;
                continue;
            }
            // Named vars: $uri / $host
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j == start {
                return Err(ConfigError::InvalidRewriteTarget {
                    value: template.to_string(),
                });
            }
            let name = std::str::from_utf8(&bytes[start..j]).unwrap_or("");
            if !matches!(name, "uri" | "host") {
                return Err(ConfigError::InvalidRewriteTarget {
                    value: template.to_string(),
                });
            }
            i = j;
            continue;
        }
        i += 1;
    }
    Ok(())
}

fn validate_rewrite_cond(cond: &RewriteCondConfig) -> Result<(), ConfigError> {
    let set = [
        cond.path.is_some(),
        cond.host.is_some(),
        cond.query.is_some(),
        cond.header.is_some(),
    ]
    .iter()
    .filter(|&&x| x)
    .count();
    if set == 0 {
        return Err(ConfigError::Parse(
            "rewrite.when entry must set path, host, query, or header".into(),
        ));
    }
    if cond.header_value.is_some() && cond.header.is_none() {
        return Err(ConfigError::Parse(
            "rewrite.when.header_value requires header".into(),
        ));
    }
    if let Some(path) = &cond.path {
        if path.len() > MAX_ROUTE_REGEX_BYTES {
            return Err(ConfigError::RouteRegexTooLong {
                limit: MAX_ROUTE_REGEX_BYTES,
                found: path.len(),
            });
        }
        regex::Regex::new(path).map_err(|e| ConfigError::InvalidRouteRegex {
            detail: format!("rewrite.when.path: {e}"),
        })?;
    }
    if let Some(query) = &cond.query {
        if query.len() > MAX_ROUTE_REGEX_BYTES {
            return Err(ConfigError::RouteRegexTooLong {
                limit: MAX_ROUTE_REGEX_BYTES,
                found: query.len(),
            });
        }
        regex::Regex::new(query).map_err(|e| ConfigError::InvalidRouteRegex {
            detail: format!("rewrite.when.query: {e}"),
        })?;
    }
    Ok(())
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

fn validate_route_match(m: &RouteMatch) -> Result<(), ConfigError> {
    let Some(kind) = m.kind() else {
        return Err(ConfigError::InvalidRouteMatchKind);
    };
    match kind {
        RouteMatchKind::Prefix => validate_match_path(&m.path)?,
        RouteMatchKind::Suffix => {
            let suffix = m.suffix.as_deref().unwrap_or("");
            if suffix.is_empty() {
                return Err(ConfigError::InvalidMatchSuffix);
            }
        }
        RouteMatchKind::Regex => {
            let pattern = m.regex.as_deref().unwrap_or("");
            if pattern.is_empty() {
                return Err(ConfigError::InvalidRouteRegex {
                    detail: "empty pattern".into(),
                });
            }
            if pattern.len() > MAX_ROUTE_REGEX_BYTES {
                return Err(ConfigError::RouteRegexTooLong {
                    limit: MAX_ROUTE_REGEX_BYTES,
                    found: pattern.len(),
                });
            }
            regex::Regex::new(pattern).map_err(|e| ConfigError::InvalidRouteRegex {
                detail: e.to_string(),
            })?;
        }
    }
    // NG-05 / ADR-051: optional header AND filter.
    if let Some(name) = m.header.as_deref() {
        if name.trim().is_empty() {
            return Err(ConfigError::Parse(
                "match.header must be a non-empty header name".into(),
            ));
        }
        if name
            .as_bytes()
            .iter()
            .any(|&b| b <= 0x20 || b == 0x7f || b == b':')
        {
            return Err(ConfigError::Parse(format!(
                "match.header `{name}` is not a valid HTTP header name"
            )));
        }
    } else if m.header_value.is_some() {
        return Err(ConfigError::Parse(
            "match.header_value requires match.header".into(),
        ));
    }
    Ok(())
}

const MAX_MAPS: usize = 32;
const MAX_MAP_ENTRIES: usize = 64;
const MAX_MAP_KEY_BYTES: usize = 256;

fn validate_map_config(map: &MapConfig) -> Result<(), ConfigError> {
    if !is_map_name_ok(&map.name) {
        return Err(ConfigError::Parse(format!(
            "map name `{}` must match [a-z][a-z0-9_]* (ADR-051)",
            map.name
        )));
    }
    if map.values.len() > MAX_MAP_ENTRIES {
        return Err(ConfigError::Parse(format!(
            "map `{}`: at most {MAX_MAP_ENTRIES} values (ADR-051)",
            map.name
        )));
    }
    for (k, v) in &map.values {
        if k.len() > MAX_MAP_KEY_BYTES || v.len() > MAX_MAP_KEY_BYTES {
            return Err(ConfigError::Parse(format!(
                "map `{}`: key/value exceeds {MAX_MAP_KEY_BYTES} bytes",
                map.name
            )));
        }
    }
    if map.default.len() > MAX_MAP_KEY_BYTES {
        return Err(ConfigError::Parse(format!(
            "map `{}`: default exceeds {MAX_MAP_KEY_BYTES} bytes",
            map.name
        )));
    }
    match map.input {
        MapInputKind::Header => {
            let Some(h) = map.header.as_deref() else {
                return Err(ConfigError::Parse(format!(
                    "map `{}`: input=header requires header field",
                    map.name
                )));
            };
            if h.trim().is_empty() {
                return Err(ConfigError::Parse(format!(
                    "map `{}`: header name empty",
                    map.name
                )));
            }
        }
        _ => {
            if map.header.is_some() {
                return Err(ConfigError::Parse(format!(
                    "map `{}`: header field only valid when input=header",
                    map.name
                )));
            }
        }
    }
    Ok(())
}

fn is_map_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn validate_proxy_set_headers(
    route: &RouteConfig,
    maps: &HashMap<String, MapConfig>,
) -> Result<(), ConfigError> {
    if route.proxy_set_headers.is_empty() {
        return Ok(());
    }
    if route.upstream.is_none() {
        return Err(ConfigError::Parse(format!(
            "route `{}`: proxy_set_headers requires upstream (ADR-051)",
            route.name
        )));
    }
    for h in &route.proxy_set_headers {
        if h.name.trim().is_empty()
            || h.name
                .as_bytes()
                .iter()
                .any(|&b| b <= 0x20 || b == 0x7f || b == b':')
        {
            return Err(ConfigError::Parse(format!(
                "route `{}`: invalid proxy_set_headers name `{}`",
                route.name, h.name
            )));
        }
        if is_hop_by_hop_denied(&h.name) {
            return Err(ConfigError::Parse(format!(
                "route `{}`: proxy_set_headers cannot set hop-by-hop `{}`",
                route.name, h.name
            )));
        }
        validate_header_template(&h.value, maps)?;
    }
    Ok(())
}

/// F3-R20: compile-time checks for `[[route.location_rewrite]]`.
fn validate_location_rewrites(route: &RouteConfig) -> Result<(), ConfigError> {
    if route.location_rewrite.is_empty() {
        return Ok(());
    }
    if route.upstream.is_none() {
        return Err(ConfigError::Parse(format!(
            "route `{}`: location_rewrite requires upstream (F3-R20)",
            route.name
        )));
    }
    for r in &route.location_rewrite {
        for (what, v) in [("from", r.from.as_str()), ("to", r.to.as_str())] {
            if v.is_empty() || v.bytes().any(|b| b <= 0x20 || b == 0x7f) || v.contains('$') {
                return Err(ConfigError::Parse(format!(
                    "route `{}`: invalid location_rewrite {what} `{v}`",
                    route.name
                )));
            }
        }
        if !(r.from.starts_with("http://") || r.from.starts_with("https://")) {
            return Err(ConfigError::Parse(format!(
                "route `{}`: location_rewrite from must be an absolute http(s) URL (`{}`)",
                route.name, r.from
            )));
        }
        if !r.to.starts_with('/') && !(r.to.starts_with("http://") || r.to.starts_with("https://"))
        {
            return Err(ConfigError::Parse(format!(
                "route `{}`: location_rewrite to must be a `/` path or absolute http(s) URL (`{}`)",
                route.name, r.to
            )));
        }
    }
    Ok(())
}

/// ADR-062: compile-time checks for `[[route.response_headers]]`.
fn validate_response_headers(route: &RouteConfig) -> Result<(), ConfigError> {
    if route.response_headers.is_empty() {
        return Ok(());
    }
    let mut seen: HashMap<String, ()> = HashMap::new();
    for h in &route.response_headers {
        if h.name.trim().is_empty()
            || h.name
                .as_bytes()
                .iter()
                .any(|&b| b <= 0x20 || b == 0x7f || b == b':')
        {
            return Err(ConfigError::Parse(format!(
                "route `{}`: invalid response_headers name `{}`",
                route.name, h.name
            )));
        }
        let key = h.name.to_ascii_lowercase();
        if seen.insert(key.clone(), ()).is_some() {
            return Err(ConfigError::Parse(format!(
                "route `{}`: duplicate response_headers name `{}` (ADR-062)",
                route.name, h.name
            )));
        }
        if is_response_header_denied(&h.name) {
            return Err(ConfigError::Parse(format!(
                "route `{}`: response_headers cannot set `{}` (ADR-062 denylist)",
                route.name, h.name
            )));
        }
        validate_response_header_template(&h.value)?;
    }
    Ok(())
}

fn is_response_header_denied(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "cache-control" || lower == "set-cookie" {
        return true;
    }
    if lower.starts_with("proxy-") {
        return true;
    }
    matches!(
        lower.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// ADR-062 response template: literals + `$host` + `$uri` only.
pub fn validate_response_header_template(template: &str) -> Result<(), ConfigError> {
    let mut rest = template;
    while let Some(idx) = rest.find('$') {
        rest = &rest[idx + 1..];
        if rest.starts_with('$') {
            rest = &rest[1..];
            continue;
        }
        let name_len = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .count();
        if name_len == 0 {
            return Err(ConfigError::Parse(format!(
                "invalid `$` in response_headers template `{template}` (ADR-062)"
            )));
        }
        let name = &rest[..name_len];
        rest = &rest[name_len..];
        if !matches!(name, "host" | "uri") {
            return Err(ConfigError::Parse(format!(
                "unknown variable `${name}` in response_headers (ADR-062 allowlist: $host, $uri)"
            )));
        }
    }
    Ok(())
}

/// Runtime context for ADR-062 response header templates.
#[derive(Debug, Clone, Copy)]
pub struct ResponseHeaderExpandCtx<'a> {
    pub host: &'a str,
    pub uri: &'a str,
}

/// Expand ADR-062 response header template (`$host` / `$uri` / `$$`).
pub fn expand_response_header_template(
    template: &str,
    ctx: &ResponseHeaderExpandCtx<'_>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(idx) = rest.find('$') {
        out.push_str(&rest[..idx]);
        rest = &rest[idx + 1..];
        if rest.starts_with('$') {
            out.push('$');
            rest = &rest[1..];
            continue;
        }
        let name_len = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .count();
        let name = &rest[..name_len];
        rest = &rest[name_len..];
        match name {
            "host" => out.push_str(ctx.host),
            "uri" => out.push_str(ctx.uri),
            _ => {}
        }
    }
    out.push_str(rest);
    out
}

fn is_hop_by_hop_denied(name: &str) -> bool {
    // `Connection` allowed when driven by `[[map]]` (ADR-051 WebSocket-adjacent).
    matches!(
        name.to_ascii_lowercase().as_str(),
        "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// Compile-time check that a header value template only uses allowlisted vars / maps.
pub fn validate_header_template(
    template: &str,
    maps: &HashMap<String, MapConfig>,
) -> Result<(), ConfigError> {
    let mut rest = template;
    while let Some(idx) = rest.find('$') {
        rest = &rest[idx + 1..];
        if rest.starts_with('$') {
            rest = &rest[1..];
            continue;
        }
        let name_len = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .count();
        if name_len == 0 {
            return Err(ConfigError::Parse(format!(
                "invalid `$` in header template `{template}`"
            )));
        }
        let name = &rest[..name_len];
        rest = &rest[name_len..];
        if matches!(name, "host" | "remote_addr" | "request_uri" | "scheme") {
            continue;
        }
        if !maps.contains_key(name) {
            return Err(ConfigError::Parse(format!(
                "unknown variable `${name}` in header template (ADR-051 allowlist / [[map]])"
            )));
        }
    }
    Ok(())
}

/// Runtime expansion context for ADR-051 templates.
#[derive(Debug, Clone, Copy)]
pub struct HeaderExpandCtx<'a> {
    pub host: &'a str,
    pub remote_addr: &'a str,
    pub request_uri: &'a str,
    pub scheme: &'a str,
    pub request_headers: &'a [(String, String)],
    pub maps: &'a HashMap<String, MapConfig>,
}

/// Note from header template expansion (Q-02 / ADR-054 — unknown `$map` only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapExpandNote {
    /// `$name` referenced but no `[[map]]` with that name.
    UnknownMap { name: String },
}

/// Expand an allowlisted header template (literals + `$host` / `$remote_addr` / … / `$map`).
pub fn expand_header_template(template: &str, ctx: &HeaderExpandCtx<'_>) -> String {
    expand_header_template_with_notes(template, ctx).0
}

/// Like [`expand_header_template`], also collecting unknown-map notes (no success-path key-miss spam).
pub fn expand_header_template_with_notes(
    template: &str,
    ctx: &HeaderExpandCtx<'_>,
) -> (String, Vec<MapExpandNote>) {
    let mut out = String::with_capacity(template.len());
    let mut notes = Vec::new();
    let mut rest = template;
    while let Some(idx) = rest.find('$') {
        out.push_str(&rest[..idx]);
        rest = &rest[idx + 1..];
        if rest.starts_with('$') {
            out.push('$');
            rest = &rest[1..];
            continue;
        }
        let name_len = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .count();
        let name = &rest[..name_len];
        rest = &rest[name_len..];
        let value = match name {
            "host" => ctx.host,
            "remote_addr" => ctx.remote_addr,
            "request_uri" => ctx.request_uri,
            "scheme" => ctx.scheme,
            other => resolve_map_var(other, ctx, &mut notes),
        };
        out.push_str(value);
    }
    out.push_str(rest);
    (out, notes)
}

fn resolve_map_var<'a>(
    name: &str,
    ctx: &HeaderExpandCtx<'a>,
    notes: &mut Vec<MapExpandNote>,
) -> &'a str {
    let Some(map) = ctx.maps.get(name) else {
        notes.push(MapExpandNote::UnknownMap {
            name: name.to_string(),
        });
        return "";
    };
    let input = match map.input {
        MapInputKind::Host => ctx.host,
        MapInputKind::RemoteAddr => ctx.remote_addr,
        MapInputKind::RequestUri => ctx.request_uri,
        MapInputKind::Scheme => ctx.scheme,
        MapInputKind::Header => {
            let want = map.header.as_deref().unwrap_or("");
            ctx.request_headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(want))
                .map(|(_, v)| v.as_str())
                .unwrap_or("")
        }
    };
    if input.is_empty() {
        return map.default.as_str();
    }
    map.values
        .get(input)
        .map(String::as_str)
        .unwrap_or(map.default.as_str())
}

/// Whether a route's optional `match.header` AND constraint is satisfied.
pub fn match_header_satisfied(m: &RouteMatch, request_headers: &[(String, String)]) -> bool {
    let Some(want_name) = m.header.as_deref() else {
        return true;
    };
    let Some((_, value)) = request_headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(want_name))
    else {
        return false;
    };
    match m.header_value.as_deref() {
        None => !value.is_empty(),
        Some(expected) => value == expected,
    }
}

/// NG-15: a `server_name` must be a usable DNS name or `*.suffix` wildcard —
/// anything else lands in the SNI map but can never match a handshake.
fn validate_server_name(name: &str, server_idx: usize) -> Result<(), ConfigError> {
    let body = name.strip_prefix("*.").unwrap_or(name);
    let valid = !body.is_empty()
        && body.len() <= 253
        && body.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if !valid || (name.contains('*') && !name.starts_with("*.")) {
        return Err(ConfigError::Parse(format!(
            "server[{server_idx}] has invalid server_name {name:?}; expected a DNS name or *.suffix wildcard"
        )));
    }
    Ok(())
}

/// NG-15: `[[server]]` blocks that share a `listen` collapse into one socket.
/// Enforce the invariants that keep every block meaningful — otherwise the
/// runtime would silently ignore configured servers/certificates.
fn validate_shared_listeners(servers: &[ServerConfig]) -> Result<(), ConfigError> {
    if servers.iter().skip(1).any(|s| s.http3_listen.is_some()) {
        return Err(ConfigError::Parse(
            "http3_listen is only honored on the first [[server]] block".into(),
        ));
    }
    if servers.iter().skip(1).any(|s| {
        s.tls
            .as_ref()
            .and_then(|t| t.acme.as_ref())
            .is_some_and(|a| a.enabled)
    }) {
        return Err(ConfigError::Parse(
            "tls.acme is only honored on the first [[server]] block".into(),
        ));
    }
    if servers
        .iter()
        .skip(1)
        .any(|s| s.tls.as_ref().is_some_and(|t| t.strict_sni))
    {
        return Err(ConfigError::Parse(
            "tls.strict_sni is only honored on the first [[server]] block".into(),
        ));
    }
    for (field, check) in [
        (
            "tls.protocols",
            (|t: &TlsConfig| !t.protocols.is_empty()) as fn(&TlsConfig) -> bool,
        ),
        ("tls.ciphers", |t| !t.ciphers.is_empty()),
        ("tls.curves", |t| !t.curves.is_empty()),
    ] {
        if servers
            .iter()
            .skip(1)
            .any(|s| s.tls.as_ref().is_some_and(check))
        {
            return Err(ConfigError::Parse(format!(
                "{field} is only honored on the first [[server]] block"
            )));
        }
    }
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, server) in servers.iter().enumerate() {
        for name in server.server_name_list() {
            validate_server_name(&name, idx)?;
        }
        groups.entry(server.listen.as_str()).or_default().push(idx);
    }
    for (listen, idxs) in groups {
        if idxs.len() < 2 {
            continue;
        }
        let first = &servers[idxs[0]];
        let mut names = std::collections::HashSet::new();
        for &idx in &idxs {
            let server = &servers[idx];
            if idx != idxs[0] && server.server_name.is_empty() {
                return Err(ConfigError::Parse(format!(
                    "server[{idx}] shares listen {listen} with server[{}] but declares no \
                     server_name; only the first block on a shared listen may omit it",
                    idxs[0]
                )));
            }
            if server.tls.is_some() != first.tls.is_some() {
                return Err(ConfigError::Parse(format!(
                    "servers sharing listen {listen} must all enable TLS or all disable it"
                )));
            }
            if server.proxy_protocol != first.proxy_protocol {
                return Err(ConfigError::Parse(format!(
                    "servers sharing listen {listen} must declare identical proxy_protocol"
                )));
            }
            for name in server.server_name_list() {
                if !names.insert(name.to_ascii_lowercase()) {
                    return Err(ConfigError::Parse(format!(
                        "duplicate server_name {name:?} on shared listen {listen}"
                    )));
                }
            }
        }
    }
    // HTTP/3 providers consume only cert/key: strict_sni, crypto policy and
    // OCSP options would be silently ignored on the QUIC path — reject.
    if servers[0].http3_listen.is_some()
        && servers[0].tls.as_ref().is_some_and(|t| {
            t.strict_sni
                || !t.protocols.is_empty()
                || !t.ciphers.is_empty()
                || !t.curves.is_empty()
                || t.ocsp_stapling.is_some()
                || t.must_staple
        })
    {
        return Err(ConfigError::Parse(
            "http3_listen cannot be combined with tls.strict_sni/protocols/ciphers/curves/\
             ocsp_stapling/must_staple (HTTP/3 providers do not honor them yet)"
                .into(),
        ));
    }
    // HTTP/3 listeners load a single certificate; combining them with SNI
    // multi-cert would silently serve the primary cert for every hostname.
    if servers[0].http3_listen.is_some()
        && servers[0].tls.is_some()
        && servers
            .iter()
            .skip(1)
            .any(|s| s.listen == servers[0].listen && s.tls.is_some())
    {
        return Err(ConfigError::Parse(
            "http3_listen cannot be combined with multiple TLS servers on the same listen \
             (QUIC does not support SNI multi-certificate yet)"
                .into(),
        ));
    }
    Ok(())
}

fn validate_tls_acme(tls: &TlsConfig) -> Result<(), ConfigError> {
    validate_tls_crypto(tls)?;
    if tls.must_staple && tls.ocsp_stapling.is_none() {
        return Err(ConfigError::Parse(
            "tls.must_staple requires tls.ocsp_stapling (a DER OCSP response path)".into(),
        ));
    }
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
        match acme.challenge_type() {
            "http-01" => {
                if acme.dns_hook.is_some() || acme.dns_propagation_secs.is_some() {
                    return Err(ConfigError::Parse(
                        "acme.dns_hook/dns_propagation_secs require acme.challenge = \"dns-01\""
                            .into(),
                    ));
                }
                if acme.domains.iter().any(|d| d.starts_with("*.")) {
                    return Err(ConfigError::Parse(
                        "wildcard acme.domains require acme.challenge = \"dns-01\"".into(),
                    ));
                }
            }
            "dns-01" => {
                if acme.dns_hook.as_deref().is_none_or(|h| h.trim().is_empty()) {
                    return Err(ConfigError::Parse(
                        "acme.challenge = \"dns-01\" requires acme.dns_hook".into(),
                    ));
                }
            }
            other => {
                return Err(ConfigError::Parse(format!(
                    "acme.challenge: unsupported {other:?} (supported: \"http-01\", \"dns-01\")"
                )));
            }
        }
    }
    Ok(())
}

/// NG-12: fail-closed validation of `tls.protocols`, `tls.ciphers`, `tls.curves`.
fn validate_tls_crypto(tls: &TlsConfig) -> Result<(), ConfigError> {
    let mut tls12 = tls.protocols.is_empty();
    let mut tls13 = tls.protocols.is_empty();
    for proto in &tls.protocols {
        match proto.trim() {
            "TLSv1.2" | "TLS1.2" | "1.2" => tls12 = true,
            "TLSv1.3" | "TLS1.3" | "1.3" => tls13 = true,
            other => {
                return Err(ConfigError::Parse(format!(
                    "tls.protocols: unsupported protocol {other:?} \
                     (supported: TLSv1.2, TLSv1.3)"
                )));
            }
        }
    }
    if !tls.ciphers.is_empty() {
        let mut any_12 = false;
        let mut any_13 = false;
        let mut count = 0usize;
        for entry in &tls.ciphers {
            for token in entry.split(':').map(str::trim).filter(|s| !s.is_empty()) {
                count += 1;
                match classify_cipher(token) {
                    Some(true) => any_13 = true,
                    Some(false) => any_12 = true,
                    None => {
                        return Err(ConfigError::Parse(format!(
                            "tls.ciphers: unsupported or weak cipher suite {token:?}"
                        )));
                    }
                }
            }
        }
        if count == 0 || !((tls12 && any_12) || (tls13 && any_13)) {
            return Err(ConfigError::Parse(
                "tls.ciphers leaves no usable cipher suite for the enabled tls.protocols".into(),
            ));
        }
    }
    if !tls.curves.is_empty() {
        let mut count = 0usize;
        for entry in &tls.curves {
            for token in entry.split(':').map(str::trim).filter(|s| !s.is_empty()) {
                count += 1;
                if !is_supported_curve(token) {
                    return Err(ConfigError::Parse(format!(
                        "tls.curves: unsupported group {token:?} \
                         (supported: X25519, secp256r1/prime256v1/P-256, secp384r1/P-384)"
                    )));
                }
            }
        }
        if count == 0 {
            return Err(ConfigError::Parse(
                "tls.curves must name at least one group when set".into(),
            ));
        }
    }
    Ok(())
}

/// Returns `Some(true)` for a TLS 1.3 suite, `Some(false)` for TLS 1.2, `None` if unknown.
fn classify_cipher(name: &str) -> Option<bool> {
    match name.to_ascii_uppercase().as_str() {
        "TLS13_AES_256_GCM_SHA384"
        | "TLS_AES_256_GCM_SHA384"
        | "TLS13_AES_128_GCM_SHA256"
        | "TLS_AES_128_GCM_SHA256"
        | "TLS13_CHACHA20_POLY1305_SHA256"
        | "TLS_CHACHA20_POLY1305_SHA256" => Some(true),
        "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384"
        | "ECDHE-ECDSA-AES256-GCM-SHA384"
        | "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256"
        | "ECDHE-ECDSA-AES128-GCM-SHA256"
        | "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256"
        | "ECDHE-ECDSA-CHACHA20-POLY1305"
        | "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384"
        | "ECDHE-RSA-AES256-GCM-SHA384"
        | "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256"
        | "ECDHE-RSA-AES128-GCM-SHA256"
        | "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256"
        | "ECDHE-RSA-CHACHA20-POLY1305" => Some(false),
        _ => None,
    }
}

fn is_supported_curve(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "x25519" | "secp256r1" | "prime256v1" | "p-256" | "p256" | "secp384r1" | "p-384" | "p384"
    )
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
    fn accepts_suffix_and_regex_match() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["php", "api_ver", "root"]

[[route]]
name = "php"
match = { suffix = ".php" }
redirect = { status = 302, location = "http://php.example/" }

[[route]]
name = "api_ver"
match = { regex = "^/api/v[0-9]+/" }
redirect = { status = 302, location = "http://apiver.example/" }

[[route]]
name = "root"
match = { path = "/" }
redirect = { status = 302, location = "http://root.example/" }
"#;
        let config: AppConfig = input.parse().expect("suffix/regex config");
        assert_eq!(
            config.routes[0].r#match.kind(),
            Some(RouteMatchKind::Suffix)
        );
        assert_eq!(config.routes[1].r#match.kind(), Some(RouteMatchKind::Regex));
        assert_eq!(
            config.routes[2].r#match.kind(),
            Some(RouteMatchKind::Prefix)
        );
    }

    #[test]
    fn accepts_proxy_set_headers_and_map() {
        let input = r#"
config_version = 1

[[map]]
name = "connection_upgrade"
input = "header"
header = "Upgrade"
default = "close"
values = { "websocket" = "upgrade" }

[[server]]
listen = "127.0.0.1:8080"
routes = ["app"]

[[route]]
name = "app"
match = { path = "/", header = "X-Tenant", header_value = "acme" }
upstream = "origin"
proxy_set_headers = [
  { name = "X-Real-IP", value = "$remote_addr" },
  { name = "Connection", value = "$connection_upgrade" },
]

[[upstream]]
name = "origin"
target = "http://127.0.0.1:9000"
"#;
        let config: AppConfig = input.parse().expect("ng05");
        assert!(config.maps.contains_key("connection_upgrade"));
        assert_eq!(config.routes[0].r#match.header.as_deref(), Some("X-Tenant"));
        assert_eq!(config.routes[0].proxy_set_headers.len(), 2);
    }

    #[test]
    fn expand_unknown_map_emits_note_without_key_miss_spam() {
        use std::collections::HashMap;
        let maps: HashMap<String, MapConfig> = HashMap::new();
        let ctx = HeaderExpandCtx {
            host: "h.example",
            remote_addr: "127.0.0.1",
            request_uri: "/x",
            scheme: "http",
            request_headers: &[],
            maps: &maps,
        };
        let (out, notes) = expand_header_template_with_notes("x=$missing_map", &ctx);
        assert_eq!(out, "x=");
        assert_eq!(
            notes,
            vec![MapExpandNote::UnknownMap {
                name: "missing_map".into()
            }]
        );
        // Known allowlisted vars produce no notes.
        let (out2, notes2) = expand_header_template_with_notes("$host", &ctx);
        assert_eq!(out2, "h.example");
        assert!(notes2.is_empty());
    }

    #[test]
    fn rejects_unknown_header_var() {
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["app"]
[[route]]
name = "app"
match = { path = "/" }
upstream = "origin"
proxy_set_headers = [ { name = "X", value = "$http_cookie" } ]
[[upstream]]
name = "origin"
target = "http://127.0.0.1:9000"
"#;
        assert!(input.parse::<AppConfig>().is_err());
    }

    #[test]
    fn accepts_error_pages_map() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
error_pages = { "404" = "/404.html", "500" = "/50x.html" }

[[route]]
name = "site"
match = { path = "/" }
root = "/var/www"
"#;
        let config: AppConfig = input.parse().expect("error_pages");
        assert_eq!(
            config.servers[0].error_pages.get("404").map(String::as_str),
            Some("/404.html")
        );
        assert_eq!(
            config.servers[0].error_pages.get("500").map(String::as_str),
            Some("/50x.html")
        );
    }

    #[test]
    fn rejects_error_pages_named_and_url() {
        for bad in [
            r#"error_pages = { "404" = "@named" }"#,
            r#"error_pages = { "404" = "http://x/404" }"#,
            r#"error_pages = { "200" = "/ok.html" }"#,
        ] {
            let input = format!(
                r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
{bad}
[[route]]
name = "site"
match = {{ path = "/" }}
root = "/var/www"
"#
            );
            assert!(
                input.parse::<AppConfig>().is_err(),
                "expected reject for {bad}"
            );
        }
    }

    #[test]
    fn accepts_fallback_rewrite_with_root() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["spa"]

[[route]]
name = "spa"
match = { path = "/" }
root = "/var/www"
fallback_rewrite = "/index.html"
try_uri = true
try_uri_dir = false
"#;
        let config: AppConfig = input.parse().expect("fallback_rewrite");
        assert_eq!(
            config.routes[0].fallback_rewrite.as_deref(),
            Some("/index.html")
        );
        assert_eq!(config.routes[0].try_uri, Some(true));
    }

    #[test]
    fn rejects_fallback_rewrite_without_root() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["bad"]

[[route]]
name = "bad"
match = { path = "/" }
rewrite = "/x"
fallback_rewrite = "/index.html"
"#;
        // rewrite + fallback_rewrite conflict (and no root for fallback).
        let err = input.parse::<AppConfig>().unwrap_err();
        let msg = err.to_string();
        assert!(
            matches!(err, ConfigError::RouteConflictingActions { .. })
                || msg.contains("fallback_rewrite")
                || msg.contains("conflicting"),
            "{err:?} / {msg}"
        );
    }

    #[test]
    fn rejects_fallback_rewrite_missing_root() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["bad"]

[[route]]
name = "bad"
match = { path = "/" }
fallback_rewrite = "/index.html"
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(
            err.to_string().contains("fallback_rewrite requires root"),
            "{err}"
        );
    }

    #[test]
    fn rejects_mixed_path_and_regex() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["bad"]

[[route]]
name = "bad"
match = { path = "/api", regex = "^/api/" }
redirect = { status = 302, location = "http://x/" }
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidRouteMatchKind));
    }

    #[test]
    fn rejects_invalid_regex_syntax() {
        let input = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["bad"]

[[route]]
name = "bad"
match = { regex = "(unclosed" }
redirect = { status = 302, location = "http://x/" }
"#;
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidRouteRegex { .. }));
    }

    #[test]
    fn rejects_regex_over_byte_limit() {
        let long = "a".repeat(MAX_ROUTE_REGEX_BYTES + 1);
        let input = format!(
            r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["bad"]

[[route]]
name = "bad"
match = {{ regex = "{long}" }}
redirect = {{ status = 302, location = "http://x/" }}
"#
        );
        let err = input.parse::<AppConfig>().unwrap_err();
        assert!(matches!(err, ConfigError::RouteRegexTooLong { .. }));
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
            None,
            None,
            0,
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
                None,
                None,
                0,
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
    fn ng11_real_ip_enabled_requires_trusted() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
[modules.real_ip]
enabled = true
"#;
        let err = AppConfig::parse_str(toml).unwrap_err().to_string();
        assert!(err.contains("trusted"), "{err}");
    }

    #[test]
    fn ng11_real_ip_rejects_bad_cidr_and_bad_header() {
        let base = |trusted: &str, header: &str| {
            format!(
                r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = {{ path = "/" }}
root = "/tmp"
[modules.real_ip]
enabled = true
trusted = [{trusted}]
header = {header}
"#
            )
        };
        let err = AppConfig::parse_str(&base(r#""not-a-cidr""#, r#""x-forwarded-for""#))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not-a-cidr"), "{err}");
        let err = AppConfig::parse_str(&base(r#""10.0.0.0/8""#, r#""bad header""#))
            .unwrap_err()
            .to_string();
        assert!(err.contains("header"), "{err}");
        AppConfig::parse_str(&base(r#""10.0.0.0/8", "192.0.2.10""#, r#""x-real-ip""#))
            .expect("valid real_ip config");
    }

    #[test]
    fn ng11_real_ip_disabled_skips_validation() {
        // Disabled module ignores malformed entries — never consulted at runtime.
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
[modules.real_ip]
enabled = false
trusted = ["garbage"]
"#;
        AppConfig::parse_str(toml).expect("disabled real_ip skips validation");
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

    #[test]
    fn cache_headers_by_mime_parses_and_looks_up() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "/var/www"
[route.cache_headers.by_mime."text/css"]
max_age = 2592000
immutable = true
[route.cache_headers.by_mime."*"]
max_age = 3600
"#;
        let cfg = AppConfig::parse_str(toml).expect("parse");
        let site = cfg.routes.iter().find(|r| r.name == "site").unwrap();
        let ch = site.cache_headers.as_ref().expect("cache_headers");
        assert_eq!(
            ch.cache_control_for("text/css; charset=utf-8").as_deref(),
            Some("public, max-age=2592000, immutable")
        );
        assert_eq!(
            ch.cache_control_for("application/octet-stream").as_deref(),
            Some("public, max-age=3600")
        );
    }

    #[test]
    fn cache_headers_zero_max_age_rejected() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "/var/www"
[route.cache_headers.by_mime."text/css"]
max_age = 0
"#;
        let err = AppConfig::parse_str(toml).unwrap_err().to_string();
        assert!(err.contains("max_age"), "{err}");
    }

    #[test]
    fn response_headers_parse_and_expand() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "/var/www"
[[route.response_headers]]
name = "X-Frame-Options"
value = "SAMEORIGIN"
[[route.response_headers]]
name = "X-Debug-Host"
value = "h=$host;u=$uri"
always = true
"#;
        let cfg = AppConfig::parse_str(toml).expect("parse");
        let site = cfg.routes.iter().find(|r| r.name == "site").unwrap();
        assert_eq!(site.response_headers.len(), 2);
        assert!(site.response_headers[1].always);
        let expanded = expand_response_header_template(
            &site.response_headers[1].value,
            &ResponseHeaderExpandCtx {
                host: "app.example",
                uri: "/page",
            },
        );
        assert_eq!(expanded, "h=app.example;u=/page");
    }

    #[test]
    fn response_headers_denylist_cache_control() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "/var/www"
[[route.response_headers]]
name = "Cache-Control"
value = "no-store"
"#;
        let err = AppConfig::parse_str(toml).unwrap_err().to_string();
        assert!(
            err.contains("denylist") || err.contains("Cache-Control"),
            "{err}"
        );
    }

    #[test]
    fn rewrite_allows_uri_and_host_vars() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["rw"]
[[route]]
name = "rw"
match = { regex = "^/x$" }
rewrite = { pattern = "^/x$", replacement = "/h/$host$uri", flag = "last" }
"#;
        AppConfig::parse_str(toml).expect("rewrite $host/$uri ok");
    }

    #[test]
    fn ng07_upstream_endpoints_and_policy_parse() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
endpoints = [
    { address = "10.0.0.1", port = 8001, weight = 3 },
    { address = "10.0.0.2", port = 8001 },
]
selection_policy = "least_conn"
"#;
        let cfg = AppConfig::parse_str(toml).expect("parse");
        let up = &cfg.upstreams["b"];
        assert_eq!(up.endpoint_set.endpoints().len(), 2);
        assert_eq!(up.selection_policy, EndpointSelectionPolicy::LeastConn);
        assert!(
            up.endpoint_set.endpoints().iter().any(|e| e.weight == 3),
            "weight flag must reach the endpoint"
        );
    }

    #[test]
    fn ng09_dns_refresh_ms_parses_and_bounds() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
endpoints = [{ address = "backend.internal", port = 8001 }]
dns_refresh_ms = 5000
"#;
        let cfg = AppConfig::parse_str(toml).expect("parse");
        assert_eq!(cfg.upstreams["b"].dns_refresh_ms, 5_000);
        // Bound check: over MAX_DNS_REFRESH_MS must error, not clamp.
        let err = UpstreamConfig::try_from_parts(
            "u".into(),
            Some("http://127.0.0.1:9".into()),
            Vec::new(),
            EndpointSelectionPolicy::default(),
            EndpointFailoverPolicy::default(),
            1_000,
            1,
            UpstreamHealthCheckConfig::default(),
            None,
            None,
            crate::MAX_DNS_REFRESH_MS + 1,
        )
        .unwrap_err();
        assert!(err.to_string().contains("dns_refresh_ms"), "{err}");
    }

    #[test]
    fn ng07_header_affinity_requires_sticky_header() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
target = "http://10.0.0.1:80"
selection_policy = "header_affinity"
"#;
        let err = AppConfig::parse_str(toml).unwrap_err().to_string();
        assert!(err.contains("sticky_header"), "{err}");
    }

    #[test]
    fn ng07_cookie_affinity_defaults_sticky_cookie() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
target = "http://10.0.0.1:80"
selection_policy = "cookie_affinity"
"#;
        let cfg = AppConfig::parse_str(toml).expect("parse");
        assert_eq!(
            cfg.upstreams["b"].sticky_cookie.as_deref(),
            Some(DEFAULT_STICKY_COOKIE)
        );
    }

    #[test]
    fn ng07_sticky_cookie_empty_or_invalid_rejected() {
        for bad in ["", "bad name", "semi;colon", "comma,sep"] {
            let toml = format!(
                r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = {{ path = "/" }}
upstream = "b"
[[upstream]]
name = "b"
target = "http://10.0.0.1:80"
selection_policy = "cookie_affinity"
sticky_cookie = "{bad}"
"#,
            );
            let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
            assert!(
                err.contains("sticky_cookie"),
                "bad cookie `{bad}` must error, got: {err}"
            );
        }
    }

    #[test]
    fn ng07_sticky_header_invalid_token_rejected() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
target = "http://10.0.0.1:80"
selection_policy = "header_affinity"
sticky_header = "x bad name"
"#;
        let err = AppConfig::parse_str(toml).unwrap_err().to_string();
        assert!(err.contains("sticky_header"), "{err}");
    }

    #[test]
    fn ng07_ambiguous_target_and_endpoints_rejected() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
target = "http://10.0.0.1:80"
endpoints = [{ address = "10.0.0.2", port = 80 }]
"#;
        assert!(AppConfig::parse_str(toml).is_err());
    }

    const NG15_TLS: &str = r#"
[server.tls]
cert = "/tmp/cert.pem"
key = "/tmp/key.pem"
"#;

    fn ng15_config(servers: &str) -> String {
        format!(
            r#"
config_version = 1
{servers}
[[route]]
name = "r"
match = {{ path = "/" }}
redirect = {{ status = 301, location = "https://example/" }}
"#
        )
    }

    #[test]
    fn ng15_accepts_sni_multi_cert_same_listen() {
        let toml = ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test", "www.a.test"]
routes = ["r"]
{NG15_TLS}
[[server]]
listen = "0.0.0.0:443"
server_name = ["b.test"]
routes = ["r"]
{NG15_TLS}
"#
        ));
        let config = AppConfig::parse_str(&toml).expect("disjoint names + tls on all blocks");
        assert_eq!(config.servers.len(), 2);
    }

    #[test]
    fn ng15_rejects_duplicate_server_name_same_listen() {
        let toml = ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test", "B.Test"]
routes = ["r"]
{NG15_TLS}
[[server]]
listen = "0.0.0.0:443"
server_name = ["b.test"]
routes = ["r"]
{NG15_TLS}
"#
        ));
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("duplicate server_name"), "{err}");
    }

    #[test]
    fn ng15_rejects_mixed_tls_same_listen() {
        let toml = ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test"]
routes = ["r"]
{NG15_TLS}
[[server]]
listen = "0.0.0.0:443"
server_name = ["b.test"]
routes = ["r"]
"#
        ));
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("enable TLS or all disable"), "{err}");
    }

    #[test]
    fn ng15_rejects_nameless_non_first_server_same_listen() {
        let toml = ng15_config(
            r#"
[[server]]
listen = "0.0.0.0:8080"
routes = ["r"]
[[server]]
listen = "0.0.0.0:8080"
routes = ["r"]
"#,
        );
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("only the first block"), "{err}");
    }

    #[test]
    fn ng15_rejects_conflicting_proxy_protocol_same_listen() {
        let toml = ng15_config(
            r#"
[[server]]
listen = "0.0.0.0:8080"
server_name = ["a.test"]
routes = ["r"]
proxy_protocol = { enabled = true }
[[server]]
listen = "0.0.0.0:8080"
server_name = ["b.test"]
routes = ["r"]
"#,
        );
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("proxy_protocol"), "{err}");
    }

    #[test]
    fn ng15_rejects_http3_listen_on_non_primary_server() {
        let toml = ng15_config(
            r#"
[[server]]
listen = "0.0.0.0:8080"
routes = ["r"]
[[server]]
listen = "0.0.0.0:8081"
server_name = ["b.test"]
routes = ["r"]
http3_listen = "0.0.0.0:8443"
"#,
        );
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("http3_listen"), "{err}");
    }

    #[test]
    fn ng15_rejects_http3_with_sni_multi_cert() {
        let toml = ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test"]
routes = ["r"]
http3_listen = "0.0.0.0:443"
{NG15_TLS}
[[server]]
listen = "0.0.0.0:443"
server_name = ["b.test"]
routes = ["r"]
{NG15_TLS}
"#
        ));
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("QUIC does not support"), "{err}");
    }

    #[test]
    fn ng15_rejects_invalid_server_names() {
        for bad in [
            "",
            "a test",
            "mid*.test",
            "*",
            "a..test",
            "-x.test",
            "a.test.",
        ] {
            let toml = ng15_config(&format!(
                r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["{bad}"]
routes = ["r"]
"#
            ));
            assert!(
                AppConfig::parse_str(&toml).is_err(),
                "server_name {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn ng15_accepts_wildcard_server_name() {
        let toml = ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["*.wild.test"]
routes = ["r"]
{NG15_TLS}
[[server]]
listen = "0.0.0.0:443"
server_name = ["b.test"]
routes = ["r"]
{NG15_TLS}
"#
        ));
        AppConfig::parse_str(&toml).expect("*.suffix wildcard is valid");
    }

    #[test]
    fn ng15_rejects_acme_on_non_primary_server() {
        let toml = ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test"]
routes = ["r"]
{NG15_TLS}
[[server]]
listen = "0.0.0.0:443"
server_name = ["b.test"]
routes = ["r"]
[server.tls]
cert = "/tmp/cert.pem"
key = "/tmp/key.pem"
[server.tls.acme]
enabled = true
email = "ops@example.com"
domains = ["b.test"]
"#
        ));
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("tls.acme"), "{err}");
    }

    #[test]
    fn ap02_rejects_strict_sni_on_non_primary_server() {
        let toml = ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test"]
routes = ["r"]
{NG15_TLS}
[[server]]
listen = "0.0.0.0:443"
server_name = ["b.test"]
routes = ["r"]
[server.tls]
cert = "/tmp/cert.pem"
key = "/tmp/key.pem"
strict_sni = true
"#
        ));
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("strict_sni"), "{err}");
    }

    #[test]
    fn ap02_accepts_strict_sni_on_primary_server() {
        let toml = ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test"]
routes = ["r"]
[server.tls]
cert = "/tmp/cert.pem"
key = "/tmp/key.pem"
strict_sni = true
[[server]]
listen = "0.0.0.0:443"
server_name = ["b.test"]
routes = ["r"]
{NG15_TLS}
"#
        ));
        let config = AppConfig::parse_str(&toml).expect("strict_sni on primary is valid");
        assert!(config.servers[0].tls.as_ref().unwrap().strict_sni);
    }

    fn ng12_toml(tls_extra: &str) -> String {
        ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test"]
routes = ["r"]
[server.tls]
cert = "/tmp/cert.pem"
key = "/tmp/key.pem"
{tls_extra}
"#
        ))
    }

    #[test]
    fn ng12_accepts_protocols_ciphers_curves() {
        let config = AppConfig::parse_str(&ng12_toml(
            r#"protocols = ["TLSv1.2", "TLSv1.3"]
ciphers = ["TLS13_AES_256_GCM_SHA384:ECDHE-RSA-AES256-GCM-SHA384"]
curves = ["X25519", "P-384"]
"#,
        ))
        .expect("valid crypto config");
        let tls = config.servers[0].tls.as_ref().unwrap();
        assert_eq!(tls.protocols, ["TLSv1.2", "TLSv1.3"]);
        assert_eq!(tls.ciphers.len(), 1);
        assert_eq!(tls.curves.len(), 2);
    }

    #[test]
    fn ng12_rejects_unknown_protocol() {
        let err = AppConfig::parse_str(&ng12_toml(r#"protocols = ["TLSv1.1"]"#))
            .unwrap_err()
            .to_string();
        assert!(err.contains("tls.protocols"), "{err}");
    }

    #[test]
    fn ng12_rejects_unknown_cipher() {
        let err = AppConfig::parse_str(&ng12_toml(r#"ciphers = ["DES-CBC3-SHA"]"#))
            .unwrap_err()
            .to_string();
        assert!(err.contains("tls.ciphers"), "{err}");
    }

    #[test]
    fn ng12_rejects_cipher_leaving_no_usable_suite() {
        let err = AppConfig::parse_str(&ng12_toml(
            r#"protocols = ["TLSv1.3"]
ciphers = ["ECDHE-RSA-AES256-GCM-SHA384"]
"#,
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("tls.ciphers"), "{err}");
    }

    #[test]
    fn ng12_rejects_unknown_curve() {
        let err = AppConfig::parse_str(&ng12_toml(r#"curves = ["secp521r1"]"#))
            .unwrap_err()
            .to_string();
        assert!(err.contains("tls.curves"), "{err}");
    }

    #[test]
    fn ap02_rejects_strict_sni_without_server_name() {
        let toml = ng15_config(
            r#"
[[server]]
listen = "0.0.0.0:443"
routes = ["r"]
[server.tls]
cert = "/tmp/cert.pem"
key = "/tmp/key.pem"
strict_sni = true
"#,
        );
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("strict_sni"), "{err}");
    }

    fn ng15_fpc_toml(fpc_extra: &str) -> String {
        ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:8080"
server_name = ["a.test"]
routes = ["r"]
{fpc_extra}
"#
        ))
    }

    #[test]
    fn ol01_accepts_private_cookies_and_rejects_keyed_overlap() {
        let config = AppConfig::parse_str(&ng15_fpc_toml(
            r#"
[full_page_cache]
enabled = true
private_cookies = ["wordpress_logged_in_abc"]
"#,
        ))
        .expect("valid private cookie");
        assert_eq!(
            config.full_page_cache.private_cookies,
            ["wordpress_logged_in_abc"]
        );

        let err = AppConfig::parse_str(&ng15_fpc_toml(
            r#"
[full_page_cache]
enabled = true
private_cookies = ["wpml"]
[full_page_cache.key]
cookies = ["wpml"]
"#,
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("both private and keyed"), "{err}");
    }

    #[test]
    fn ng16_accepts_cache_tags_and_stale_window() {
        let config = AppConfig::parse_str(&ng15_fpc_toml(
            r#"
[full_page_cache]
enabled = true
stale_if_error_secs = 120
"#,
        ))
        .expect("valid stale window");
        assert_eq!(config.full_page_cache.stale_if_error_secs, 120);

        let toml = r#"
config_version = 1

[[server]]
listen = "0.0.0.0:8080"
routes = ["r"]

[full_page_cache]
enabled = true

[[route]]
name = "r"
match = { path = "/" }
root = "/srv"
cache_tags = ["blog", "post:1"]
"#;
        let config = AppConfig::parse_str(toml).expect("valid cache_tags");
        assert_eq!(config.routes[0].cache_tags, ["blog", "post:1"]);
    }

    #[test]
    fn ng16_rejects_invalid_cache_tag() {
        let toml = r#"
config_version = 1

[[server]]
listen = "0.0.0.0:8080"
routes = ["r"]

[[route]]
name = "r"
match = { path = "/" }
root = "/srv"
cache_tags = ["bad tag"]
"#;
        let err = AppConfig::parse_str(toml).unwrap_err().to_string();
        assert!(err.contains("cache_tags"), "{err}");
    }

    #[test]
    fn ng15_accepts_cache_key_and_bypass() {
        let config = AppConfig::parse_str(&ng15_fpc_toml(
            r#"
[full_page_cache]
enabled = true
[full_page_cache.key]
query = ["page"]
cookies = ["wpml"]
headers = ["Accept-Language"]
[full_page_cache.bypass]
cookies = ["wordpress_logged_in_x"]
headers = ["X-No-Cache"]
"#,
        ))
        .expect("valid fpc key/bypass");
        let fpc = &config.full_page_cache;
        assert_eq!(fpc.key.query, ["page"]);
        assert_eq!(fpc.key.cookies, ["wpml"]);
        assert_eq!(fpc.key.headers, ["Accept-Language"]);
        assert_eq!(fpc.bypass.headers, ["X-No-Cache"]);
    }

    #[test]
    fn ng15_rejects_cookie_both_keyed_and_bypassed() {
        let err = AppConfig::parse_str(&ng15_fpc_toml(
            r#"
[full_page_cache]
enabled = true
[full_page_cache.key]
cookies = ["wpml"]
[full_page_cache.bypass]
cookies = ["WPML"]
"#,
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("both keyed and bypassed"), "{err}");
    }

    #[test]
    fn ng15_rejects_invalid_key_names() {
        for extra in [
            "[full_page_cache]\nenabled = true\n[full_page_cache.key]\nheaders = [\"\"]",
            "[full_page_cache]\nenabled = true\n[full_page_cache.key]\nquery = [\"a=b\"]",
            "[full_page_cache]\nenabled = true\n[full_page_cache.bypass]\nheaders = [\"a b\"]",
        ] {
            assert!(
                AppConfig::parse_str(&ng15_fpc_toml(extra)).is_err(),
                "{extra}"
            );
        }
    }

    fn q09_toml(domains: &str, acme_extra: &str) -> String {
        ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test"]
routes = ["r"]
[server.tls]
cert = "/tmp/cert.pem"
key = "/tmp/key.pem"
[server.tls.acme]
email = "ops@a.test"
domains = {domains}
{acme_extra}
"#
        ))
    }

    #[test]
    fn q09_accepts_dns01_with_hook_and_wildcard() {
        let config = AppConfig::parse_str(&q09_toml(
            r#"["a.test", "*.a.test"]"#,
            r#"challenge = "dns-01"
dns_hook = "/usr/local/bin/acme-dns"
dns_propagation_secs = 30
"#,
        ))
        .expect("dns-01 with hook and wildcard is valid");
        let acme = config.servers[0]
            .tls
            .as_ref()
            .unwrap()
            .acme
            .as_ref()
            .unwrap();
        assert_eq!(acme.challenge_type(), "dns-01");
        assert_eq!(acme.domains.len(), 2);
    }

    #[test]
    fn q09_rejects_dns01_without_hook() {
        let err = AppConfig::parse_str(&q09_toml(r#"["a.test"]"#, r#"challenge = "dns-01""#))
            .unwrap_err()
            .to_string();
        assert!(err.contains("dns_hook"), "{err}");
    }

    #[test]
    fn q09_rejects_wildcard_with_http01() {
        let err = AppConfig::parse_str(&q09_toml(r#"["*.a.test"]"#, ""))
            .unwrap_err()
            .to_string();
        assert!(err.contains("wildcard"), "{err}");
    }

    #[test]
    fn q09_rejects_unknown_challenge_and_http01_hook() {
        for extra in [
            r#"challenge = "tls-alpn-01""#,
            r#"dns_hook = "/bin/true""#,
            r#"dns_propagation_secs = 5"#,
        ] {
            assert!(
                AppConfig::parse_str(&q09_toml(r#"["a.test"]"#, extra)).is_err(),
                "{extra}"
            );
        }
    }

    #[test]
    fn ng13_must_staple_requires_ocsp_stapling() {
        let err = AppConfig::parse_str(&ng12_toml("must_staple = true"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("must_staple"), "{err}");

        let config = AppConfig::parse_str(&ng12_toml(
            r#"ocsp_stapling = "/tmp/ocsp.der"
must_staple = true"#,
        ))
        .expect("must_staple with ocsp_stapling is valid");
        let tls = config.servers[0].tls.as_ref().unwrap();
        assert_eq!(
            tls.ocsp_stapling.as_deref(),
            Some(std::path::Path::new("/tmp/ocsp.der"))
        );
    }

    #[test]
    fn ng12_rejects_crypto_and_strict_sni_with_http3() {
        for extra in [
            r#"protocols = ["TLSv1.3"]"#,
            r#"ciphers = ["TLS13_AES_256_GCM_SHA384"]"#,
            r#"curves = ["X25519"]"#,
            "strict_sni = true",
        ] {
            let toml = ng15_config(&format!(
                r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test"]
routes = ["r"]
http3_listen = "0.0.0.0:443"
[server.tls]
cert = "/tmp/cert.pem"
key = "/tmp/key.pem"
{extra}
"#
            ));
            let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
            assert!(err.contains("http3_listen"), "{extra}: {err}");
        }
    }

    #[test]
    fn ng12_rejects_crypto_on_non_primary_server() {
        let toml = ng15_config(&format!(
            r#"
[[server]]
listen = "0.0.0.0:443"
server_name = ["a.test"]
routes = ["r"]
{NG15_TLS}
[[server]]
listen = "0.0.0.0:443"
server_name = ["b.test"]
routes = ["r"]
[server.tls]
cert = "/tmp/cert.pem"
key = "/tmp/key.pem"
protocols = ["TLSv1.3"]
"#
        ));
        let err = AppConfig::parse_str(&toml).unwrap_err().to_string();
        assert!(err.contains("tls.protocols"), "{err}");
    }
}
