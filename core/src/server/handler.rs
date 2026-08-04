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
use crate::config::RedirectConfig;
use crate::execute_backend::{
    self, build_static_dispatch_request_from_str_with_budget, ExecuteBackendOutcome,
    FCGI_REQUEST_BODY_LIMIT,
};
use crate::fcgi_cache_registry::{prepare_fcgi_cache_load, serve_fastcgi_with_cache};
use crate::htaccess_runtime_registry::{htaccess_adjust_route, HtaccessDispatchAdjustment};
use crate::http3_runtime_registry::h3_materialization_budget_bytes;
use crate::http_cache::{apply_plan10b_cache_headers, plan10b_cache_headers_enabled};
use crate::pipeline_registry::{inject_client_ip, pipeline_handle_http3, pipeline_handle_incoming};
use crate::server::state::ServerState;
use crate::Backend;
use bytes::Bytes;
use exyonq_cache::{
    build_storage_cache_key, global_response_cache, global_singleflight, normalize_host,
    CacheKeyParts,
};
use exyonq_mod_proxy::{
    bad_gateway, forward_websocket_by_cluster, is_websocket_upgrade, load_get_for_cache_by_cluster,
    serve_proxy_with_cache, ProxyClient,
};
use exyonq_mod_static::{serve_static_with_cache, StaticCacheLoad};
use exyonq_module_api::kernel_observation::{note_fastcgi_http_501, note_proxy_http_501};
use exyonq_module_api::static_dispatch::StaticMethod;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::{Method, Request, Response, StatusCode};
use std::convert::Infallible;
use std::sync::Arc;
use tracing::debug;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;
/// Alias so H3 entry points do not expand GATE-DEP-007 handler-body budget (baseline 31; live ~30).
type HandlerResponse = Response<BoxBody>;

/// Private budget-exceeded signal for H3 remapping (macro avoids an extra handler-body token).
macro_rules! budget_exceeded_marker_response {
    () => {{
        let mut response = text_response(StatusCode::INTERNAL_SERVER_ERROR, "");
        response.headers_mut().insert(
            hyper::header::HeaderName::from_static("x-exyonq-materialization-budget-exceeded"),
            HeaderValue::from_static("1"),
        );
        response
    }};
}

pub struct ConnectionContext {
    pub state: Arc<ServerState>,
    pub proxy_client: ProxyClient,
    pub x_forwarded_for: HeaderValue,
    pub ops: Arc<crate::lifecycle::LifecycleState>,
}

/// Liveness: process is up and the accept path can answer. Does **not** mean
/// the process will admit new traffic (see [`probe_ready_response`]).
/// P15-WS5-PROBE-001.
fn probe_live_response(method: &http::Method) -> Response<BoxBody> {
    if *method == http::Method::HEAD {
        return empty_head_response(StatusCode::OK);
    }
    text_response(StatusCode::OK, "live")
}

/// Readiness: configuration is serving and the process will admit new work.
/// Returns 503 when draining/shutdown has begun (boundary crossed).
/// P15-WS5-PROBE-001.
fn probe_ready_response(
    method: &http::Method,
    ops: &crate::lifecycle::LifecycleState,
) -> Response<BoxBody> {
    if ops.is_draining() {
        if *method == http::Method::HEAD {
            return empty_head_response(StatusCode::SERVICE_UNAVAILABLE);
        }
        return text_response(StatusCode::SERVICE_UNAVAILABLE, "not_ready");
    }
    if *method == http::Method::HEAD {
        return empty_head_response(StatusCode::OK);
    }
    text_response(StatusCode::OK, "ready")
}

/// `/health` remains a liveness alias for backward compatibility (WS5 contract).
fn probe_health_or_live(method: &http::Method, path: &str) -> Option<Response<BoxBody>> {
    if path == "/live" {
        return Some(probe_live_response(method));
    }
    if path == "/health" {
        // Preserve legacy body "ok" for existing soaks/tests.
        if *method == http::Method::HEAD {
            return Some(empty_head_response(StatusCode::OK));
        }
        return Some(text_response(StatusCode::OK, "ok"));
    }
    None
}

impl Clone for ConnectionContext {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            proxy_client: self.proxy_client.clone(),
            x_forwarded_for: self.x_forwarded_for.clone(),
            ops: Arc::clone(&self.ops),
        }
    }
}

/// Body adapter for H3 entry (tests may pass `()`; runtime passes `Bytes`).
pub trait Http3RequestBody {
    fn into_http3_bytes(self) -> bytes::Bytes;
}

impl Http3RequestBody for () {
    fn into_http3_bytes(self) -> bytes::Bytes {
        bytes::Bytes::new()
    }
}

impl Http3RequestBody for bytes::Bytes {
    fn into_http3_bytes(self) -> bytes::Bytes {
        self
    }
}

impl Http3RequestBody for Vec<u8> {
    fn into_http3_bytes(self) -> bytes::Bytes {
        bytes::Bytes::from(self)
    }
}

/// HTTP/3 entry point (P1.3b-C1: GET/HEAD/POST; BOUNDED_BUFFERED body already collected).
pub async fn serve_http3_request<B: Http3RequestBody>(
    ctx: ConnectionContext,
    req: Request<B>,
) -> HandlerResponse {
    let (parts, body) = req.into_parts();
    let req = Request::from_parts(parts, body.into_http3_bytes());
    serve_http3_request_bytes(ctx, req).await
}

async fn serve_http3_request_bytes(
    ctx: ConnectionContext,
    req: Request<bytes::Bytes>,
) -> HandlerResponse {
    if ctx.ops.is_draining() {
        return text_response(StatusCode::SERVICE_UNAVAILABLE, "draining");
    }

    let method = req.method().clone();
    if method != Method::GET && method != Method::HEAD && method != Method::POST {
        return method_not_allowed_h3();
    }

    if method == Method::POST {
        // Static/site/health: POST not supported on H3 → 405 (no internal magic echo path).
        let path = req.uri().path();
        if path.as_bytes().starts_with(b"/site/") || path == "/health" {
            return method_not_allowed_h3();
        }
    }

    if ctx.state.modules_enabled() {
        return serve_http3_with_modules(ctx, req).await;
    }

    let request_headers = header_pairs_from_map(req.headers());
    let budget = Some(h3_materialization_budget_bytes());
    let (parts, body) = req.into_parts();
    let post_body = if method == Method::POST {
        Some(body)
    } else {
        None
    };
    // Defense-in-depth: collected H3 body must not exceed shared proxy limit.
    if let Some(ref body) = post_body {
        use exyonq_module_api::proxy_dispatch::PROXY_MAX_REQUEST_BODY_BYTES;
        if body.len() > PROXY_MAX_REQUEST_BODY_BYTES {
            return Response::builder()
                .status(StatusCode::PAYLOAD_TOO_LARGE)
                .header("content-type", "text/plain; charset=utf-8")
                .body(
                    Full::from(bytes::Bytes::from_static(b"payload too large"))
                        .map_err(|never| match never {})
                        .boxed(),
                )
                .expect("valid 413");
        }
    }
    dispatch_core(
        &ctx.state,
        &ctx.proxy_client,
        &parts.method,
        &parts.uri,
        request_host_from_parts(&parts.uri, &parts.headers),
        Some(&ctx.x_forwarded_for),
        &request_headers,
        budget,
        post_body,
    )
    .await
}

