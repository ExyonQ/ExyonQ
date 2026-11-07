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
//! Optional official module settings (Fase 2).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModulesConfig {
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub compression: CompressionConfig,
    #[serde(default)]
    pub ratelimit: RateLimitConfig,
    #[serde(default)]
    pub real_ip: RealIpConfig,
    /// NG-18 / ADR-064: `gunzip` response filter (default off).
    #[serde(default, skip_serializing_if = "GunzipConfig::is_default")]
    pub gunzip: GunzipConfig,
    /// NG-18 / ADR-064 Phase C: nginx `sub_filter` rules per route.
    #[serde(default, skip_serializing_if = "SubFilterConfig::is_default")]
    pub sub_filter: SubFilterConfig,
    /// NG-18 / ADR-064 Phase D: nginx `mirror` per route.
    #[serde(default, skip_serializing_if = "MirrorConfig::is_default")]
    pub mirror: MirrorConfig,
    /// NG-18 / ADR-064 Phase E: nginx `slice` per route.
    #[serde(default, skip_serializing_if = "SliceConfig::is_default")]
    pub slice: SliceConfig,
    /// 0.5: nginx `auth_basic` per route.
    #[serde(default, skip_serializing_if = "AuthConfig::is_default")]
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthRouteConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub realm: String,
    /// htpasswd file path.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user_file: String,
    /// AP-04: external authorization endpoint (`auth_request`-style).
    /// Mutually exclusive with the htpasswd fields above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_request: Option<AuthRequestConfig>,
}

/// AP-04: external authorization subrequest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthRequestConfig {
    /// Absolute `http://` URL called with `GET`. The shared client stack is
    /// HTTP-only in v1, so an `https://` endpoint is rejected at config load
    /// instead of failing closed on every request.
    pub endpoint: String,
    /// Hard upper bound for the auth subrequest; default 2 s.
    #[serde(default = "default_auth_request_timeout_ms")]
    pub timeout_ms: u32,
}

fn default_auth_request_timeout_ms() -> u32 {
    2_000
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, AuthRouteConfig>,
}

