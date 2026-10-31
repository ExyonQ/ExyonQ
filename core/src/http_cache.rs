/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
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