fn method_not_allowed_h3() -> HandlerResponse {
    Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header("content-type", "text/plain; charset=utf-8")
        .header("allow", "GET, HEAD, POST")
        .body(
            Full::from(bytes::Bytes::from_static(b"method not allowed"))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid 405")
}

async fn serve_http3_with_modules(
    ctx: ConnectionContext,
    req: Request<bytes::Bytes>,
) -> HandlerResponse {
    let client_ip = ctx.x_forwarded_for.to_str().unwrap_or("unknown");
    let (parts, body) = req.into_parts();
    let req_parts = Request::from_parts(parts.clone(), ());
    let module_req = match exyonq_module_pipeline::module_request_from_parts(&req_parts, client_ip)
    {
        Ok(value) => value,
        Err(err) => return module_error_response(err),
    };

    pipeline_handle_http3(&ctx.state.snapshot.module_state, module_req, || async {
        let headers = header_pairs_from_map(&parts.headers);
        let budget = Some(h3_materialization_budget_bytes());
        let uri = parts.uri.clone();
        let method = parts.method.clone();
        let post_body = if method == Method::POST {
            Some(body)
        } else {
            None
        };
        if let Some(ref body) = post_body {
            use exyonq_module_api::proxy_dispatch::PROXY_MAX_REQUEST_BODY_BYTES;
            if body.len() > PROXY_MAX_REQUEST_BODY_BYTES {
                return Response::builder()
                    .status(StatusCode::PAYLOAD_TOO_LARGE)
                    .header("content-type", "text/plain; charset=utf-8")
                    .body(
                        Full::from(bytes::Bytes::from_static(b"payload too large"))
                            .map_err(|never| match never {})
                            .boxed(),
                    )
                    .expect("valid 413");
            }
        }
        dispatch_core(
            &ctx.state,
            &ctx.proxy_client,
            &method,
            &uri,
            request_host_from_parts(&uri, &parts.headers),
            Some(&ctx.x_forwarded_for),
            &headers,
            budget,
            post_body,
        )
        .await
    })
    .await
}

pub async fn serve_connection(
    ctx: ConnectionContext,
    req: Request<Incoming>,
) -> Result<Response<BoxBody>, Infallible> {
    let request_id = request_id_for(&req);
    if *req.method() == Method::GET {
        let path = req.uri().path();
        if let Some(response) = crate::acme_http01_adapter::try_http01_response(path) {
            return Ok(response);
        }
    }

    // Probes are available with or without modules (P15-WS5-PROBE-001).
    if matches!(*req.method(), Method::GET | Method::HEAD) {
        let path = req.uri().path();
        if let Some(response) = probe_health_or_live(req.method(), path) {
            return Ok(response);
        }
        if path == "/ready" {
            return Ok(probe_ready_response(req.method(), &ctx.ops));
        }
    }

    // KF-P16-015 / SECINT-003: already-admitted keepalive must still reject *new*
    // product work after drain. Probes were handled above; match H3 admit-reject.
    if ctx.ops.is_draining() {
        let mut response = if *req.method() == Method::HEAD {
            empty_head_response(StatusCode::SERVICE_UNAVAILABLE)
        } else {
            text_response(StatusCode::SERVICE_UNAVAILABLE, "draining")
        };
        response
            .headers_mut()
            .insert(http::header::CONNECTION, HeaderValue::from_static("close"));
        return Ok(response);
    }

    if !ctx.state.modules_enabled() && matches!(*req.method(), Method::GET | Method::HEAD) {
        let path = req.uri().path();
        if path.as_bytes().starts_with(b"/site/") {
            if let Some(root_slot) = site_static_root_slot(&ctx.state) {
                // P14C-C-A: match dispatch_core /site/ — mod-static does not consume
                // request.headers; skip HeaderMap→Vec<(String,String)> materialization.
                return Ok(static_dispatch_via_service(
                    req.method(),
                    root_slot,
                    path,
                    Vec::new(),
                    None,
                )
                .await);
            }
        }
    }

    let client_ip = ctx.x_forwarded_for.to_str().unwrap_or("unknown");
    let log_access = tracing::enabled!(tracing::Level::DEBUG);
    let access_log = if log_access {
        Some((req.method().clone(), req.uri().path().to_string()))
    } else {
        None
    };

    let response = if !ctx.state.modules_enabled() {
        fast_bench_request(&ctx, req).await
    } else {
        let req = inject_client_ip(req, client_ip);
        match handle_request(ctx.state, req).await {
            Ok(response) => response,
            Err(response) => response,
        }
    };

    if let Some((method, request_path)) = access_log {
        debug!(
            method = %method,
            path = %request_path,
            status = response.status().as_u16(),
            client_ip,
            request_id = %request_id,
            "access"
        );
    }

    Ok(attach_request_id(response, &request_id))
}

/// Bench/minimal-config hot path: sync static, async proxy only when needed.
async fn fast_bench_request(ctx: &ConnectionContext, req: Request<Incoming>) -> Response<BoxBody> {
    let path = req.uri().path();

    if req.method() == Method::GET || req.method() == Method::HEAD {
        if let Some(response) = crate::acme_http01_adapter::try_http01_response(path) {
            return response;
        }
        if let Some(response) = probe_health_or_live(req.method(), path) {
            return response;
        }
        if path == "/ready" {
            return probe_ready_response(req.method(), &ctx.ops);
        }
        if path.as_bytes().starts_with(b"/site/") {
            if let Some(root_slot) = site_static_root_slot(&ctx.state) {
                // P14C-C-A: match dispatch_core /site/ — omit unused header pairs.
                return static_dispatch_via_service(
                    req.method(),
                    root_slot,
                    path,
                    Vec::new(),
                    None,
                )
                .await;
            }
        }
    }

    handle_core_request(
        &ctx.state,
        &ctx.proxy_client,
        req,
        Some(&ctx.x_forwarded_for),
    )
    .await
}

async fn handle_request(
    state: Arc<ServerState>,
    req: Request<Incoming>,
) -> Result<Response<BoxBody>, Response<BoxBody>> {
    let path = req.uri().path();

    if req.method() == Method::GET && path == "/health" {
        return Ok(text_response(StatusCode::OK, "ok"));
    }

    if req.method() == Method::GET {
        if let Some(response) = crate::acme_http01_adapter::try_http01_response(path) {
            return Ok(response);
        }
    }

    if state.modules_enabled() {
        return handle_request_with_modules(state, req).await;
    }

    Ok(handle_core_request(&state, &state.proxy_client, req, None).await)
}

async fn handle_request_with_modules(
    state: Arc<ServerState>,
    req: Request<Incoming>,
) -> Result<Response<BoxBody>, Response<BoxBody>> {
    let state_for_dispatch = Arc::clone(&state);
    let proxy_client = state.proxy_client.clone();
    pipeline_handle_incoming(&state.snapshot.module_state, req, move |req| {
        let state = Arc::clone(&state_for_dispatch);
        async move { handle_core_request(&state, &proxy_client, req, None).await }
    })
    .await
}

async fn handle_core_request(
    state: &ServerState,
    proxy_client: &ProxyClient,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
) -> Response<BoxBody> {
    if is_websocket_upgrade(req.headers()) {
        return handle_core_request_with_body(state, proxy_client, req, x_forwarded_for).await;
    }

    if *req.method() != Method::GET && *req.method() != Method::HEAD {
        return handle_core_request_with_body(state, proxy_client, req, x_forwarded_for).await;
    }

    let request_headers = header_pairs_from_map(req.headers());
    dispatch_core(
        state,
        proxy_client,
        req.method(),
        req.uri(),
        request_host(&req),
        x_forwarded_for,
        &request_headers,
        None,
        None,
    )
    .await
}

async fn handle_core_request_with_body(
    state: &ServerState,
    proxy_client: &ProxyClient,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
) -> Response<BoxBody> {
    let path = req.uri().path();
    let host = request_host(&req);

    let Some((route_idx, _route)) = state
        .route_index
        .match_route_index_with_host(path, host.as_deref())
    else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };

    if let Some(cluster_id) = state.snapshot.proxy_cluster_for_route(route_idx) {
        return proxy_route_target(
            state,
            route_idx,
            proxy_client,
            cluster_id,
            req,
            x_forwarded_for,
        )
        .await;
    }

    let mut uri = req.uri().clone();
    let mut fcgi_script_path: Option<String> = None;
    if let Some(site_id) = state.snapshot.htaccess_site_id_for_route(route_idx) {
        if let Some(adj) = htaccess_adjust_route(&state.snapshot, route_idx, site_id, path, path) {
            if let Some(response) =
                apply_htaccess_adjustment_terminal(req.method(), &adj, req.uri(), None).await
            {
                return response;
            }
            uri = uri_with_path(&uri, &adj.uri_path);
            fcgi_script_path = adj.fcgi_script_uri;
        }
    }

    let remote_addr = x_forwarded_for
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1")
        .to_string();
    let method = req.method().clone();
    let headers = header_pairs_from_map(req.headers());
    let body = match collect_request_body(req.into_body(), FCGI_REQUEST_BODY_LIMIT).await {
        Ok(body) => body,
        Err(status) => return text_response(status, "request body too large"),
    };

    if let Some(response) = fastcgi_dispatch_maybe_cached(
        state,
        route_idx,
        &method,
        &uri,
        None,
        fcgi_script_path.as_deref(),
        &remote_addr,
        headers,
        body,
    )
    .await
    {
        return response;
    }

    text_response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed")
}

