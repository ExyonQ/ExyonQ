//! Plan 10b bench cache telemetry headers (core shell only — no store logic).

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::{header, Response};

type HttpBoxBody = BoxBody<Bytes, hyper::Error>;

pub fn plan10b_cache_headers_enabled() -> bool {
    std::env::var("EXYONQ_BENCH_CACHE_HEADERS")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

pub fn apply_plan10b_cache_headers(response: &mut Response<HttpBoxBody>, outcome: &str) {
    if !plan10b_cache_headers_enabled() {
        return;
    }
    if let Ok(value) = header::HeaderValue::from_str(outcome) {
        response
            .headers_mut()
            .insert(header::HeaderName::from_static("x-plan10b-cache"), value);
    }
    response.headers_mut().insert(
        header::HeaderName::from_static("x-plan10b-server"),
        header::HeaderValue::from_static("exyonq"),
    );
}
