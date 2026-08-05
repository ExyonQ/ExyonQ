//! KD4.1 — static microcache serve orchestration.

use bytes::Bytes;
use exyonq_cache::{
    serve_with_cache, CacheKey, CacheLoadOutcome, HitValidation, ResponseCache, ServeHooks,
    Singleflight,
};
use exyonq_module_api::static_dispatch::StaticResourceIdentitySnapshot;
use exyonq_module_api::{assess_cacheability, CompiledCachePolicy};
use http::Method;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use std::sync::Arc;

use crate::static_cache::{
    canonical_path_for_snapshot, note_invalidation, note_revalidation_failure,
    note_revalidation_success, snapshot_matches_current,
};

pub const STATIC_CACHE_NAMESPACE: exyonq_cache::CacheNamespace =
    exyonq_cache::CacheNamespace::new(1);

const STATIC_METRICS: exyonq_cache::NamespaceMetrics = exyonq_cache::NamespaceMetrics {
    on_hit: crate::static_cache::note_cache_hit,
    on_miss: crate::static_cache::note_cache_miss,
    on_insert: || {},
    on_rejection: || {},
};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

pub struct StaticCacheLoad {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub identity: Option<StaticResourceIdentitySnapshot>,
}

pub fn validate_static_cache_hit(entry: &exyonq_cache::CachedEntry) -> HitValidation {
    let Some(identity) = entry.static_identity() else {
        return HitValidation::Accept;
    };
    if snapshot_matches_current(identity) {
        note_revalidation_success();
        HitValidation::Accept
    } else {
        note_revalidation_failure();
        note_invalidation();
        HitValidation::Reject {
            entry_id: entry.entry_id(),
        }
    }
}

fn static_load_to_outcome(load: StaticCacheLoad, max_object_bytes: usize) -> CacheLoadOutcome {
    let invalidation_tags: Arc<[String]> = load
        .identity
        .as_ref()
        .map(|id| {
            Arc::from([canonical_path_for_snapshot(id)
                .to_string_lossy()
                .into_owned()])
        })
        .unwrap_or_else(|| Arc::from([]));
    let assessment = assess_cacheability(
        "GET",
        &[],
        load.status,
        &load.headers,
        load.body.len(),
        max_object_bytes,
    );
    CacheLoadOutcome {
        status: load.status,
        headers: load.headers,
        body: load.body,
        store: assessment.is_ok(),
        rejection: assessment.err(),
        static_identity: load.identity,
        invalidation_tags,
    }
}

pub fn served_to_hyper(served: exyonq_cache::ServedResponse) -> Response<BoxBody> {
    let status = StatusCode::from_u16(served.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder().status(status);
    for (name, value) in &served.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    if served.head_only {
        builder
            .body(
                http_body_util::Empty::<Bytes>::new()
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .expect("valid head response")
    } else {
        builder
            .body(
                Full::from(served.body)
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .expect("valid response")
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn serve_static_with_cache<F, Fut>(
    cache: &ResponseCache,
    singleflight: &Singleflight,
    key: CacheKey,
    route_idx: usize,
    runtime_generation: u64,
    client_method: &Method,
    policy: &CompiledCachePolicy,
    load: F,
) -> Response<BoxBody>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = StaticCacheLoad>,
{
    let head_only = *client_method == Method::HEAD;
    let hooks = ServeHooks {
        namespace: STATIC_CACHE_NAMESPACE,
        metrics: STATIC_METRICS,
        validate_hit: Some(validate_static_cache_hit),
    };
    let max_object_bytes = policy.max_object_bytes;
    let (served, _outcome) = serve_with_cache(
        cache,
        singleflight,
        key,
        route_idx,
        runtime_generation,
        head_only,
        policy,
        hooks,
        || async move { static_load_to_outcome(load().await, max_object_bytes) },
    )
    .await;
    served_to_hyper(served)
}

pub fn invalidate_static_path(cache: &ResponseCache, path: &std::path::Path) {
    let tag = crate::canonical_path_for_invalidation(path)
        .to_string_lossy()
        .into_owned();
    cache.invalidate_tag(&tag);
}