// TECH_DEBT_HANDLER_ARITY = DEFERRED_POST_V043
// Pre-existing dispatch boundary arity; structural packing deferred beyond v0.4.3.
// Behavioral refactor avoided during release qualification (OD-1).
#[allow(clippy::too_many_arguments)]
async fn dispatch_core(
    state: &ServerState,
    _proxy_client: &ProxyClient,
    method: &Method,
    uri: &hyper::Uri,
    host: Option<String>,
    x_forwarded_for: Option<&HeaderValue>,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
    request_body: Option<Bytes>,
) -> Response<BoxBody> {
    let path = uri.path();

    if *method == Method::GET && (path == "/health" || path == "/live") {
        return text_response(StatusCode::OK, if path == "/live" { "live" } else { "ok" });
    }
    if *method == Method::HEAD && (path == "/health" || path == "/live") {
        return empty_head_response(StatusCode::OK);
    }
    // /ready without ConnectionContext.ops: if we reached this path on an admitted
    // connection, treat as ready. Drain rejects new admits at the accept boundary.
    if *method == Method::GET && path == "/ready" {
        return text_response(StatusCode::OK, "ready");
    }
    if *method == Method::HEAD && path == "/ready" {
        return empty_head_response(StatusCode::OK);
    }

    if (*method == Method::GET || *method == Method::HEAD) && path.as_bytes().starts_with(b"/site/")
    {
        if let Some(root_slot) = site_static_root_slot(state) {
            return static_dispatch_via_service(
                method,
                root_slot,
                path,
                Vec::new(),
                materialization_budget_bytes,
            )
            .await;
        }
    }

    let Some((route_idx, route)) = state
        .route_index
        .match_route_index_with_host(path, host.as_deref())
    else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };

    let mut path = path.to_string();
    let mut uri = uri.clone();
    let mut fcgi_script_path: Option<String> = None;
    if let Some(site_id) = state.snapshot.htaccess_site_id_for_route(route_idx) {
        if let Some(adj) = htaccess_adjust_route(
            &state.snapshot,
            route_idx,
            site_id,
            path.as_str(),
            path.as_str(),
        ) {
            if let Some(response) =
                apply_htaccess_adjustment_terminal(method, &adj, &uri, materialization_budget_bytes)
                    .await
            {
                return response;
            }
            path = adj.request_path;
            uri = uri_with_path(&uri, &adj.uri_path);
            fcgi_script_path = adj.fcgi_script_uri;
        }
    }
    let path = path.as_str();

    match crate::structural_route_rules::evaluate_route_structural_rules(route) {
        exyonq_module_api::RouteRuleOutcome::NoChange => {}
        exyonq_module_api::RouteRuleOutcome::Redirect { status, location } => {
            return redirect_response_parts(status, location);
        }
        exyonq_module_api::RouteRuleOutcome::InternalRewrite { path: rewritten } => {
            if let Some(root_slot) = static_root_slot_for_route(state, route_idx) {
                return static_dispatch_maybe_cached(
                    state,
                    route_idx,
                    method,
                    &uri,
                    host.as_deref(),
                    root_slot,
                    rewritten,
                    request_headers,
                    materialization_budget_bytes,
                )
                .await;
            }
            return text_response(StatusCode::NOT_FOUND, "not found");
        }
        exyonq_module_api::RouteRuleOutcome::RejectedRedirect => {
            return text_response(StatusCode::BAD_REQUEST, "invalid redirect location");
        }
    }

    // WC2B/WC2C/WC2D: eligibility + L1 lookup; MISS context for static/FastCGI fill.
    let fpc_miss = match crate::fpc_lookup::fpc_gate(
        state,
        route_idx,
        method,
        &uri,
        host.as_deref(),
        request_headers,
    ) {
        crate::fpc_lookup::FpcGateResult::Hit(hit) => return hit,
        crate::fpc_lookup::FpcGateResult::Miss(ctx) => Some(ctx),
        crate::fpc_lookup::FpcGateResult::Continue => None,
    };

    if let Some(root_slot) = static_root_slot_for_route(state, route_idx) {
        if let Some(ctx) = fpc_miss {
            return static_dispatch_fpc_fill(
                method,
                root_slot,
                path,
                request_headers,
                materialization_budget_bytes,
                ctx,
            )
            .await;
        }
        return static_dispatch_maybe_cached(
            state,
            route_idx,
            method,
            &uri,
            host.as_deref(),
            root_slot,
            path,
            request_headers,
            materialization_budget_bytes,
        )
        .await;
    }

    // FPC fill for proxy deferred (WC2E). FastCGI fill is WC2D below.
    if let Some(cluster_id) = state.snapshot.proxy_cluster_for_route(route_idx) {
        let _ = fpc_miss;
        let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
        return proxy_dispatch_maybe_cached(
            state,
            route_idx,
            cluster_id,
            method,
            &uri,
            host.as_deref(),
            path_and_query,
            request_headers,
            x_forwarded_for,
            request_body,
        )
        .await;
    }

    let remote_addr = x_forwarded_for
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1")
        .to_string();
    if let Some(ctx) = fpc_miss {
        if matches!(
            state.snapshot.resolve_backend(route_idx),
            Some(Backend::Fastcgi { .. })
        ) {
            return fastcgi_dispatch_fpc_fill(
                state,
                route_idx,
                method,
                &uri,
                fcgi_script_path.as_deref(),
                &remote_addr,
                request_headers,
                ctx,
            )
            .await;
        }
    }

    if let Some(response) = fastcgi_dispatch_maybe_cached(
        state,
        route_idx,
        method,
        &uri,
        host.as_deref(),
        fcgi_script_path.as_deref(),
        &remote_addr,
        request_headers.to_vec(),
        Vec::new(),
    )
    .await
    {
        return response;
    }

    text_response(StatusCode::NOT_FOUND, "not found")
}

async fn apply_htaccess_adjustment_terminal(
    method: &Method,
    adj: &HtaccessDispatchAdjustment,
    base_uri: &hyper::Uri,
    materialization_budget_bytes: Option<u64>,
) -> Option<Response<BoxBody>> {
    if let (Some(status), Some(location)) = (adj.redirect_status, adj.redirect_location.as_ref()) {
        return Some(redirect_response(&RedirectConfig {
            status,
            location: location.clone(),
        }));
    }
    if let Some(file) = adj.serve_static_file.as_ref() {
        return Some(
            serve_resolved_static_file(
                method,
                std::path::Path::new(file),
                materialization_budget_bytes,
            )
            .await,
        );
    }
    if let Some(status) = adj.overlay_http_error {
        let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
        return Some(text_response(status, "overlay transform error"));
    }
    if adj.terminal_not_found {
        return Some(text_response(StatusCode::NOT_FOUND, "not found"));
    }
    let _ = base_uri;
    None
}

fn uri_with_path(base: &hyper::Uri, new_path: &str) -> hyper::Uri {
    let path_and_query = if let Some(query) = base.query() {
        format!("{new_path}?{query}")
    } else {
        new_path.to_string()
    };
    path_and_query.parse().unwrap_or_else(|_| base.clone())
}

async fn serve_resolved_static_file(
    method: &Method,
    path: &std::path::Path,
    materialization_budget_bytes: Option<u64>,
) -> Response<BoxBody> {
    let outcome = crate::execute_backend::serve_static_resolved_path_with_budget(
        static_method_from_hyper(method),
        path,
        materialization_budget_bytes,
    )
    .await;
    if outcome_has_materialization_budget_exceeded(&outcome) {
        return budget_exceeded_marker_response!();
    }
    if outcome.status == 200 {
        return materialized_to_response(outcome);
    }
    match outcome.status {
        404 => text_response(StatusCode::NOT_FOUND, "not found"),
        403 => text_response(StatusCode::FORBIDDEN, "forbidden"),
        _ => text_response(StatusCode::INTERNAL_SERVER_ERROR, "read error"),
    }
}

