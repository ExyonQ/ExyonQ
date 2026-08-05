use crate::key::CacheKey;
use crate::metrics::note_rejection;
use crate::namespace::ServeHooks;
use crate::singleflight::{wait_flight, JoinResult, Singleflight};
use crate::store::{CachedEntry, ResponseCache};
use bytes::Bytes;
use exyonq_module_api::static_dispatch::StaticResourceIdentitySnapshot;
use exyonq_module_api::{filter_storable_headers, CacheRejection, CompiledCachePolicy};
use std::sync::Arc;

/// Neutral load outcome from a backend (policy evaluated by caller before store).
#[derive(Debug)]
pub struct CacheLoadOutcome {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub store: bool,
    pub rejection: Option<CacheRejection>,
    pub static_identity: Option<StaticResourceIdentitySnapshot>,
    pub invalidation_tags: Arc<[String]>,
}

/// Neutral HTTP response returned to the kernel shell (no Hyper types).
#[derive(Debug, Clone)]
pub struct ServedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub head_only: bool,
}

pub fn entry_to_served(entry: &CachedEntry, head_only: bool) -> ServedResponse {
    ServedResponse {
        status: entry.status,
        headers: entry.headers.clone(),
        body: if head_only {
            Bytes::new()
        } else {
            entry.body.clone()
        },
        head_only,
    }
}

struct SingleflightGuard<'a> {
    singleflight: &'a Singleflight,
    key: CacheKey,
}

impl Drop for SingleflightGuard<'_> {
    fn drop(&mut self) {
        self.singleflight.finish(&self.key);
    }
}

/// How a cache serve completed (bench telemetry / plan10b headers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheServeOutcome {
    Hit,
    Miss,
    Bypass,
}

impl CacheServeOutcome {
    pub fn plan10b_label(self) -> &'static str {
        match self {
            Self::Hit => "HIT",
            Self::Miss => "MISS",
            Self::Bypass => "BYPASS",
        }
    }
}

/// Shared cache path: singleflight → lookup → load → optional GET insert → response.
#[allow(clippy::too_many_arguments)]
pub async fn serve_with_cache<F, Fut>(
    cache: &ResponseCache,
    singleflight: &Singleflight,
    key: CacheKey,
    route_idx: usize,
    runtime_generation: u64,
    head_only: bool,
    policy: &CompiledCachePolicy,
    hooks: ServeHooks,
    load: F,
) -> (ServedResponse, CacheServeOutcome)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = CacheLoadOutcome>,
{
    if !singleflight.has_flight(&key) {
        if let Some(cached) = cache.lookup_with_hooks(&key, true, hooks.metrics, hooks.validate_hit)
        {
            return (entry_to_served(&cached, head_only), CacheServeOutcome::Hit);
        }
    }

    match singleflight.join(&key) {
        JoinResult::Follower(flight) => {
            wait_flight(&flight).await;
            for _ in 0..32 {
                if let Some(cached) = cache.get_if_alive(&key, hooks.metrics, hooks.validate_hit) {
                    return (entry_to_served(&cached, head_only), CacheServeOutcome::Hit);
                }
                tokio::task::yield_now().await;
            }
            let load = load().await;
            store_load(
                cache,
                key,
                route_idx,
                runtime_generation,
                policy,
                hooks,
                load,
                head_only,
            )
        }
        JoinResult::Leader(_flight) => {
            let _guard = SingleflightGuard {
                singleflight,
                key: key.clone(),
            };
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
            if let Some(cached) =
                cache.lookup_after_singleflight(&key, hooks.metrics, hooks.validate_hit)
            {
                return (entry_to_served(&cached, head_only), CacheServeOutcome::Hit);
            }
            let load = load().await;
            store_load(
                cache,
                key,
                route_idx,
                runtime_generation,
                policy,
                hooks,
                load,
                head_only,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn store_load(
    cache: &ResponseCache,
    key: CacheKey,
    route_idx: usize,
    runtime_generation: u64,
    policy: &CompiledCachePolicy,
    hooks: ServeHooks,
    load: CacheLoadOutcome,
    head_only: bool,
) -> (ServedResponse, CacheServeOutcome) {
    if !load.store {
        if let Some(reason) = load.rejection {
            note_rejection(reason);
            (hooks.metrics.on_rejection)();
        }
        return (
            ServedResponse {
                status: load.status,
                headers: load.headers,
                body: if head_only { Bytes::new() } else { load.body },
                head_only,
            },
            CacheServeOutcome::Bypass,
        );
    }

    let headers = filter_storable_headers(&load.headers);
    cache.insert_entry(
        key,
        0, // site_id: request-path wiring deferred (WC2 foundation)
        route_idx,
        runtime_generation,
        policy.ttl,
        load.status,
        headers.clone(),
        load.body.clone(),
        load.static_identity,
        hooks.namespace,
        load.invalidation_tags,
        hooks.metrics,
    );

    (
        ServedResponse {
            status: load.status,
            headers,
            body: if head_only { Bytes::new() } else { load.body },
            head_only,
        },
        CacheServeOutcome::Miss,
    )
}