impl AuthConfig {
    pub fn is_default(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        for (route, cfg) in &self.routes {
            let has_basic = !cfg.realm.trim().is_empty() || !cfg.user_file.trim().is_empty();
            match (&cfg.auth_request, has_basic) {
                (Some(request), false) => {
                    let uri: hyper::Uri = request.endpoint.parse().map_err(|_| {
                        crate::ConfigError::Parse(format!(
                            "modules.auth.routes.{route}.auth_request.endpoint must be an absolute http:// URI"
                        ))
                    })?;
                    if !matches!(uri.scheme_str(), Some("http"))
                        || uri.authority().is_none()
                        || uri.host().is_none()
                        || uri
                            .authority()
                            .is_some_and(|authority| authority.as_str().contains('@'))
                    {
                        return Err(crate::ConfigError::Parse(format!(
                            "modules.auth.routes.{route}.auth_request.endpoint must be an absolute http:// URI without userinfo"
                        )));
                    }
                    if uri.path().is_empty() {
                        return Err(crate::ConfigError::Parse(format!(
                            "modules.auth.routes.{route}.auth_request.endpoint needs a path"
                        )));
                    }
                    if !(1..=30_000).contains(&request.timeout_ms) {
                        return Err(crate::ConfigError::Parse(format!(
                            "modules.auth.routes.{route}.auth_request.timeout_ms must be 1..=30000"
                        )));
                    }
                }
                (Some(_), true) => {
                    return Err(crate::ConfigError::Parse(format!(
                        "modules.auth.routes.{route}: auth_request cannot be combined with realm/user_file"
                    )));
                }
                (None, true) => {
                    if cfg.realm.trim().is_empty() {
                        return Err(crate::ConfigError::Parse(format!(
                            "modules.auth.routes.{route}: empty realm"
                        )));
                    }
                    if cfg.user_file.trim().is_empty() {
                        return Err(crate::ConfigError::Parse(format!(
                            "modules.auth.routes.{route}: empty user_file"
                        )));
                    }
                }
                (None, false) => {
                    return Err(crate::ConfigError::Parse(format!(
                        "modules.auth.routes.{route}: needs realm/user_file or auth_request.endpoint"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Effective nginx `slice` policy for one route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceRouteConfig {
    /// Segment size in bytes (`slice 1m` → 1048576).
    pub size: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceConfig {
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, SliceRouteConfig>,
}

impl SliceConfig {
    pub fn is_default(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        for (route, cfg) in &self.routes {
            if !(1024..=1_073_741_824).contains(&cfg.size) {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.slice.routes.{route}.size must be 1024..=1073741824, got {}",
                    cfg.size
                )));
            }
        }
        Ok(())
    }
}

/// Effective nginx `mirror` policy for one route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MirrorRouteConfig {
    /// Internal destination paths (`mirror /path;`).
    #[serde(default)]
    pub destinations: Vec<String>,
    /// `mirror_request_body` (nginx 1.27 default: on).
    #[serde(default = "default_true")]
    pub request_body: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MirrorConfig {
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, MirrorRouteConfig>,
}

impl MirrorConfig {
    pub fn is_default(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        for (route, cfg) in &self.routes {
            if cfg.destinations.is_empty() {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.mirror.routes.{route}: no destinations"
                )));
            }
            for dest in &cfg.destinations {
                if !dest.starts_with('/') {
                    return Err(crate::ConfigError::Parse(format!(
                        "modules.mirror.routes.{route}: destination `{dest}` must be an internal path"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// One nginx `sub_filter <pattern> <replacement>` rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubFilterRule {
    pub pattern: String,
    pub replacement: String,
}

/// Effective `sub_filter` policy for one route (importer resolves
/// http/server/location inheritance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubFilterRouteConfig {
    #[serde(default)]
    pub rules: Vec<SubFilterRule>,
    /// nginx `sub_filter_once` (default on).
    #[serde(default = "default_true")]
    pub once: bool,
    /// nginx `sub_filter_types` (default `text/html`).
    #[serde(default = "default_sub_filter_types")]
    pub types: Vec<String>,
    /// nginx `sub_filter_last_modified` (default off: remove Last-Modified).
    #[serde(default = "default_false")]
    pub last_modified: bool,
}

fn default_true() -> bool {
    true
}

fn default_sub_filter_types() -> Vec<String> {
    vec!["text/html".to_string()]
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubFilterConfig {
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, SubFilterRouteConfig>,
}

impl SubFilterConfig {
    pub fn is_default(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        for (route, cfg) in &self.routes {
            for rule in &cfg.rules {
                if rule.pattern.is_empty() {
                    return Err(crate::ConfigError::Parse(format!(
                        "modules.sub_filter.routes.{route}: empty pattern"
                    )));
                }
            }
            if cfg.types.iter().any(|t| t.trim().is_empty()) {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.sub_filter.routes.{route}: empty sub_filter_types entry"
                )));
            }
        }
        Ok(())
    }
}

/// NG-11: trust-aware client-IP resolution (nginx `set_real_ip_from` +
/// `real_ip_header`). When the immediate peer is inside `trusted`, the real
/// client IP is recovered from `header` (right-to-left, skipping trusted
/// hops). Default: disabled — the socket peer is always the client IP and
/// client-supplied forwarding headers can never spoof it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealIpConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Trusted proxy CIDRs / IPs (e.g. `"10.0.0.0/8"`, `"192.0.2.10"`).
    #[serde(default)]
    pub trusted: Vec<String>,
    /// Header carrying the forwarded chain (nginx `real_ip_header`).
    #[serde(default = "default_real_ip_header")]
    pub header: String,
}

fn default_real_ip_header() -> String {
    "x-forwarded-for".to_string()
}

impl Default for RealIpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            trusted: Vec::new(),
            header: default_real_ip_header(),
        }
    }
}

impl RealIpConfig {
    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        if !self.enabled {
            return Ok(());
        }
        if self.trusted.is_empty() {
            return Err(crate::ConfigError::Parse(
                "modules.real_ip: enabled requires at least one trusted CIDR".into(),
            ));
        }
        for t in &self.trusted {
            let ok = t.parse::<ipnetwork::IpNetwork>().is_ok()
                || t.trim().parse::<std::net::IpAddr>().is_ok();
            if !ok {
                return Err(crate::ConfigError::Parse(format!(
                    "modules.real_ip.trusted entry `{t}` is not a CIDR or IP"
                )));
            }
        }
        let name = self.header.trim();
        let token_ok = !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
        if !token_ok {
            return Err(crate::ConfigError::Parse(format!(
                "modules.real_ip.header `{}` is not a valid header name",
                self.header
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_metrics_path")]
    pub path: String,
    #[serde(default = "default_health_path")]
    pub health_path: String,
    /// Required when metrics is enabled and any `server.listen` is non-loopback
    /// (LA-CAP054-008). Optional on loopback-only binds (Cap054 E2E).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrape_bearer_token: Option<String>,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: default_metrics_path(),
            health_path: default_health_path(),
            scrape_bearer_token: None,
        }
    }
}

/// NG-18 / ADR-064: nginx `gunzip` response filter.
///
/// `buffer_bytes` bounds the decoder output per poll; it is the ExyonQ
/// equivalent of `gunzip_buffers number size` (total bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GunzipConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_gunzip_buffer_bytes")]
    pub buffer_bytes: usize,
}

fn default_gunzip_buffer_bytes() -> usize {
    32 * 1024
}

impl Default for GunzipConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            buffer_bytes: default_gunzip_buffer_bytes(),
        }
    }
}

impl GunzipConfig {
    pub fn is_default(&self) -> bool {
        !self.enabled && self.buffer_bytes == default_gunzip_buffer_bytes()
    }

    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        if !(1024..=1_048_576).contains(&self.buffer_bytes) {
            return Err(crate::ConfigError::Parse(format!(
                "modules.gunzip.buffer_bytes must be 1024..=1048576, got {}",
                self.buffer_bytes
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompressionConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_min_bytes")]
    pub min_bytes: usize,
    /// flate2 level 1–9 (default 1 = fastest, bench P9).
    #[serde(default = "default_compression_level")]
    pub level: u32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_bytes: default_min_bytes(),
            level: default_compression_level(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_rps")]
    pub requests_per_second: u32,
    #[serde(default = "default_burst")]
    pub burst: u32,
    /// 0.5: nginx `limit_req_zone` equivalents.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub zones: std::collections::BTreeMap<String, RateLimitZoneConfig>,
    /// 0.5: route name → zone name (`limit_req zone=...`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub routes: std::collections::BTreeMap<String, String>,
    /// Per-route nginx `limit_req` options. Kept separate from `routes` so
    /// older zone-only TOML remains loadable.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub route_options: std::collections::BTreeMap<String, RateLimitRouteOptions>,
    /// Multiple `limit_req` directives for one route. Imported nginx configs
    /// use this map; `routes` + `route_options` remain the single-limit form.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub route_limits: std::collections::BTreeMap<String, RateLimitRoutePolicy>,
    /// OL-03: OpenLiteSpeed-style per-client throttling. Independent from
    /// nginx `limit_req` zones; applies to every route when enabled.
    #[serde(default, skip_serializing_if = "ClientThrottleConfig::is_default")]
    pub client: ClientThrottleConfig,
}

/// OL-03: complete per-client throttling policy.
///
/// Mirrors the OpenLiteSpeed `perClientConnLimit` block (WebAdmin labels it
/// "Per Client Throttling"): `softLimit`, `hardLimit`, `outBandwidth`,
/// `inBandwidth`, `staticReqPerSec`, `dynReqPerSec`, `gracePeriod`,
/// `banPeriod` and `blockBadReq`. ExyonQ counts HTTP requests, not TCP flows,
/// and reports throttling as `429` instead of closing the socket.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientThrottleConfig {
    #[serde(default)]
    pub enabled: bool,
    /// OpenLiteSpeed `softLimit`: concurrent requests allowed to exceed the
    /// soft ceiling only for `grace_seconds` before a ban is armed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_connections_per_ip: Option<u32>,
    /// OpenLiteSpeed `hardLimit`: concurrent requests rejected immediately at
    /// this ceiling (never delayed by the grace window).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_connections_per_ip: Option<u32>,
    /// OpenLiteSpeed `outBandwidth`: response-body bytes per second per IP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bandwidth_bytes_per_second: Option<u64>,
    /// OpenLiteSpeed `inBandwidth`: request-body bytes per second per IP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_bandwidth_bytes_per_second: Option<u64>,
    /// OpenLiteSpeed `staticReqPerSec`: static requests per second per IP.
    /// The excess is deferred to the next one-second window, not rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_requests_per_second: Option<u32>,
    /// OpenLiteSpeed `dynReqPerSec`: dynamic (proxy/FastCGI) requests per
    /// second per IP. The excess is deferred, not rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_requests_per_second: Option<u32>,
    /// OpenLiteSpeed `blockBadReq`: ban clients that repeatedly send
    /// malformed HTTP requests during `ban_seconds`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub block_bad_req: bool,
    /// OpenLiteSpeed `banPeriod`: how long a soft-limit or malformed-request
    /// offender stays banned. Required when `block_bad_req` is enabled.
    #[serde(default)]
    pub ban_seconds: u64,
    /// OpenLiteSpeed `gracePeriod`: how long the soft limit may be exceeded
    /// before `ban_seconds` is armed.
    #[serde(default)]
    pub grace_seconds: u64,
}

impl ClientThrottleConfig {
    pub fn is_default(&self) -> bool {
        !self.enabled
            && self.soft_connections_per_ip.is_none()
            && self.max_connections_per_ip.is_none()
            && self.bandwidth_bytes_per_second.is_none()
            && self.input_bandwidth_bytes_per_second.is_none()
            && self.static_requests_per_second.is_none()
            && self.dynamic_requests_per_second.is_none()
            && !self.block_bad_req
            && self.ban_seconds == 0
            && self.grace_seconds == 0
    }
}

/// 0.5: one `limit_req_zone` (key is `ip`, `host`, `route`, `user`,
/// `header:<name>`, or `cookie:<name>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitZoneConfig {
    pub key: String,
    #[serde(default = "default_rps")]
    pub requests_per_second: u32,
    #[serde(default = "default_burst")]
    pub burst: u32,
}

/// Per-route nginx `limit_req` policy. `burst` and `delay` are parsed values;
/// `None` means the zone default for `burst` and nginx's default `delay=0`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitRouteOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burst: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<u32>,
    #[serde(default)]
    pub nodelay: bool,
}

/// A complete nginx `limit_req` policy for one route. The settings mirror the
/// `limit_req_status`, `limit_req_dry_run` and `limit_req_log_level` directives
/// inherited by the location that declares the limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitRoutePolicy {
    pub limits: Vec<RateLimitLimit>,
    #[serde(default = "default_rate_limit_status")]
    pub status: u16,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_rate_limit_log_level")]
    pub log_level: String,
}

/// One `limit_req zone=...` binding inside a route policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitLimit {
    pub zone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burst: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<u32>,
    #[serde(default)]
    pub nodelay: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_second: default_rps(),
            burst: default_burst(),
            zones: std::collections::BTreeMap::new(),
            routes: std::collections::BTreeMap::new(),
            route_options: std::collections::BTreeMap::new(),
            route_limits: std::collections::BTreeMap::new(),
            client: ClientThrottleConfig::default(),
        }
    }
}

