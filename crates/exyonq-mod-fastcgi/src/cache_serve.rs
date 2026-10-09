//! KD4.1 — FastCGI microcache serve orchestration.

use bytes::Bytes;
use exyonq_cache::{
    serve_with_cache, CacheKey, CacheLoadOutcome, CacheServeOutcome, ResponseCache, ServeHooks,
    Singleflight,
};
use exyonq_module_api::{assess_cacheability, CacheRejection, CompiledCachePolicy};
use http::Method;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use std::pin::Pin;

use crate::cache_metrics::{
    note_fcgi_hit, note_fcgi_insertion, note_fcgi_miss, note_fcgi_rejection,
};

pub const FCGI_CACHE_NAMESPACE: exyonq_cache::CacheNamespace = exyonq_cache::CacheNamespace::new(3);

const FCGI_METRICS: exyonq_cache::NamespaceMetrics = exyonq_cache::NamespaceMetrics {
    on_hit: note_fcgi_hit,
    on_miss: note_fcgi_miss,
    on_insert: note_fcgi_insertion,
    on_rejection: note_fcgi_rejection,
};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

/// Backward-compat alias — neutral load type lives in `exyonq-cache`.
pub type FcgiCacheLoad = CacheLoadOutcome;

pub fn prepare_fcgi_cache_load(
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
    max_object_bytes: usize,
    request_headers: &[(String, String)],
) -> CacheLoadOutcome {
    let invalidation_tags = html_menu_tag(&headers);
    if body.len() > max_object_bytes {
        return CacheLoadOutcome {
            status,
            headers,
            body,
            store: false,
            rejection: Some(CacheRejection::BodyTooLarge),
            static_identity: None,
            invalidation_tags,
        };
    }
    let assessment = assess_cacheability(
        "GET",
        request_headers,
        status,
        &headers,
        body.len(),
        max_object_bytes,
    );
    CacheLoadOutcome {
        status,
        headers,
        body,
        store: assessment.is_ok(),
        rejection: assessment.err(),
        static_identity: None,
        invalidation_tags,
    }
}

/// Anonymous HTML is tagged `menu` so a menu edit can drop those pages
/// without emptying the rest of the site cache.
fn html_menu_tag(headers: &[(String, String)]) -> std::sync::Arc<[String]> {
    let html = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && value
                .split(';')
                .next()
                .unwrap_or(value)
                .trim()
                .eq_ignore_ascii_case("text/html")
    });
    if html {
        std::sync::Arc::from([String::from("menu")])
    } else {
        std::sync::Arc::from([])
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
pub async fn serve_fastcgi_with_cache<F, Fut>(
    cache: &ResponseCache,
    singleflight: &Singleflight,
    key: CacheKey,
    route_idx: usize,
    runtime_generation: u64,
    client_method: &Method,
    policy: &CompiledCachePolicy,
    load: F,
) -> (Response<BoxBody>, CacheServeOutcome)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = CacheLoadOutcome>,
{
    let head_only = *client_method == Method::HEAD;
    let hooks = ServeHooks::new(FCGI_CACHE_NAMESPACE, FCGI_METRICS);
    let (served, outcome) = serve_with_cache(
        cache,
        singleflight,
        key,
        route_idx,
        runtime_generation,
        head_only,
        policy,
        hooks,
        || async move { load().await },
    )
    .await;
    (served_to_hyper(served), outcome)
}

/// Composition-root hook — type-erased load closure for core registration without a prod dep.
#[allow(clippy::too_many_arguments)]
pub fn serve_fastcgi_with_cache_hook<'a>(
    cache: &'a ResponseCache,
    singleflight: &'a Singleflight,
    key: CacheKey,
    route_idx: usize,
    runtime_generation: u64,
    client_method: &'a Method,
    policy: &'a CompiledCachePolicy,
    load: Box<
        dyn FnOnce() -> Pin<Box<dyn std::future::Future<Output = CacheLoadOutcome> + Send>>
            + Send
            + 'a,
    >,
) -> Pin<Box<dyn std::future::Future<Output = (Response<BoxBody>, CacheServeOutcome)> + Send + 'a>>
{
    Box::pin(async move {
        serve_fastcgi_with_cache(
            cache,
            singleflight,
            key,
            route_idx,
            runtime_generation,
            client_method,
            policy,
            load,
        )
        .await
    })
}

#[cfg(test)]
mod tests {
    use super::prepare_fcgi_cache_load;
    use bytes::Bytes;

    #[test]
    fn html_is_tagged_menu_and_other_types_are_not() {
        let html = prepare_fcgi_cache_load(
            200,
            vec![("content-type".into(), "text/html; charset=UTF-8".into())],
            Bytes::from_static(b"<p>hi</p>"),
            1024,
            &[],
        );
        assert_eq!(html.invalidation_tags.as_ref(), ["menu"]);
        let css = prepare_fcgi_cache_load(
            200,
            vec![("content-type".into(), "text/css".into())],
            Bytes::from_static(b"body{}"),
            1024,
            &[],
        );
        assert!(css.invalidation_tags.is_empty());
    }
}
