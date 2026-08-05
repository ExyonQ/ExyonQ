//! WC7C lab composition — Redis coordination under `EXYONQ_L2_COORD_LAB=1`.
//!
//! Redis types stay in this CLI module + `exyonq-cache-redis` only (not core).

use exyonq_cache_redis::{
    EventSigningKeys, RedisCoordConfig, RedisCoordSecrets, RedisCoordinationProvider, ReplayPolicy,
};
use exyonq_core::lab_coord_hooks::{
    install_lab_purge_port_builder, install_lab_subscriber_starter,
};
use exyonq_core::{cache_purge_port, reload::SharedServerState};
use exyonq_module_api::{
    invalidation_event_to_purge_op, GenerationStore, InvalidationEvent, InvalidationPublisher,
    InvalidationSubscriber,
};
use std::sync::Arc;
use std::time::Duration;

struct LabPublisher(RedisCoordinationProvider);
struct LabSubscriber(RedisCoordinationProvider);
struct LabGeneration(RedisCoordinationProvider);

impl InvalidationPublisher for LabPublisher {
    fn publish(
        &self,
        event: InvalidationEvent,
    ) -> Result<(), exyonq_module_api::CoordinationError> {
        self.0.publish(event)
    }
}

impl InvalidationSubscriber for LabSubscriber {
    fn try_recv(&self) -> Result<Option<InvalidationEvent>, exyonq_module_api::CoordinationError> {
        self.0.try_recv()
    }
    fn stop(&self) {
        self.0.stop()
    }
}

impl GenerationStore for LabGeneration {
    fn get_generation(&self, site_id: u64) -> Result<u64, exyonq_module_api::CoordinationError> {
        self.0.get_generation(site_id)
    }
    fn advance_generation(
        &self,
        site_id: u64,
        to: u64,
    ) -> Result<u64, exyonq_module_api::CoordinationError> {
        self.0.advance_generation(site_id, to)
    }
}