fn default_false() -> bool {
    false
}

fn default_metrics_path() -> String {
    "/metrics".to_string()
}

fn default_health_path() -> String {
    // Cap054: core `handle_request` owns GET /health as liveness body "ok" (WS5).
    // Metrics JSON health must not collide with that probe path.
    "/exyonq-metrics-health".to_string()
}

fn default_min_bytes() -> usize {
    256
}

fn default_compression_level() -> u32 {
    1
}

fn default_rps() -> u32 {
    100
}

fn default_burst() -> u32 {
    200
}

fn default_rate_limit_status() -> u16 {
    429
}

fn default_rate_limit_log_level() -> String {
    "error".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn external_auth_is_exclusive_and_validated() {
        let mut routes = BTreeMap::new();
        routes.insert(
            "api".to_string(),
            AuthRouteConfig {
                realm: String::new(),
                user_file: String::new(),
                auth_request: Some(AuthRequestConfig {
                    endpoint: "http://127.0.0.1:9000/verify".into(),
                    timeout_ms: 2_000,
                }),
            },
        );
        AuthConfig { routes }.validate().expect("external auth");

        let mut config = AuthConfig {
            routes: BTreeMap::new(),
        };
        config.routes.insert(
            "api".into(),
            AuthRouteConfig {
                realm: "basic".into(),
                user_file: "/tmp/users".into(),
                auth_request: Some(AuthRequestConfig {
                    endpoint: "http://127.0.0.1:9000/verify".into(),
                    timeout_ms: 2_000,
                }),
            },
        );
        assert!(config.validate().is_err(), "mechanisms must not combine");
    }

    #[test]
    fn external_auth_rejects_relative_and_unknown_scheme() {
        for endpoint in [
            "/verify",
            "ftp://127.0.0.1/verify",
            "http:///verify",
            "https://127.0.0.1/verify",
            "http://user:pass@127.0.0.1/verify",
        ] {
            let mut routes = BTreeMap::new();
            routes.insert(
                "api".to_string(),
                AuthRouteConfig {
                    realm: String::new(),
                    user_file: String::new(),
                    auth_request: Some(AuthRequestConfig {
                        endpoint: endpoint.into(),
                        timeout_ms: 2_000,
                    }),
                },
            );
            assert!(
                AuthConfig { routes }.validate().is_err(),
                "{endpoint} must fail closed"
            );
        }
    }
}