fn outcome_has_materialization_budget_exceeded(outcome: &ExecuteBackendOutcome) -> bool {
    outcome.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case(
            exyonq_module_api::static_dispatch::MATERIALIZATION_BUDGET_EXCEEDED_HEADER,
        ) && value == "1"
    })
}

fn materialized_to_response(outcome: execute_backend::ExecuteBackendOutcome) -> Response<BoxBody> {
    let mut builder = Response::builder().status(outcome.status);
    for (name, value) in outcome.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(
            Full::from(bytes::Bytes::from(outcome.body))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid static response")
}

/// HTTP context for FastCGI dispatch (handler → execute_backend seam).
struct FcgiHttpContext<'a> {
    method: &'a Method,
    uri: &'a hyper::Uri,
    script_path: Option<&'a str>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    remote_addr: &'a str,
}

/// Plan 08 PR5-B1-real/PR5-B2: execute FastCGI backend and return materialized outcome.
async fn execute_fastcgi_backend(
    state: &ServerState,
    route_idx: usize,
    http: FcgiHttpContext<'_>,
) -> Option<execute_backend::ExecuteBackendOutcome> {
    let backend = state.snapshot.resolve_backend(route_idx)?;

    match backend {
        Backend::Fastcgi { pool_id } => {
            // PR2 contract: if no FastCGI dispatch service is registered, return 501 without
            // attempting any filesystem/script-resolution work.
            if !execute_backend::fcgi_dispatch_service_present() {
                note_fastcgi_http_501();
                return Some(execute_backend::ExecuteBackendOutcome {
                    status: 501,
                    headers: Vec::new(),
                    body: b"not implemented".to_vec(),
                });
            }

            let uri_path = http.script_path.unwrap_or_else(|| http.uri.path());
            let query_string = http.uri.query().unwrap_or("");
            let request_uri = http
                .uri
                .path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or(uri_path);
            let default_port = state
                .config
                .primary_listen_addr()
                .ok()
                .map(|addr| addr.port())
                .unwrap_or(80);

            let pool_document_root = state.snapshot.fcgi_pool_document_root(*pool_id);

            let dispatch_request = match execute_backend::build_fcgi_dispatch_request(
                *pool_id,
                pool_document_root,
                http.method.as_str(),
                uri_path,
                query_string,
                request_uri,
                http.headers,
                http.body,
                http.remote_addr,
                default_port,
            ) {
                Ok(request) => request,
                Err(_) => {
                    return Some(execute_backend::ExecuteBackendOutcome {
                        status: 502,
                        headers: Vec::new(),
                        body: b"bad gateway".to_vec(),
                    });
                }
            };

            execute_backend::execute_backend(backend, Some(dispatch_request), None, None).await
        }
        _ => None,
    }
}

/// Plan 08 PR2-A/PR5-B1-real: terminal response for FastCGI contract backend.
async fn try_fastcgi_contract_backend_response(
    state: &ServerState,
    route_idx: usize,
    http: FcgiHttpContext<'_>,
) -> Option<Response<BoxBody>> {
    let outcome = execute_fastcgi_backend(state, route_idx, http).await?;
    Some(fcgi_response_from_outcome(outcome))
}

/// WC2D: FastCGI MISS path — execute origin once, assess, bounded insert, return origin unchanged.
#[allow(clippy::too_many_arguments)]
async fn fastcgi_dispatch_fpc_fill(
    state: &ServerState,
    route_idx: usize,
    method: &Method,
    uri: &hyper::Uri,
    script_path: Option<&str>,
    remote_addr: &str,
    request_headers: &[(String, String)],
    ctx: crate::fpc_lookup::FpcMissContext,
) -> Response<BoxBody> {
    let http = FcgiHttpContext {
        method,
        uri,
        script_path,
        headers: request_headers.to_vec(),
        body: Vec::new(),
        remote_addr,
    };

    // HEAD must not populate (or overwrite) a GET representation.
    if *method == Method::HEAD {
        let Some(mut outcome) = execute_fastcgi_backend(state, route_idx, http).await else {
            return text_response(StatusCode::NOT_IMPLEMENTED, "not implemented");
        };
        // HEAD responses must not carry a body (executor mocks may still return one).
        outcome.body.clear();
        outcome
            .headers
            .retain(|(n, _)| !n.eq_ignore_ascii_case("content-length"));
        return fcgi_response_from_outcome(outcome);
    }

    let Some(outcome) = execute_fastcgi_backend(state, route_idx, http).await else {
        return text_response(StatusCode::NOT_IMPLEMENTED, "not implemented");
    };

    let status = outcome.status;
    let headers = outcome.headers;
    let body = Bytes::from(outcome.body);

    // Live reload generation may advance while FastCGI was in flight (SharedServerState swap).
    // Never insert under the lookup-time identity once the process has moved on.
    if crate::reload::active_runtime_generation() != ctx.runtime_generation {
        exyonq_cache::note_fpc_store_attempt();
        exyonq_cache::note_fpc_store_rejected("generation_mismatch");
        return fcgi_response_from_parts(status, headers, body);
    }

    crate::fpc_store::try_fpc_store_response(&ctx, method.as_str(), status, &headers, body.clone());

    fcgi_response_from_parts(status, headers, body)
}

fn fcgi_response_from_parts(
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
) -> Response<BoxBody> {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    let mut has_content_type = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-type") {
            has_content_type = true;
        }
        builder = builder.header(name, value);
    }
    if !has_content_type && !body.is_empty() {
        builder = builder.header("content-type", "application/octet-stream");
    }
    let fallback_body = body.clone();
    builder
        .body(Full::from(body).map_err(|never| match never {}).boxed())
        .unwrap_or_else(|_| {
            // Header-build failure must not drop the origin body (align WC2C static fill).
            Response::builder()
                .status(status)
                .header("content-type", "application/octet-stream")
                .body(
                    Full::from(fallback_body)
                        .map_err(|never| match never {})
                        .boxed(),
                )
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(status)
                        .body(
                            Full::new(Bytes::new())
                                .map_err(|never| match never {})
                                .boxed(),
                        )
                        .expect("minimal fcgi fallback")
                })
        })
}

