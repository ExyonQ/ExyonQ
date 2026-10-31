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
//! WC2B/WC2C — full-page cache request eligibility + L1 lookup (insert via `fpc_store`).

use crate::server::state::ServerState;
use bytes::Bytes;
use exyonq_cache::{
    build_storage_cache_key, entry_to_served, normalize_host, note_fpc_bypass, note_fpc_hit,
    note_fpc_lookup, note_fpc_lookup_error, note_fpc_miss, CacheKey, CacheKeyParts, CachedEntry,
    ResponseCache,
};
use exyonq_module_api::{fpc_request_evaluate, fpc_vary_variant, FpcRequestPolicy};
use exyonq_runtime_plan::BackendId;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::{header, Response, StatusCode};
use std::sync::Arc;
use std::time::Duration;

type HttpBoxBody = BoxBody<Bytes, hyper::Error>;

/// Context retained across a MISS so WC2C can insert after the origin response is complete.
#[derive(Clone)]
pub struct FpcMissContext {
    pub cache: Arc<ResponseCache>,
    pub key: CacheKey,
    pub site_id: u64,
    pub namespace: u16,
    pub route_idx: usize,
    pub runtime_generation: u64,
    pub max_object_bytes: usize,
    pub default_ttl: Duration,
    pub max_ttl: Duration,
    /// NG-15: headers that may appear in a stored response's `Vary`.
    pub vary_allowlist: Vec<String>,
    /// NG-16: invalidation tags bound to the route (purge by tag).
    pub tags: Arc<[String]>,
}

