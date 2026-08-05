//! Redis provider configuration (no secrets in IR defaults).

use crate::auth::ReplayPolicy;
use std::time::Duration;

/// Provider-specific Redis settings (composition / lab). Endpoint may come from env.
#[derive(Debug, Clone)]
pub struct RedisCoordConfig {
    pub endpoint: String,
    /// Deployment identity included in HMAC canonical bytes (not a secret).
    pub deployment_id: String,
    pub namespace: String,
    pub node_id: String,
    pub connect_timeout: Duration,
    pub command_timeout: Duration,
    pub reconnect_min_backoff: Duration,
    pub reconnect_max_backoff: Duration,
    pub stream_maxlen: usize,
    pub max_pending_events: usize,
    pub max_event_bytes: usize,
    pub invalidation_enabled: bool,
    pub generation_enabled: bool,
    /// Sites known locally for reconciliation (bounded list).
    pub known_site_ids: Vec<u64>,
    pub replay: ReplayPolicy,
}

impl Default for RedisCoordConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            deployment_id: "default".into(),
            namespace: "exyonq:fpc:v1".into(),
            node_id: "node".into(),
            connect_timeout: Duration::from_millis(500),
            command_timeout: Duration::from_millis(500),
            reconnect_min_backoff: Duration::from_millis(100),
            reconnect_max_backoff: Duration::from_secs(5),
            stream_maxlen: 10_000,
            max_pending_events: 256,
            max_event_bytes: 8192,
            invalidation_enabled: true,
            generation_enabled: true,
            known_site_ids: Vec::new(),
            replay: ReplayPolicy::default(),
        }
    }
}

impl RedisCoordConfig {
    pub fn stream_key(&self) -> String {
        format!("{}:inv", self.namespace)
    }

    pub fn consumer_group(&self) -> String {
        format!("{}:cg", self.namespace)
    }

    pub fn generation_key(&self, site_id: u64) -> String {
        format!("{}:gen:{}", self.namespace, site_id)
    }
}

/// Secrets from environment / mounts — never logged.
#[derive(Clone, Default)]
pub struct RedisCoordSecrets {
    /// Optional Redis AUTH password (`EXYONQ_REDIS_COORD_PASSWORD`).
    pub password: Option<String>,
}

impl std::fmt::Debug for RedisCoordSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisCoordSecrets")
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

impl RedisCoordSecrets {
    pub fn from_env() -> Self {
        let password = std::env::var("EXYONQ_REDIS_COORD_PASSWORD")
            .ok()
            .filter(|s| !s.is_empty());
        Self { password }
    }
}