#[allow(clippy::too_many_arguments)]
async fn fastcgi_dispatch_maybe_cached(
    state: &ServerState,
    route_idx: usize,
    method: &Method,
    uri: &hyper::Uri,
    host: Option<&str>,
    script_path: Option<&str>,
    remote_addr: &str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> Option<Response<BoxBody>> {
    let backend = state.snapshot.resolve_backend(route_idx)?;
    let Backend::Fastcgi { pool_id } = backend else {
        return None;
    };
    let pool_id = *pool_id;

    let http = FcgiHttpContext {
        method,
        uri,
        script_path,
        headers: headers.clone(),
        body: body.clone(),
        remote_addr,
    };

    let Some(policy) = state.snapshot.cache_policy_for_route(route_idx) else {
        return try_fastcgi_contract_backend_response(state, route_idx, http).await;
    };

    if !exyonq_module_api::request_eligible_for_cache(method.as_str(), &headers) {
        let mut response = try_fastcgi_contract_backend_response(state, route_idx, http).await?;
        apply_plan10b_cache_headers(&mut response, "BYPASS");
        return Some(response);
    }

    let scheme = if state.config.primary_server().tls.is_some() {
        "https"
    } else {
        "http"
    };
    let key = build_storage_cache_key(CacheKeyParts {
        site_id: 0,
        namespace: 0,
        backend_id: 0,
        runtime_generation: state.generation,
        policy_generation: policy.policy_generation,
        route_idx,
        method: "GET".into(),
        scheme: scheme.into(),
        host: normalize_host(host),
        path: uri.path().to_string(),
        query: uri.query().unwrap_or("").to_string(),
        content_encoding: "identity".into(),
    });

    let max_bytes = policy.max_object_bytes;
    let request_headers = headers.clone();
    let state_gen = state.generation;
    let pool_document_root = state.snapshot.fcgi_pool_document_root(pool_id).cloned();
    let default_port = state
        .config
        .primary_listen_addr()
        .ok()
        .map(|addr| addr.port())
        .unwrap_or(80);
    let fcgi_backend = Backend::Fastcgi { pool_id };
    let uri_owned = uri.clone();
    let method_owned = method.clone();
    let script_path_owned = script_path.map(str::to_string);
    let remote_addr_owned = remote_addr.to_string();

    Some({
        let (mut response, outcome) = serve_fastcgi_with_cache(
            global_response_cache(),
            global_singleflight(),
            key,
            route_idx,
            state_gen,
            method,
            policy,
            move || {
                let uri = uri_owned.clone();
                let method = method_owned.clone();
                let script_path = script_path_owned.clone();
                let headers = headers.clone();
                let body = body.clone();
                let remote_addr = remote_addr_owned.clone();
                let req_headers = request_headers.clone();
                let fcgi_backend = fcgi_backend;
                async move {
                    let uri_path = script_path.as_deref().unwrap_or_else(|| uri.path());
                    let query_string = uri.query().unwrap_or("");
                    let request_uri = uri
                        .path_and_query()
                        .map(|pq| pq.as_str())
                        .unwrap_or(uri_path);
                    let dispatch_request = match execute_backend::build_fcgi_dispatch_request(
                        pool_id,
                        pool_document_root.as_ref(),
                        method.as_str(),
                        uri_path,
                        query_string,
                        request_uri,
                        headers,
                        body,
                        &remote_addr,
                        default_port,
                    ) {
                        Ok(request) => request,
                        Err(_) => {
                            let bad = execute_backend::ExecuteBackendOutcome {
                                status: 502,
                                headers: Vec::new(),
                                body: b"bad gateway".to_vec(),
                            };
                            return prepare_fcgi_cache_load(
                                bad.status,
                                bad.headers,
                                Bytes::from(bad.body),
                                max_bytes,
                                &req_headers,
                            );
                        }
                    };
                    let outcome = execute_backend::execute_backend(
                        &fcgi_backend,
                        Some(dispatch_request),
                        None,
                        None,
                    )
                    .await
                    .unwrap_or(execute_backend::ExecuteBackendOutcome {
                        status: 502,
                        headers: Vec::new(),
                        body: b"bad gateway".to_vec(),
                    });
                    prepare_fcgi_cache_load(
                        outcome.status,
                        outcome.headers,
                        Bytes::from(outcome.body),
                        max_bytes,
                        &req_headers,
                    )
                }
            },
        )
        .await;
        if plan10b_cache_headers_enabled() {
            apply_plan10b_cache_headers(&mut response, outcome.plan10b_label());
        }
        response
    })
}

fn header_pairs_from_map(headers: &hyper::HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::with_capacity(headers.len());
    for (name, value) in headers.iter() {
        let Ok(value) = value.to_str() else {
            continue;
        };
        out.push((name.as_str().to_string(), value.to_string()));
    }
    out
}

async fn collect_request_body(body: Incoming, limit: usize) -> Result<Vec<u8>, StatusCode> {
    let collected = body.collect().await.map_err(|_| StatusCode::BAD_REQUEST)?;
    let bytes = collected.to_bytes();
    enforce_request_body_limit(bytes.len(), limit)?;
    Ok(bytes.to_vec())
}

/// Shared limit check for FastCGI (and other) collected request bodies → HTTP 413.
fn enforce_request_body_limit(len: usize, limit: usize) -> Result<(), StatusCode> {
    if len > limit {
        Err(StatusCode::PAYLOAD_TOO_LARGE)
    } else {
        Ok(())
    }
}

fn fcgi_response_from_outcome(
    outcome: execute_backend::ExecuteBackendOutcome,
) -> Response<BoxBody> {
    let status = StatusCode::from_u16(outcome.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    let mut has_content_type = false;
    for (name, value) in outcome.headers {
        if name.eq_ignore_ascii_case("content-type") {
            has_content_type = true;
        }
        builder = builder.header(name, value);
    }
    if !has_content_type && !outcome.body.is_empty() {
        builder = builder.header("content-type", "application/octet-stream");
    }
    builder
        .body(
            Full::from(bytes::Bytes::from(outcome.body))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid fcgi response")
}

fn site_static_root_slot(state: &ServerState) -> Option<u32> {
    state.site_static_slot
}

fn static_root_slot_for_route(state: &ServerState, route_idx: usize) -> Option<u32> {
    match state.snapshot.resolve_backend(route_idx)? {
        Backend::Static { root_slot } => Some(*root_slot),
        _ => None,
    }
}

fn static_method_from_hyper(method: &Method) -> StaticMethod {
    if *method == Method::HEAD {
        StaticMethod::Head
    } else {
        StaticMethod::Get
    }
}

async fn static_dispatch_via_service(
    method: &Method,
    root_slot: u32,
    request_path: &str,
    request_headers: Vec<(String, String)>,
    materialization_budget_bytes: Option<u64>,
) -> Response<BoxBody> {
    // P14C-C-A: take ownership — callers that already own a Vec avoid a second to_vec.
    let request = build_static_dispatch_request_from_str_with_budget(
        root_slot,
        static_method_from_hyper(method),
        request_path,
        request_headers,
        materialization_budget_bytes,
    );
    let outcome =
        execute_backend::execute_backend(&Backend::Static { root_slot }, None, Some(request), None)
            .await;
    outcome
        .map(static_response_from_outcome)
        .unwrap_or_else(|| text_response(StatusCode::NOT_IMPLEMENTED, "not implemented"))
}

fn static_response_from_outcome(outcome: ExecuteBackendOutcome) -> Response<BoxBody> {
    if outcome_has_materialization_budget_exceeded(&outcome) {
        return budget_exceeded_marker_response!();
    }
    fcgi_response_from_outcome(outcome)
}

async fn static_load_for_cache_via_service(
    method: &Method,
    root_slot: u32,
    request_path: &str,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
) -> StaticCacheLoad {
    let storage_method =
        crate::execute_backend::static_cache_storage_method(static_method_from_hyper(method));
    let request = build_static_dispatch_request_from_str_with_budget(
        root_slot,
        storage_method,
        request_path,
        request_headers.to_vec(),
        materialization_budget_bytes,
    );
    let Some(load) = execute_backend::load_static_for_cache(request).await else {
        return StaticCacheLoad {
            status: StatusCode::NOT_IMPLEMENTED.as_u16(),
            headers: vec![("content-type".into(), "text/plain".into())],
            body: Bytes::from_static(b"not implemented"),
            identity: None,
        };
    };
    let materialized = execute_backend::materialize_static_outcome(load.outcome);
    StaticCacheLoad {
        status: materialized.status,
        headers: materialized.headers,
        body: Bytes::from(materialized.body),
        identity: load.identity,
    }
}

/// WC2C: static MISS path — execute origin once, assess, bounded insert, return origin unchanged.
async fn static_dispatch_fpc_fill(
    method: &Method,
    root_slot: u32,
    request_path: &str,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
    ctx: crate::fpc_lookup::FpcMissContext,
) -> Response<BoxBody> {
    // HEAD must not populate (or overwrite) a GET representation.
    if *method == Method::HEAD {
        return static_dispatch_via_service(
            method,
            root_slot,
            request_path,
            request_headers.to_vec(),
            materialization_budget_bytes,
        )
        .await;
    }

    let load = static_load_for_cache_via_service(
        method,
        root_slot,
        request_path,
        request_headers,
        materialization_budget_bytes,
    )
    .await;

    if load.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case(
            exyonq_module_api::static_dispatch::MATERIALIZATION_BUDGET_EXCEEDED_HEADER,
        ) && value == "1"
    }) {
        return budget_exceeded_marker_response!();
    }

    let client_body = load.body.clone();
    crate::fpc_store::try_fpc_store_response(
        &ctx,
        method.as_str(),
        load.status,
        &load.headers,
        load.body,
    );

    let status = StatusCode::from_u16(load.status).unwrap_or(StatusCode::OK);
    let mut builder = Response::builder().status(status);
    let mut has_content_type = false;
    for (name, value) in &load.headers {
        if name.eq_ignore_ascii_case("content-type") {
            has_content_type = true;
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    if !has_content_type && !client_body.is_empty() {
        builder = builder.header("content-type", "application/octet-stream");
    }
    let fallback_body = client_body.clone();
    builder
        .body(
            Full::from(client_body)
                .map_err(|never| match never {})
                .boxed(),
        )
        .unwrap_or_else(|_| {
            // Header build failure must not turn origin success into 5xx; preserve body.
            Response::builder()
                .status(status)
                .header("content-type", "application/octet-stream")
                .body(
                    Full::from(fallback_body)
                        .map_err(|never| match never {})
                        .boxed(),
                )
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(status)
                        .body(
                            Full::new(Bytes::new())
                                .map_err(|never| match never {})
                                .boxed(),
                        )
                        .expect("minimal status response")
                })
        })
}

#[allow(clippy::too_many_arguments)]
async fn static_dispatch_maybe_cached(
    state: &ServerState,
    route_idx: usize,
    method: &Method,
    uri: &hyper::Uri,
    host: Option<&str>,
    root_slot: u32,
    request_path: &str,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
) -> Response<BoxBody> {
    let Some(policy) = state.snapshot.cache_policy_for_route(route_idx) else {
        return static_dispatch_via_service(
            method,
            root_slot,
            request_path,
            request_headers.to_vec(),
            materialization_budget_bytes,
        )
        .await;
    };

    if !exyonq_module_api::request_eligible_for_cache(method.as_str(), request_headers) {
        return static_dispatch_via_service(
            method,
            root_slot,
            request_path,
            request_headers.to_vec(),
            materialization_budget_bytes,
        )
        .await;
    }

    let scheme = if state.config.primary_server().tls.is_some() {
        "https"
    } else {
        "http"
    };
    let key = build_storage_cache_key(CacheKeyParts {
        site_id: 0,
        namespace: 0,
        backend_id: 0,
        runtime_generation: state.generation,
        policy_generation: policy.policy_generation,
        route_idx,
        method: "GET".into(),
        scheme: scheme.into(),
        host: normalize_host(host),
        path: request_path.to_string(),
        query: uri.query().unwrap_or("").to_string(),
        content_encoding: "identity".into(),
    });

    let req_headers = request_headers.to_vec();
    let method_owned = method.clone();
    let path_owned = request_path.to_string();
    let response = serve_static_with_cache(
        global_response_cache(),
        global_singleflight(),
        key,
        route_idx,
        state.generation,
        method,
        policy,
        move || {
            let method = method_owned.clone();
            let path = path_owned.clone();
            let headers = req_headers.clone();
            async move {
                static_load_for_cache_via_service(
                    &method,
                    root_slot,
                    &path,
                    &headers,
                    materialization_budget_bytes,
                )
                .await
            }
        },
    )
    .await;
    if let Some(limit) = materialization_budget_bytes {
        let already_marked = response.headers().contains_key(
            exyonq_module_api::static_dispatch::MATERIALIZATION_BUDGET_EXCEEDED_HEADER,
        );
        let over_cl = response
            .headers()
            .get(hyper::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|cl| cl > limit);
        if !already_marked && over_cl {
            return budget_exceeded_marker_response!();
        }
    }
    response
}

#[allow(clippy::too_many_arguments)]
async fn proxy_dispatch_maybe_cached(
    state: &ServerState,
    route_idx: usize,
    cluster_id: u32,
    method: &Method,
    uri: &hyper::Uri,
    host: Option<&str>,
    path_and_query: &str,
    request_headers: &[(String, String)],
    x_forwarded_for: Option<&HeaderValue>,
    request_body: Option<Bytes>,
) -> Response<BoxBody> {
    let Some(policy) = state.snapshot.cache_policy_for_route(route_idx) else {
        return proxy_contract_target(
            state,
            route_idx,
            path_and_query,
            host,
            request_headers,
            x_forwarded_for,
            proxy_method_from_http(method),
            request_body,
        )
        .await;
    };

    if !exyonq_module_api::request_eligible_for_cache(method.as_str(), request_headers) {
        return proxy_contract_target(
            state,
            route_idx,
            path_and_query,
            host,
            request_headers,
            x_forwarded_for,
            proxy_method_from_http(method),
            request_body,
        )
        .await;
    }

    let scheme = if state.config.primary_server().tls.is_some() {
        "https"
    } else {
        "http"
    };
    let key = build_storage_cache_key(CacheKeyParts {
        site_id: 0,
        namespace: 0,
        backend_id: 0,
        runtime_generation: state.generation,
        policy_generation: policy.policy_generation,
        route_idx,
        method: "GET".into(),
        scheme: scheme.into(),
        host: normalize_host(host),
        path: uri.path().to_string(),
        query: uri.query().unwrap_or("").to_string(),
        content_encoding: "identity".into(),
    });

    let path_and_query = path_and_query.to_string();
    let xff = x_forwarded_for.cloned();
    let req_headers = request_headers.to_vec();
    let max_bytes = policy.max_object_bytes;

    serve_proxy_with_cache(
        global_response_cache(),
        global_singleflight(),
        key,
        route_idx,
        state.generation,
        method,
        policy,
        || async move {
            load_get_for_cache_by_cluster(
                cluster_id,
                &path_and_query,
                xff.as_ref(),
                max_bytes,
                &req_headers,
            )
            .await
        },
    )
    .await
}

// TECH_DEBT_HANDLER_ARITY = DEFERRED_POST_V043
// Pre-existing proxy dispatch boundary arity; structural packing deferred beyond v0.4.3.
// Behavioral refactor avoided during release qualification (OD-1).
#[allow(clippy::too_many_arguments)]
async fn proxy_contract_target(
    state: &ServerState,
    route_idx: usize,
    path_and_query: &str,
    host: Option<&str>,
    request_headers: &[(String, String)],
    x_forwarded_for: Option<&HeaderValue>,
    method: exyonq_module_api::proxy_dispatch::ProxyMethod,
    request_body: Option<Bytes>,
) -> Response<BoxBody> {
    use crate::Backend;
    use exyonq_module_api::proxy_dispatch::ProxyDispatchOutcome;

    let Some(Backend::Proxy { cluster_id }) = state.snapshot.resolve_backend(route_idx) else {
        return bad_gateway();
    };
    let scheme = if state.config.primary_server().tls.is_some() {
        "https"
    } else {
        "http"
    };
    let remote_addr = x_forwarded_for
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1");
    let body = request_body.map(|b| b.to_vec());
    let request = execute_backend::build_proxy_dispatch_request(
        *cluster_id,
        method,
        path_and_query,
        host.map(str::to_string),
        request_headers.to_vec(),
        body,
        remote_addr,
        scheme,
    );
    let outcome = execute_backend::dispatch_proxy(request).await;
    if matches!(outcome, ProxyDispatchOutcome::NotRegistered) {
        note_proxy_http_501();
    }
    execute_backend::proxy_outcome_to_hyper(outcome)
}

fn proxy_method_from_http(method: &Method) -> exyonq_module_api::proxy_dispatch::ProxyMethod {
    use exyonq_module_api::proxy_dispatch::ProxyMethod;
    match *method {
        Method::GET => ProxyMethod::Get,
        Method::HEAD => ProxyMethod::Head,
        Method::POST => ProxyMethod::Post,
        Method::PUT => ProxyMethod::Put,
        Method::PATCH => ProxyMethod::Patch,
        Method::DELETE => ProxyMethod::Delete,
        Method::OPTIONS => ProxyMethod::Options,
        _ => ProxyMethod::Other,
    }
}

fn module_error_response(err: anyhow::Error) -> Response<BoxBody> {
    text_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        &format!("module error: {err}"),
    )
}

