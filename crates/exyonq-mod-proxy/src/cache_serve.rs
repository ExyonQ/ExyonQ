//! KD4.1 — proxy microcache serve orchestration (module-owned policy path).
//!
//! SEC-PROXY-003: `ProxyCacheLoad::Passthrough` streams the upstream/prefix body
//! without a second unbounded `collect()`.

use bytes::Bytes;
use exyonq_cache::{
    serve_with_cache, CacheKey, CacheLoadOutcome, ResponseCache, ServeHooks, ServedResponse,
    Singleflight,
};
use exyonq_module_api::CompiledCachePolicy;
use http::Method;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use std::sync::Arc;

use crate::cache_metrics::{
    note_proxy_hit, note_proxy_insertion, note_proxy_miss, note_proxy_rejection,
};
use crate::proxy_cache::ProxyCacheLoad;

pub const PROXY_CACHE_NAMESPACE: exyonq_cache::CacheNamespace =
    exyonq_cache::CacheNamespace::new(2);

const PROXY_METRICS: exyonq_cache::NamespaceMetrics = exyonq_cache::NamespaceMetrics {
    on_hit: note_proxy_hit,
    on_miss: note_proxy_miss,
    on_insert: note_proxy_insertion,
    on_rejection: note_proxy_rejection,
};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

/// Materialized loads only — Passthrough must not be converted via collect.
fn materialized_to_outcome(
    body: Bytes,
    status: StatusCode,
    headers: Vec<(String, String)>,
    cacheable: bool,
    rejection: Option<exyonq_module_api::CacheRejection>,
) -> CacheLoadOutcome {
    CacheLoadOutcome {
        status: status.as_u16(),
        headers,
        body,
        store: cacheable,
        rejection,
        static_identity: None,
        invalidation_tags: Arc::from([]),
    }
}

fn headers_from_parts(parts: &http::response::Parts) -> Vec<(String, String)> {
    parts
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

pub fn served_to_hyper(served: ServedResponse) -> Response<BoxBody> {
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
pub async fn serve_proxy_with_cache<F, Fut>(
    cache: &ResponseCache,
    singleflight: &Singleflight,
    key: CacheKey,
    route_idx: usize,
    runtime_generation: u64,
    client_method: &Method,
    policy: &CompiledCachePolicy,
    load_fn: F,
) -> Response<BoxBody>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ProxyCacheLoad>,
{
    // oneshot avoids naming BoxBody on extra lines (GATE-DEP-007 budget).
    let (tx, rx) = tokio::sync::oneshot::channel();
    let head_only = *client_method == Method::HEAD;
    let hooks = ServeHooks::new(PROXY_CACHE_NAMESPACE, PROXY_METRICS);
    let (served, _outcome) = serve_with_cache(
        cache,
        singleflight,
        key,
        route_idx,
        runtime_generation,
        head_only,
        policy,
        hooks,
        move || {
            let tx = tx;
            async move {
                match load_fn().await {
                    ProxyCacheLoad::Materialized {
                        body,
                        status,
                        headers,
                        cacheable,
                        rejection,
                    } => {
                        drop(tx);
                        materialized_to_outcome(body, status, headers, cacheable, rejection)
                    }
                    ProxyCacheLoad::Passthrough {
                        response,
                        rejection,
                    } => {
                        let status = response.status().as_u16();
                        let (parts, body) = response.into_parts();
                        let headers = headers_from_parts(&parts);
                        // SEC-PROXY-003: never collect — stream prefix + remainder.
                        if head_only {
                            drop(body);
                            drop(tx);
                        } else {
                            let _ = tx.send(body);
                        }
                        CacheLoadOutcome {
                            status,
                            headers,
                            body: Bytes::new(),
                            store: false,
                            rejection: Some(rejection),
                            static_identity: None,
                            invalidation_tags: Arc::from([]),
                        }
                    }
                }
            }
        },
    )
    .await;

    match rx.await {
        Ok(body) => {
            let status =
                StatusCode::from_u16(served.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let mut builder = Response::builder().status(status);
            for (name, value) in &served.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            builder.body(body).expect("valid streaming passthrough")
        }
        Err(_) => served_to_hyper(served),
    }
}