/// Result of the FPC gate after BackendId is known.
pub enum FpcGateResult {
    /// Serve from L1; backend must not run.
    Hit(Response<HttpBoxBody>),
    /// Eligible request with no entry — continue backend, then optional insert.
    Miss(FpcMissContext),
    /// FPC enabled but this request skipped — attach `X-Cache: bypass` + reason.
    Bypass { reason: &'static str },
    /// FPC not configured / disabled for this process — no X-Cache header.
    Skip,
}

/// FPC eligibility + L1 lookup. Never inserts.
pub fn fpc_gate(
    state: &ServerState,
    route_idx: usize,
    method: &hyper::Method,
    uri: &hyper::Uri,
    host: Option<&str>,
    request_headers: &[(String, String)],
) -> FpcGateResult {
    let fpc = &state.snapshot.full_page_cache;
    if !fpc.enabled {
        return FpcGateResult::Skip;
    }
    let Some(cache) = state.fpc_cache.as_ref() else {
        note_fpc_lookup_error();
        return bypass("lookup_error");
    };
    if fpc.namespace == 0 {
        return bypass("namespace");
    }
    let Some(site_id) = fpc
        .route_site_ids
        .get(route_idx)
        .copied()
        .filter(|&id| id != 0)
    else {
        return bypass("no_site");
    };
    let Some(backend_id) = state.snapshot.route_backend_id(route_idx) else {
        return bypass("no_backend");
    };
    if backend_id == BackendId::INVALID {
        return bypass("no_backend");
    }

    note_fpc_lookup();

    let path = uri.path();
    let query = uri.query().unwrap_or("");
    // NG-15: `[full_page_cache.key]` / `.bypass` drive eligibility and the key.
    let key_cookies: Vec<&str> = fpc.key_cookies.iter().map(String::as_str).collect();
    let key_headers: Vec<&str> = fpc.key_headers.iter().map(String::as_str).collect();
    let key_query: Vec<&str> = fpc.key_query.iter().map(String::as_str).collect();
    let bypass_cookies: Vec<&str> = fpc.bypass_cookies.iter().map(String::as_str).collect();
    let bypass_headers: Vec<&str> = fpc.bypass_headers.iter().map(String::as_str).collect();
    let private_cookies: Vec<&str> = fpc.private_cookies.iter().map(String::as_str).collect();
    let policy = FpcRequestPolicy {
        safe_cookie_allowlist: &key_cookies,
        functional_query_allowlist: &key_query,
        bypass_cookies: &bypass_cookies,
        bypass_headers: &bypass_headers,
        private_cookies: &private_cookies,
    };
    let (canon_path, canon_query) =
        match fpc_request_evaluate(method.as_str(), path, query, request_headers, &policy) {
            Ok(v) => v,
            Err(reason) => return bypass(reason.as_str()),
        };
    let vary = fpc_vary_variant(request_headers, &key_cookies, &key_headers);

    let scheme = if state.config.primary_server().tls.is_some() {
        "https"
    } else {
        "http"
    };
    let key = build_storage_cache_key(CacheKeyParts {
        site_id,
        namespace: fpc.namespace,
        backend_id: backend_id.index(),
        runtime_generation: state.generation,
        policy_generation: 0,
        route_idx,
        method: "GET".into(),
        scheme: scheme.into(),
        host: normalize_host(host),
        path: canon_path,
        query: canon_query,
        content_encoding: "identity".into(),
        vary,
    });

    let miss_ctx = FpcMissContext {
        cache: Arc::clone(cache),
        key: key.clone(),
        site_id,
        namespace: fpc.namespace,
        route_idx,
        runtime_generation: state.generation,
        max_object_bytes: fpc.max_object_bytes,
        default_ttl: Duration::from_secs(fpc.default_ttl_secs.max(1)),
        max_ttl: Duration::from_secs(fpc.max_ttl_secs.max(1)),
        vary_allowlist: key_headers.iter().map(|h| (*h).to_string()).collect(),
        tags: Arc::from(
            fpc.route_cache_tags
                .get(route_idx)
                .map(|tags| tags.to_vec())
                .unwrap_or_default(),
        ),
    };

    let entry = match cache.lookup(&key) {
        Some(entry) => entry,
        None => {
            note_fpc_miss();
            return FpcGateResult::Miss(miss_ctx);
        }
    };

    note_fpc_hit();
    let head_only = *method == hyper::Method::HEAD;
    FpcGateResult::Hit(served_entry_to_response(entry.as_ref(), head_only))
}

fn bypass(reason: &'static str) -> FpcGateResult {
    note_fpc_bypass();
    FpcGateResult::Bypass { reason }
}

/// Backward-compatible helper: HIT → `Some(response)`, otherwise `None`.
pub fn try_fpc_lookup_hit(
    state: &ServerState,
    route_idx: usize,
    method: &hyper::Method,
    uri: &hyper::Uri,
    host: Option<&str>,
    request_headers: &[(String, String)],
) -> Option<Response<HttpBoxBody>> {
    match fpc_gate(state, route_idx, method, uri, host, request_headers) {
        FpcGateResult::Hit(response) => Some(response),
        FpcGateResult::Miss(_) | FpcGateResult::Bypass { .. } | FpcGateResult::Skip => None,
    }
}

/// Q-04 / ADR-057: observable cache outcome on FPC-served responses.
pub const X_CACHE_HEADER: &str = "x-cache";
/// Companion reason when `X-Cache: bypass`.
pub const X_CACHE_REASON_HEADER: &str = "x-cache-reason";

/// Insert or replace `X-Cache` (lowercase wire name). Values: `hit` | `miss` | `bypass`.
pub fn apply_x_cache(headers: &mut header::HeaderMap, value: &str) {
    if let Ok(v) = header::HeaderValue::from_str(value) {
        headers.insert(header::HeaderName::from_static(X_CACHE_HEADER), v);
    }
}

/// `X-Cache: bypass` + `X-Cache-Reason: <token>`.
pub fn apply_x_cache_bypass(headers: &mut header::HeaderMap, reason: &str) {
    apply_x_cache(headers, "bypass");
    if let Ok(v) = header::HeaderValue::from_str(reason) {
        headers.insert(header::HeaderName::from_static(X_CACHE_REASON_HEADER), v);
    }
}

/// Apply bypass headers when the gate returned [`FpcGateResult::Bypass`].
pub fn maybe_apply_bypass<B>(response: &mut Response<B>, reason: Option<&str>) {
    if let Some(r) = reason {
        apply_x_cache_bypass(response.headers_mut(), r);
    }
}

/// NG-16: serve the stale representation of `ctx.key` when the backend failed.
/// `None` when there is no entry inside the stale window.
pub fn stale_response(
    ctx: &FpcMissContext,
    method: &hyper::Method,
) -> Option<Response<HttpBoxBody>> {
    let entry = ctx.cache.lookup_stale(&ctx.key)?;
    exyonq_cache::note_fpc_stale();
    let head_only = *method == hyper::Method::HEAD;
    let mut resp = served_entry_to_response(entry.as_ref(), head_only);
    apply_x_cache(resp.headers_mut(), "stale");
    Some(resp)
}

fn served_entry_to_response(entry: &CachedEntry, head_only: bool) -> Response<HttpBoxBody> {
    let served = entry_to_served(entry, head_only);
    let status = StatusCode::from_u16(served.status).unwrap_or(StatusCode::OK);
    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in &served.headers {
            let lower = name.to_ascii_lowercase();
            if lower == "set-cookie"
                || lower == "connection"
                || lower == "keep-alive"
                || lower == "transfer-encoding"
                || lower == "upgrade"
                || lower == "te"
                || lower == "trailer"
                || lower == "proxy-authenticate"
                || lower == "proxy-authorization"
                || lower == X_CACHE_HEADER
                || lower == X_CACHE_REASON_HEADER
            {
                continue;
            }
            if let (Ok(n), Ok(v)) = (
                header::HeaderName::from_bytes(name.as_bytes()),
                header::HeaderValue::from_str(value),
            ) {
                headers.insert(n, v);
            }
        }
        apply_x_cache(headers, "hit");
    }
    let body = if head_only { Bytes::new() } else { served.body };
    builder
        .body(Full::new(body).map_err(|never| match never {}).boxed())
        .unwrap_or_else(|_| {
            // Never turn an L1 HIT into 5xx (WC2C fail-open serve).
            Response::builder()
                .status(status)
                .body(
                    Full::new(if head_only {
                        Bytes::new()
                    } else {
                        entry.body.clone()
                    })
                    .map_err(|never| match never {})
                    .boxed(),
                )
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(
                            Full::new(Bytes::new())
                                .map_err(|never| match never {})
                                .boxed(),
                        )
                        .expect("minimal hit fallback")
                })
        })
}
