//! WC2B/WC2C — full-page cache request eligibility + L1 lookup (insert via `fpc_store`).

use crate::server::state::ServerState;
use bytes::Bytes;
use exyonq_cache::{
    build_storage_cache_key, entry_to_served, normalize_host, note_fpc_bypass, note_fpc_hit,
    note_fpc_lookup, note_fpc_lookup_error, note_fpc_miss, CacheKey, CacheKeyParts, CachedEntry,
    ResponseCache,
};
use exyonq_module_api::fpc_request_evaluate;
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
}

/// Result of the FPC gate after BackendId is known.
pub enum FpcGateResult {
    /// Serve from L1; backend must not run.
    Hit(Response<HttpBoxBody>),
    /// Eligible request with no entry — continue backend, then optional insert.
    Miss(FpcMissContext),
    /// Disabled / bypass / error — continue backend without insert.
    Continue,
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
        return FpcGateResult::Continue;
    }
    let Some(cache) = state.fpc_cache.as_ref() else {
        note_fpc_lookup_error();
        return FpcGateResult::Continue;
    };
    if fpc.namespace == 0 {
        note_fpc_bypass();
        return FpcGateResult::Continue;
    }
    let Some(site_id) = fpc
        .route_site_ids
        .get(route_idx)
        .copied()
        .filter(|&id| id != 0)
    else {
        note_fpc_bypass();
        return FpcGateResult::Continue;
    };
    let Some(backend_id) = state.snapshot.route_backend_id(route_idx) else {
        note_fpc_bypass();
        return FpcGateResult::Continue;
    };
    if backend_id == BackendId::INVALID {
        note_fpc_bypass();
        return FpcGateResult::Continue;
    }

    note_fpc_lookup();

    let path = uri.path();
    let query = uri.query().unwrap_or("");
    let (canon_path, canon_query) =
        match fpc_request_evaluate(method.as_str(), path, query, request_headers, &[], &[]) {
            Ok(v) => v,
            Err(_) => {
                note_fpc_bypass();
                return FpcGateResult::Continue;
            }
        };

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
        FpcGateResult::Miss(_) | FpcGateResult::Continue => None,
    }
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