/// Install purge publish + apply loop if lab env is set. No-op otherwise.
pub fn maybe_install_l2_coord_lab(app: &exyonq_config_ir::AppConfig) -> anyhow::Result<()> {
    let lab = std::env::var("EXYONQ_L2_COORD_LAB")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !lab {
        return Ok(());
    }

    let dc = &app.full_page_cache.distributed_cache;
    if !dc.enabled || dc.provider != "redis" {
        tracing::warn!(
            "EXYONQ_L2_COORD_LAB=1 but distributed_cache.provider!=redis or disabled; skip"
        );
        return Ok(());
    }

    let endpoint = if !dc.redis.endpoint.is_empty() {
        dc.redis.endpoint.clone()
    } else {
        std::env::var("EXYONQ_REDIS_COORD_URL").unwrap_or_default()
    };
    if endpoint.is_empty() {
        anyhow::bail!("L2 lab: redis endpoint required (IR or EXYONQ_REDIS_COORD_URL)");
    }

    let node_id = std::env::var("EXYONQ_NODE_ID").unwrap_or_else(|_| "node".into());
    let deployment_id =
        std::env::var("EXYONQ_L2_DEPLOYMENT_ID").unwrap_or_else(|_| "wc7c-lab".into());
    let namespace = if dc.redis.namespace.is_empty() {
        format!("exyonq:fpc:v1:{deployment_id}")
    } else {
        dc.redis.namespace.clone()
    };

    let site_ids: Vec<u64> = std::env::var("EXYONQ_L2_KNOWN_SITE_IDS")
        .ok()
        .map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![1]);

    let cfg = RedisCoordConfig {
        endpoint,
        deployment_id,
        namespace,
        node_id: node_id.clone(),
        connect_timeout: Duration::from_millis(dc.redis.connect_timeout_ms.max(1)),
        command_timeout: Duration::from_millis(dc.redis.command_timeout_ms.max(1)),
        reconnect_min_backoff: Duration::from_millis(dc.redis.reconnect_min_backoff_ms.max(1)),
        reconnect_max_backoff: Duration::from_millis(dc.redis.reconnect_max_backoff_ms.max(1)),
        stream_maxlen: dc.redis.stream_maxlen.max(1),
        max_pending_events: dc.max_pending_events.max(1),
        max_event_bytes: dc.max_event_bytes.max(256),
        invalidation_enabled: dc.invalidation_enabled,
        generation_enabled: dc.generation_enabled,
        known_site_ids: site_ids,
        replay: ReplayPolicy {
            max_age_ms: std::env::var("EXYONQ_L2_EVENT_MAX_AGE_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(dc.security.replay_window_ms),
            max_future_skew_ms: std::env::var("EXYONQ_L2_EVENT_MAX_FUTURE_SKEW_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(dc.security.max_clock_skew_ms),
        },
    };

    // IR security paths → env for EventSigningKeys::from_env (FILE precedence).
    apply_ir_security_env(&dc.security);

    let keys = EventSigningKeys::from_env().map_err(|_| {
        anyhow::anyhow!("L2 lab: HMAC keys required (IR security paths or EXYONQ_L2_EVENT_HMAC_*)")
    })?;
    let secrets = RedisCoordSecrets::from_env();
    let provider = RedisCoordinationProvider::connect(cfg, secrets, keys)
        .map_err(|e| anyhow::anyhow!("L2 lab Redis connect failed: {:?}", e.reason))?;
    provider
        .start_subscriber()
        .map_err(|e| anyhow::anyhow!("L2 lab subscriber start failed: {:?}", e.reason))?;

    let pub_arc: Arc<dyn InvalidationPublisher> = Arc::new(LabPublisher(provider.clone()));
    let gen_store: Arc<dyn GenerationStore> = Arc::new(LabGeneration(provider.clone()));
    let provider_for_reconcile = provider.clone();
    let node_for_purge = node_id.clone();
    let inv_enabled = dc.invalidation_enabled;

    install_lab_purge_port_builder(Arc::new(move |shared: SharedServerState| {
        cache_purge_port::purge_port_with_coordination(
            shared,
            Some(Arc::clone(&pub_arc)),
            node_for_purge.clone(),
            inv_enabled,
        )
    }));

    install_lab_subscriber_starter(Arc::new(move |shared: SharedServerState| {
        let port = cache_purge_port::purge_port(shared);
        let sub = LabSubscriber(provider_for_reconcile.clone());
        let gen_store = Arc::clone(&gen_store);
        let recon = provider_for_reconcile.clone();
        std::thread::Builder::new()
            .name("exyonq-l2-apply".into())
            .spawn(move || loop {
                match sub.try_recv() {
                    Ok(Some(ev)) => {
                        if let Ok(Some(apply)) = recon.accept_received(ev, true) {
                            let site = apply.site_id;
                            let gen = apply.generation;
                            if let Ok(op) = invalidation_event_to_purge_op(&apply) {
                                let _ = port.purge(op);
                            }
                            let _ = gen_store.advance_generation(site, gen);
                        }
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
                if recon.reconcile_needed() {
                    let _ = recon.reconcile_known_sites();
                }
            })
            .ok();
        tracing::info!("L2 coordination apply loop started (lab)");
    }));

    tracing::info!(%node_id, "L2 Redis coordination lab composition installed");
    Ok(())
}

fn apply_ir_security_env(sec: &exyonq_config_ir::DistributedCacheSecurityConfig) {
    if !sec.active_key_id.is_empty() {
        std::env::set_var("EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_ID", &sec.active_key_id);
    }
    if !sec.active_key_file.is_empty() {
        std::env::set_var("EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_FILE", &sec.active_key_file);
    }
    if !sec.previous_key_id.is_empty() {
        std::env::set_var("EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY_ID", &sec.previous_key_id);
    }
    if !sec.previous_key_file.is_empty() {
        std::env::set_var(
            "EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY_FILE",
            &sec.previous_key_file,
        );
    }
}
