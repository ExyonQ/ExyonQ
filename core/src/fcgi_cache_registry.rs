//! FastCGI cache hooks registered at composition root (CLI) — no `exyonq-mod-fastcgi` prod dep in core.

use bytes::Bytes;
use exyonq_cache::{CacheKey, CacheLoadOutcome, CacheServeOutcome, ResponseCache, Singleflight};
use exyonq_module_api::{assess_cacheability, CacheRejection, CompiledCachePolicy};
use http::Method;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

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

type ServeFuture<'a> =
    Pin<Box<dyn Future<Output = (hyper::Response<BoxBody>, CacheServeOutcome)> + Send + 'a>>;

pub type ServeFn = for<'a> fn(
    &'a ResponseCache,
    &'a Singleflight,
    CacheKey,
    usize,
    u64,
    &'a Method,
    &'a CompiledCachePolicy,
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = CacheLoadOutcome> + Send>> + Send + 'a>,
) -> ServeFuture<'a>;

#[derive(Clone, Copy)]
pub struct FcgiCacheMetricsFns {
    pub hits: fn() -> u64,
    pub misses: fn() -> u64,
    pub insertions: fn() -> u64,
    pub rejections: fn() -> u64,
    pub reset: fn(),
}

pub struct FcgiCacheHooks {
    pub serve: ServeFn,
    pub metrics: FcgiCacheMetricsFns,
}

static HOOKS: OnceLock<FcgiCacheHooks> = OnceLock::new();

pub fn register_fcgi_cache_hooks(hooks: FcgiCacheHooks) -> Result<(), FcgiCacheHooks> {
    HOOKS.set(hooks)
}

fn hooks() -> &'static FcgiCacheHooks {
    HOOKS
        .get()
        .expect("register_fcgi_cache_hooks must run at CLI startup before serving FastCGI cache")
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
) -> (hyper::Response<BoxBody>, CacheServeOutcome)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = CacheLoadOutcome> + Send + 'static,
{
    let load_box = Box::new(move || {
        let fut = load();
        Box::pin(fut) as Pin<Box<dyn Future<Output = CacheLoadOutcome> + Send>>
    });
    (hooks().serve)(
        cache,
        singleflight,
        key,
        route_idx,
        runtime_generation,
        client_method,
        policy,
        load_box,
    )
    .await
}

/// Build hooks from module function pointers (CLI / integration tests).
pub fn fcgi_cache_hooks_from_module(
    serve: ServeFn,
    metrics: FcgiCacheMetricsFns,
) -> FcgiCacheHooks {
    FcgiCacheHooks { serve, metrics }
}