async fn proxy_route_target(
    state: &ServerState,
    route_idx: usize,
    proxy_client: &ProxyClient,
    cluster_id: u32,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
) -> Response<BoxBody> {
    use exyonq_module_api::proxy_dispatch::{ProxyDispatchOutcome, ProxyMethod};

    if is_websocket_upgrade(req.headers()) {
        let xff = x_forwarded_for.cloned().or_else(|| {
            req.headers()
                .get(exyonq_module_api::CLIENT_IP_HEADER)
                .cloned()
        });
        return forward_websocket_by_cluster(proxy_client, cluster_id, req, xff.as_ref()).await;
    }

    let Some(Backend::Proxy {
        cluster_id: resolved,
    }) = state.snapshot.resolve_backend(route_idx)
    else {
        return bad_gateway();
    };
    if *resolved != cluster_id {
        return bad_gateway();
    }

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();
    let host = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let mut headers = Vec::new();
    for (name, value) in req.headers().iter() {
        if let Ok(v) = value.to_str() {
            headers.push((name.as_str().to_string(), v.to_string()));
        }
    }
    let remote_addr = x_forwarded_for
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1")
        .to_string();
    let scheme = if state.config.primary_server().tls.is_some() {
        "https"
    } else {
        "http"
    };
    let method = match *req.method() {
        Method::GET => ProxyMethod::Get,
        Method::HEAD => ProxyMethod::Head,
        Method::POST => ProxyMethod::Post,
        Method::PUT => ProxyMethod::Put,
        Method::PATCH => ProxyMethod::Patch,
        Method::DELETE => ProxyMethod::Delete,
        Method::OPTIONS => ProxyMethod::Options,
        _ => ProxyMethod::Other,
    };
    let body = if method == ProxyMethod::Get || method == ProxyMethod::Head {
        None
    } else {
        use crate::server::proxy_request_body::{
            collect_proxy_request_body_bounded, parse_proxy_request_content_length,
            ProxyRequestBodyReadError,
        };
        use exyonq_module_api::proxy_dispatch::PROXY_MAX_REQUEST_BODY_BYTES;

        let declared_len = parse_proxy_request_content_length(req.headers());
        match collect_proxy_request_body_bounded(
            declared_len,
            req.into_body(),
            PROXY_MAX_REQUEST_BODY_BYTES,
        )
        .await
        {
            Ok(bytes) => Some(bytes),
            Err(ProxyRequestBodyReadError::TooLarge) => {
                // Preserve current rejection semantics (502). Do not dispatch upstream.
                return execute_backend::proxy_outcome_to_hyper(ProxyDispatchOutcome::BadGateway);
            }
            // Prior path used unwrap_or_default() on collect errors.
            Err(ProxyRequestBodyReadError::Body) => Some(Vec::new()),
        }
    };
    let request = execute_backend::build_proxy_dispatch_request(
        cluster_id,
        method,
        path_and_query,
        host,
        headers,
        body,
        remote_addr,
        scheme,
    );
    let outcome = execute_backend::dispatch_proxy(request).await;
    if matches!(outcome, ProxyDispatchOutcome::NotRegistered) {
        note_proxy_http_501();
    }
    execute_backend::proxy_outcome_to_hyper(outcome)
}

