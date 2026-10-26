//! WC2B — full-page cache IR (lookup gate; default off).

use serde::{Deserialize, Serialize};

fn default_false() -> bool {
    false
}

fn default_fpc_max_entries() -> usize {
    10_000
}

fn default_fpc_max_total_bytes() -> usize {
    64 * 1024 * 1024
}

fn default_fpc_ttl_seconds() -> u64 {
    30
}

fn default_fpc_max_ttl_seconds() -> u64 {
    3600
}

fn default_fpc_max_object_bytes() -> usize {
    1024 * 1024
}

/// Default FPC namespace (isolated from Plan 12 static/proxy/fcgi 1–3).
fn default_fpc_namespace() -> u16 {
    4
}

/// `[full_page_cache]` — public FPC L1 (WC2B lookup + WC2C insert).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullPageCacheConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_fpc_max_entries")]
    pub max_entries: usize,
    #[serde(default = "default_fpc_max_total_bytes")]
    pub max_total_bytes: usize,
    #[serde(default = "default_fpc_max_object_bytes")]
    pub max_object_bytes: usize,
    #[serde(default = "default_fpc_ttl_seconds")]
    pub default_ttl_seconds: u64,
    #[serde(default = "default_fpc_max_ttl_seconds")]
    pub max_ttl_seconds: u64,
    /// Explicit cache namespace; must be non-zero when `enabled`.
    #[serde(default = "default_fpc_namespace")]
    pub namespace: u16,
    /// WC7B1 — optional L2 coordination (default off; no Redis/Lux URLs here).
    #[serde(default)]
    pub distributed_cache: DistributedCacheConfig,
    /// NG-15 — request material that shapes the cache key (`proxy_cache_key`).
    #[serde(default, skip_serializing_if = "FpcKeyConfig::is_empty")]
    pub key: FpcKeyConfig,
    /// NG-15 — request material that forces a bypass.
    #[serde(default, skip_serializing_if = "FpcBypassConfig::is_empty")]
    pub bypass: FpcBypassConfig,
    /// NG-16: keep expired entries this many seconds so a failing backend can
    /// be answered with the stale representation (`stale-if-error`).
    /// 0 = off (expired entries are dropped on lookup).
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub stale_if_error_secs: u64,
    /// OL-01: cookies that mark a request as **private** (logged-in session).
    /// A request carrying any of them bypasses the shared cache entirely —
    /// ExyonQ never stores or serves it from FPC. Empty = only the built-in
    /// hard-deny list applies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub private_cookies: Vec<String>,
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

/// NG-15: `[full_page_cache.key]` — values folded into the cache key.
///
/// `query` names are the only query params that may reach the key (any other
/// functional param bypasses). `cookies` are the only cookies allowed through
/// the eligibility gate, and their values enter the key. `headers` values enter
/// the key and allow a matching response `Vary`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FpcKeyConfig {
    #[serde(default)]
    pub query: Vec<String>,
    #[serde(default)]
    pub cookies: Vec<String>,
    #[serde(default)]
    pub headers: Vec<String>,
}

impl FpcKeyConfig {
    /// Empty key config must not perturb the canonical config fingerprint.
    pub fn is_empty(&self) -> bool {
        self.query.is_empty() && self.cookies.is_empty() && self.headers.is_empty()
    }
}

/// NG-15: `[full_page_cache.bypass]` — presence of any listed item bypasses FPC.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FpcBypassConfig {
    #[serde(default)]
    pub cookies: Vec<String>,
    #[serde(default)]
    pub headers: Vec<String>,
}

impl FpcBypassConfig {
    /// Empty bypass config must not perturb the canonical config fingerprint.
    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty() && self.headers.is_empty()
    }
}

/// `[full_page_cache.distributed_cache]` — WC7B1/B2/WC7D coordination flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistributedCacheConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// `local` | `redis` — composition selects adapter; default empty = unset.
    #[serde(default)]
    pub provider: String,
    #[serde(default = "default_false")]
    pub invalidation_enabled: bool,
    #[serde(default = "default_false")]
    pub generation_enabled: bool,
    #[serde(default = "default_max_pending_events")]
    pub max_pending_events: usize,
    #[serde(default = "default_max_event_bytes")]
    pub max_event_bytes: usize,
    #[serde(default = "default_dedup_capacity")]
    pub dedup_capacity: usize,
    #[serde(default = "default_dedup_ttl_ms")]
    pub dedup_ttl_ms: u64,
    /// Redis provider settings (ignored unless `provider = "redis"`).
    #[serde(default)]
    pub redis: DistributedCacheRedisConfig,
    /// HMAC / replay security paths (secrets via `_file` only — never inline).
    #[serde(default)]
    pub security: DistributedCacheSecurityConfig,
}

/// `[full_page_cache.distributed_cache.redis]` — WC7B2/WC7D; no secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistributedCacheRedisConfig {
    /// `redis://host:port/` — password via `EXYONQ_REDIS_COORD_PASSWORD` only.
    #[serde(default)]
    pub endpoint: String,
    #[serde(default = "default_redis_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_redis_command_timeout_ms")]
    pub command_timeout_ms: u64,
    #[serde(default = "default_redis_reconnect_min_ms")]
    pub reconnect_min_backoff_ms: u64,
    #[serde(default = "default_redis_reconnect_max_ms")]
    pub reconnect_max_backoff_ms: u64,
    #[serde(default = "default_redis_namespace")]
    pub namespace: String,
    #[serde(default = "default_redis_stream_maxlen")]
    pub stream_maxlen: usize,
}

