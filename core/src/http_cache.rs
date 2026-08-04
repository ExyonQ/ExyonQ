//! Plan 10b cache telemetry helpers (core shell only — no store logic).
//!
//! Bench-environment response header injection (`EXYONQ_BENCH_CACHE_HEADERS`) was
//! removed for project integrity: production responses must not gain or lose
//! headers based on benchmark/demo environment variables.

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::Response;

type HttpBoxBody = BoxBody<Bytes, hyper::Error>;

/// Always false — env-gated bench header injection is retired.
pub fn plan10b_cache_headers_enabled() -> bool {
    false
}

/// No-op: does not mutate response headers (integrity remediation phase 1).
pub fn apply_plan10b_cache_headers(_response: &mut Response<HttpBoxBody>, _outcome: &str) {}