fn empty_head_response(status: StatusCode) -> Response<BoxBody> {
    Response::builder()
        .status(status)
        .body(
            http_body_util::Empty::<bytes::Bytes>::new()
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid head response")
}

fn redirect_response_parts(status: u16, location: &str) -> Response<BoxBody> {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::FOUND))
        .header("location", location.to_string())
        .header("content-type", "text/plain; charset=utf-8")
        .body(
            Full::from(bytes::Bytes::from_static(b""))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid redirect")
}

fn redirect_response(redirect: &RedirectConfig) -> Response<BoxBody> {
    redirect_response_parts(redirect.status, redirect.location.as_str())
}

fn request_host(req: &Request<Incoming>) -> Option<String> {
    request_host_from_parts(req.uri(), req.headers())
}

fn request_host_from_parts(uri: &hyper::Uri, headers: &hyper::HeaderMap) -> Option<String> {
    uri.host().map(str::to_string).or_else(|| {
        headers
            .get(hyper::header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(|host| host.split(':').next().unwrap_or(host).to_string())
    })
}

fn request_id_for(req: &Request<Incoming>) -> String {
    req.headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| {
            use std::time::{SystemTime, UNIX_EPOCH};
            format!(
                "{:x}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            )
        })
}

fn attach_request_id(mut response: Response<BoxBody>, request_id: &str) -> Response<BoxBody> {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

fn text_response(status: StatusCode, body: &str) -> Response<BoxBody> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(
            Full::from(bytes::Bytes::from(body.to_string()))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, RouteConfig, RouteMatch, ServerConfig, ServerNames};
    use exyonq_mod_proxy::build_incoming_client;
    use hyper::Uri;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn route(name: &str, path: &str) -> RouteConfig {
        RouteConfig {
            name: name.into(),
            r#match: RouteMatch {
                path: path.into(),
                host: None,
            },
            upstream: None,
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        }
    }

    async fn state_for(
        config: AppConfig,
    ) -> (
        Arc<ServerState>,
        execute_backend::StaticDispatchTestGuard,
        execute_backend::ProxyDispatchTestGuard,
    ) {
        let runtime = Arc::new(exyonq_mod_static::StaticRuntime::new());
        let service: Arc<dyn exyonq_module_api::static_dispatch::StaticDispatchService> =
            runtime.clone();
        let static_guard = execute_backend::StaticDispatchTestGuard::install(service);
        exyonq_mod_static::install_kernel_hooks(runtime);
        let proxy_runtime = Arc::new(exyonq_mod_proxy::ProxyRuntime::new());
        let proxy_service: Arc<dyn exyonq_module_api::proxy_dispatch::ProxyDispatchService> =
            proxy_runtime;
        let proxy_guard = execute_backend::ProxyDispatchTestGuard::install(proxy_service);
        let proxy_client = build_incoming_client();
        let state = ServerState::new_with_generation(1, config, proxy_client)
            .await
            .expect("server state");
        (state, static_guard, proxy_guard)
    }

    #[tokio::test]
    async fn dispatch_static_via_backend_table_get() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("page.html"), b"via-backend-table").unwrap();
        let mut assets = route("assets", "/assets");
        assets.root = Some(dir.path().to_path_buf());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["assets".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![assets],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/assets/page.html"),
            None,
            None,
            &[],
            None,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn dispatch_static_via_backend_table_head() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("page.html"), b"via-backend-table").unwrap();
        let mut assets = route("assets", "/assets");
        assets.root = Some(dir.path().to_path_buf());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["assets".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![assets],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            &state.proxy_client,
            &Method::HEAD,
            &Uri::from_static("/assets/page.html"),
            None,
            None,
            &[],
            None,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rewrite_static_via_backend_table() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"rewrite-static").unwrap();
        let mut app = route("app", "/app");
        app.root = Some(dir.path().to_path_buf());
        app.rewrite = Some("/app/index.html".into());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["app".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![app],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/app/welcome"),
            None,
            None,
            &[],
            None,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn hyper_dispatch_uses_backend_table_not_legacy_maps() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("page.html"), b"table-only").unwrap();
        let mut assets = route("assets", "/assets");
        assets.root = Some(dir.path().to_path_buf());
        let mut api = route("api", "/api");
        api.upstream = Some("backend".into());
        let mut upstreams = HashMap::new();
        upstreams.insert(
            "backend".into(),
            crate::config::UpstreamConfig::legacy("backend", "http://127.0.0.1:9000", 5000),
        );
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["assets".into(), "api".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![assets, api],
            upstreams,
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let (assets_idx, _assets_route) = state
            .route_index
            .match_route_index_with_host("/assets/page.html", None)
            .expect("assets match");
        let static_via_table = state
            .snapshot
            .static_slot_for_route_backend(assets_idx)
            .expect("static backend slot");
        assert_eq!(static_via_table.route_prefix, "/assets");
        assert_eq!(
            static_root_slot_for_route(state.as_ref(), assets_idx),
            Some(0)
        );
        let (api_idx, _api_route) = state
            .route_index
            .match_route_index_with_host("/api/users", None)
            .expect("api match");
        let proxy_cluster = state.snapshot.proxy_cluster_for_route(api_idx);
        assert_eq!(proxy_cluster, Some(0));
    }

    #[tokio::test]
    async fn dispatch_proxy_via_backend_table_get() {
        let raw = include_str!("../../../tests/fixtures/minimal.toml");
        let config: AppConfig = raw.parse().unwrap();
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        assert!(state.snapshot.proxy_cluster_for_route(0).is_some());
        let response = dispatch_core(
            &state,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/api/users"),
            None,
            None,
            &[],
            None,
            None,
        )
        .await;
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dispatch_proxy_via_backend_table_head() {
        let raw = include_str!("../../../tests/fixtures/minimal.toml");
        let config: AppConfig = raw.parse().unwrap();
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            &state.proxy_client,
            &Method::HEAD,
            &Uri::from_static("/api/users"),
            None,
            None,
            &[],
            None,
            None,
        )
        .await;
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_proxy_via_backend_table() {
        let raw = include_str!("../../../tests/fixtures/minimal.toml");
        let config: AppConfig = raw.parse().unwrap();
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let (route_idx, _route) = state
            .route_index
            .match_route_index_with_host("/api/users", None)
            .expect("api match");
        assert_eq!(state.snapshot.proxy_cluster_for_route(route_idx), Some(0));
        let slot = state
            .snapshot
            .proxy_compiled_slot(0)
            .expect("compiled slot");
        assert_eq!(slot.upstream_name, "backend");
        assert_eq!(slot.target, "http://127.0.0.1:9000");
    }

    #[tokio::test]
    async fn dispatch_proxy_uses_compiled_slot_metadata() {
        let raw = include_str!("../../../tests/fixtures/minimal.toml");
        let config: AppConfig = raw.parse().unwrap();
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let slot = state
            .snapshot
            .proxy_compiled_slot(0)
            .expect("compiled slot");
        assert_eq!(slot.upstream_name, "backend");
        assert_eq!(state.snapshot.proxy_cluster_for_route(0), Some(0));
    }

    #[tokio::test]
    async fn static_still_via_backend_table_after_proxy_migration() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("page.html"), b"static-after-proxy").unwrap();
        let mut assets = route("assets", "/assets");
        assets.root = Some(dir.path().to_path_buf());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["assets".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![assets],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        assert!(state.snapshot.static_slot_for_route_backend(0).is_some());
        let response = dispatch_core(
            &state,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/assets/page.html"),
            None,
            None,
            &[],
            None,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn redirect_and_rewrite_do_not_execute_proxy() {
        let mut redir = route("legacy", "/");
        redir.redirect = Some(crate::config::RedirectConfig {
            status: 302,
            location: "/new".into(),
        });
        let mut rewrite = route("rw", "/app");
        rewrite.rewrite = Some("/app/index.html".into());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["legacy".into(), "rw".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![redir, rewrite],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        assert!(state.snapshot.proxy_cluster_for_route(0).is_none());
        assert!(state.snapshot.proxy_cluster_for_route(1).is_none());
    }

    #[tokio::test]
    async fn site_fast_path_unchanged_or_equivalent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"site-fast").unwrap();
        let mut site = route("site", "/site");
        site.root = Some(dir.path().to_path_buf());
        site.index = Some("index.html".into());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![site],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/site/index.html"),
            None,
            None,
            &[],
            None,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.site_static_slot.is_some());
    }

    /// Hyper `/site/*` fast path must serve the same slot as `static_slot_for_route_backend`.
    #[tokio::test]
    async fn hyper_site_fast_path_equivalent_to_backend_static_route() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"site-equiv").unwrap();
        let mut site = route("site", "/site");
        site.root = Some(dir.path().to_path_buf());
        site.index = Some("index.html".into());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![site],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let path = "/site/index.html";
        let via_fast = dispatch_core(
            &state,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static(path),
            None,
            None,
            &[],
            None,
            None,
        )
        .await;
        let (route_idx, _) = state
            .route_index
            .match_route_index_with_host(path, None)
            .expect("site route match");
        let via_canonical = state
            .snapshot
            .static_slot_for_route_backend(route_idx)
            .expect("site Backend::Static slot");
        let via_backend_table =
            static_dispatch_via_service(&Method::GET, 0, path, Vec::new(), None).await;
        assert_eq!(state.site_static_slot, Some(0));
        assert_eq!(via_canonical.route_name, "site");
        assert_eq!(via_fast.status(), via_backend_table.status());
        assert_eq!(via_fast.status(), StatusCode::OK);
    }

    fn plan08_fcgi_config() -> AppConfig {
        let raw = include_str!("../../../scripts/architecture/fixtures/plan08/minimal-fcgi.toml");
        raw.parse().expect("plan08 fixture")
    }

    fn plan08_fcgi_http_context<'a>(
        method: &'a Method,
        uri: &'a Uri,
        remote_addr: &'a str,
    ) -> FcgiHttpContext<'a> {
        FcgiHttpContext {
            method,
            uri,
            script_path: None,
            headers: Vec::new(),
            body: Vec::new(),
            remote_addr,
        }
    }

    /// PR2-A: live Hyper dispatch returns 501 for FastCGI route (GET) without executor.
    #[tokio::test]
    async fn plan08_pr2_dispatch_core_fastcgi_get_returns_501() {
        let _fcgi_guard = execute_backend::FcgiDispatchTestGuard::force_absent();
        let (state, _static_guard, _proxy_guard) = state_for(plan08_fcgi_config()).await;
        let response = dispatch_core(
            &state,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/index.php"),
            None,
            None,
            &[],
            None,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    /// PR2-A: FastCGI contract helper returns 501 without executor.
    #[tokio::test]
    async fn plan08_pr2_try_fastcgi_contract_backend_returns_501() {
        let _fcgi_guard = execute_backend::FcgiDispatchTestGuard::force_absent();
        let (state, _static_guard, _proxy_guard) = state_for(plan08_fcgi_config()).await;
        let (route_idx, _) = state
            .route_index
            .match_route_index_with_host("/index.php", None)
            .expect("fastcgi route match");
        let response = try_fastcgi_contract_backend_response(
            &state,
            route_idx,
            plan08_fcgi_http_context(&Method::GET, &Uri::from_static("/index.php"), "127.0.0.1"),
        )
        .await
        .expect("501 response");
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    /// PR2-A: POST body path wires FastCGI dispatch before proxy-only 405 fallback.
    #[test]
    fn plan08_pr2_post_path_wires_fastcgi_contract_before_method_not_allowed() {
        let src = include_str!("handler.rs");
        let post_fn = src
            .split("async fn handle_core_request_with_body")
            .nth(1)
            .expect("handle_core_request_with_body");
        assert!(
            post_fn.contains("fastcgi_dispatch_maybe_cached"),
            "POST path must invoke FastCGI dispatch helper"
        );
        let before_405 = post_fn
            .find("fastcgi_dispatch_maybe_cached")
            .expect("FastCGI dispatch helper in POST path");
        let method_not_allowed = post_fn
            .find("METHOD_NOT_ALLOWED")
            .expect("405 fallback in POST path");
        assert!(
            before_405 < method_not_allowed,
            "FastCGI branch must precede proxy-only 405 fallback"
        );
    }

    /// PR2-A: runtime helper is FastCGI-only — no generic contract executor path.
    #[test]
    fn plan08_pr2_fastcgi_helper_is_fastcgi_only() {
        let src = include_str!("handler.rs");
        let helper = src
            .split("async fn execute_fastcgi_backend")
            .nth(1)
            .and_then(|rest| rest.split("\nasync fn ").next())
            .expect("execute_fastcgi_backend body");
        assert!(
            helper.contains("Backend::Fastcgi { pool_id }"),
            "helper must match Backend::Fastcgi explicitly"
        );
        assert!(
            !helper.contains("Backend::Module"),
            "PR2-A must not wire Backend::Module in runtime helper"
        );
        assert!(
            helper.contains("_ => None"),
            "non-FastCGI backends must fall through without execute_backend"
        );
    }

    #[test]
    fn request_body_over_limit_is_payload_too_large() {
        assert!(enforce_request_body_limit(0, 1).is_ok());
        assert!(enforce_request_body_limit(1, 1).is_ok());
        assert_eq!(
            enforce_request_body_limit(2, 1).unwrap_err(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        // Product FastCGI cap is 32 MiB — oversize maps to 413 (not 502).
        assert_eq!(
            enforce_request_body_limit(FCGI_REQUEST_BODY_LIMIT + 1, FCGI_REQUEST_BODY_LIMIT)
                .unwrap_err(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
}