/// `[full_page_cache.distributed_cache.security]` — WC7D; paths only, no secret bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistributedCacheSecurityConfig {
    #[serde(default)]
    pub active_key_id: String,
    /// Absolute path to HMAC active key file (`EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_FILE`).
    #[serde(default)]
    pub active_key_file: String,
    #[serde(default)]
    pub previous_key_id: String,
    #[serde(default)]
    pub previous_key_file: String,
    /// Max future clock skew for `issued_at` (ms). Maps to ReplayPolicy.max_future_skew_ms.
    #[serde(default = "default_max_clock_skew_ms")]
    pub max_clock_skew_ms: u64,
    /// Max event age / replay window (ms). Maps to ReplayPolicy.max_age_ms.
    #[serde(default = "default_replay_window_ms")]
    pub replay_window_ms: u64,
}

fn default_max_clock_skew_ms() -> u64 {
    60_000
}
fn default_replay_window_ms() -> u64 {
    300_000
}

impl Default for DistributedCacheSecurityConfig {
    fn default() -> Self {
        Self {
            active_key_id: String::new(),
            active_key_file: String::new(),
            previous_key_id: String::new(),
            previous_key_file: String::new(),
            max_clock_skew_ms: default_max_clock_skew_ms(),
            replay_window_ms: default_replay_window_ms(),
        }
    }
}

fn default_redis_connect_timeout_ms() -> u64 {
    500
}
fn default_redis_command_timeout_ms() -> u64 {
    500
}
fn default_redis_reconnect_min_ms() -> u64 {
    100
}
fn default_redis_reconnect_max_ms() -> u64 {
    5_000
}
fn default_redis_namespace() -> String {
    "exyonq:fpc:v1".into()
}
fn default_redis_stream_maxlen() -> usize {
    10_000
}

impl Default for DistributedCacheRedisConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            connect_timeout_ms: default_redis_connect_timeout_ms(),
            command_timeout_ms: default_redis_command_timeout_ms(),
            reconnect_min_backoff_ms: default_redis_reconnect_min_ms(),
            reconnect_max_backoff_ms: default_redis_reconnect_max_ms(),
            namespace: default_redis_namespace(),
            stream_maxlen: default_redis_stream_maxlen(),
        }
    }
}

fn default_max_pending_events() -> usize {
    256
}

fn default_max_event_bytes() -> usize {
    8192
}

fn default_dedup_capacity() -> usize {
    1024
}

fn default_dedup_ttl_ms() -> u64 {
    60_000
}

impl Default for DistributedCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: String::new(),
            invalidation_enabled: false,
            generation_enabled: false,
            max_pending_events: default_max_pending_events(),
            max_event_bytes: default_max_event_bytes(),
            dedup_capacity: default_dedup_capacity(),
            dedup_ttl_ms: default_dedup_ttl_ms(),
            redis: DistributedCacheRedisConfig::default(),
            security: DistributedCacheSecurityConfig::default(),
        }
    }
}

impl FullPageCacheConfig {
    /// NG-15: reject contradictory or malformed key/bypass names when enabled.
    pub fn validate(&self) -> Result<(), String> {
        for (field, names) in [
            ("key.query", &self.key.query),
            ("key.cookies", &self.key.cookies),
            ("key.headers", &self.key.headers),
            ("bypass.cookies", &self.bypass.cookies),
            ("bypass.headers", &self.bypass.headers),
        ] {
            for name in names {
                if name.trim().is_empty() {
                    return Err(format!("full_page_cache.{field}: empty name"));
                }
                if name.contains(|c: char| c.is_whitespace() || c == ';' || c == '=') {
                    return Err(format!("full_page_cache.{field}: invalid name {name:?}"));
                }
            }
        }
        for cookie in &self.private_cookies {
            if self
                .key
                .cookies
                .iter()
                .any(|k| k.eq_ignore_ascii_case(cookie))
            {
                return Err(format!(
                    "full_page_cache: cookie {cookie:?} is both private and keyed"
                ));
            }
        }
        for cookie in &self.bypass.cookies {
            if self
                .key
                .cookies
                .iter()
                .any(|k| k.eq_ignore_ascii_case(cookie))
            {
                return Err(format!(
                    "full_page_cache: cookie {cookie:?} is both keyed and bypassed"
                ));
            }
        }
        Ok(())
    }
}

impl Default for FullPageCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries: default_fpc_max_entries(),
            max_total_bytes: default_fpc_max_total_bytes(),
            max_object_bytes: default_fpc_max_object_bytes(),
            default_ttl_seconds: default_fpc_ttl_seconds(),
            max_ttl_seconds: default_fpc_max_ttl_seconds(),
            namespace: default_fpc_namespace(),
            distributed_cache: DistributedCacheConfig::default(),
            key: FpcKeyConfig::default(),
            bypass: FpcBypassConfig::default(),
            stale_if_error_secs: 0,
            private_cookies: Vec::new(),
        }
    }
}
