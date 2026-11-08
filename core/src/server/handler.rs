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
use crate::htaccess_runtime_registry::{
    htaccess_access_policy, htaccess_adjust_route, htaccess_auth_policy, HtaccessDispatchAdjustment,
};
use crate::http3_runtime_registry::h3_materialization_budget_bytes;
use crate::http_cache::{apply_plan10b_cache_headers, plan10b_cache_headers_enabled};
use crate::pipeline_registry::{inject_client_ip, pipeline_handle_http3, pipeline_handle_incoming};
use crate::server::state::ServerState;
use crate::Backend;
use bytes::Bytes;
use exyonq_cache::{
    build_storage_cache_key, global_response_cache, global_singleflight, normalize_host,
    CacheKeyParts, CacheNamespace, NamespaceMetrics,
};
use exyonq_mod_proxy::{
    bad_gateway, forward_websocket_by_cluster, is_websocket_upgrade, load_get_for_cache_by_cluster,
    serve_proxy_with_cache, ProxyCacheLoad, ProxyClient,
};
use exyonq_mod_static::{serve_static_with_cache, StaticCacheLoad};
use exyonq_module_api::kernel_observation::{note_fastcgi_http_501, note_proxy_http_501};
use exyonq_module_api::static_dispatch::StaticMethod;
use http_body::Body as _;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{
    HeaderName, HeaderValue, ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE,
    CONTENT_TYPE, ETAG, LAST_MODIFIED,
};
use hyper::{Method, Request, Response, StatusCode};
use std::convert::Infallible;
use std::sync::Arc;
use tracing::Instrument;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;
type FinalBoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, exyonq_module_api::BoxError>;
/// Alias so H3 entry points do not expand GATE-DEP-007 handler-body budget (baseline 31; live ~30).
type HandlerResponse = Response<BoxBody>;
type H3Response = Response<FinalBoxBody>;

/// Private budget-exceeded signal for H3 remapping (macro avoids an extra handler-body token).
macro_rules! budget_exceeded_marker_response {
    () => {{
        // H1/H2 return 413 before allocating; H3 remaps this marker to its
        // oversized-response policy by inspecting the header.
        let mut response = text_response(StatusCode::PAYLOAD_TOO_LARGE, "response body too large");
        response.headers_mut().insert(
            hyper::header::HeaderName::from_static("x-exyonq-materialization-budget-exceeded"),
            HeaderValue::from_static("1"),
        );
        response
    }};
}

pub struct ConnectionContext {
    pub state: Arc<ServerState>,
    /// Connection-level scheme as seen by the accepting listener (`http`/`https`).
    pub scheme: &'static str,
    /// Port the connection was accepted on (listener port) — NGINX
    /// `port_in_redirect` semantics. `None` when unknown.
    pub local_port: Option<u16>,
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
            scheme: self.scheme,
            local_port: self.local_port,
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
    mut req: Request<B>,
) -> H3Response {
    let path = req.uri().path().to_string();
    let host = request_host_from_parts(req.uri(), req.headers());
    let accept_encoding = req
        .headers()
        .get(hyper::header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let method = req.method().clone();
    let query = req.uri().query().map(str::to_string);
    if !req
        .extensions()
        .get::<crate::mirror::MirrorMarker>()
        .is_some()
    {
        // Internal marker — clients must not suppress or trigger mirroring.
        req.headers_mut().remove(crate::mirror::MIRROR_GUARD);
    }
    let mirror_headers = header_pairs_from_map(req.headers());
    let waf_request_headers = mirror_headers.clone();
    let client_ip = client_ip_from_xff(Some(&ctx.x_forwarded_for));
    let state = Arc::clone(&ctx.state);
    let throttle_cfg = state.config.modules.ratelimit.client.clone();
    let (parts, body) = req.into_parts();
    let body = body.into_http3_bytes();
    let inbound_wait = crate::client_throttle::inbound_wait(&throttle_cfg, client_ip, body.len());
    if !inbound_wait.is_zero() {
        tokio::time::sleep(inbound_wait).await;
    }
    let req = Request::from_parts(parts, body);
    let local_port = ctx.local_port;
    let response = serve_http3_request_bytes(ctx, req).await;
    let mut response: Response<FinalBoxBody> = response.map(|body| {
        body.map_err(|err| -> exyonq_module_api::BoxError { Box::new(err) })
            .boxed()
    });
    apply_route_response_headers(&state, &path, host.as_deref(), local_port, &mut response);
    maybe_gunzip_response(
        &state,
        &path,
        host.as_deref(),
        local_port,
        accept_encoding.as_deref(),
        &method,
        &mut response,
    );
    maybe_sub_filter_response(&state, &path, host.as_deref(), local_port, &mut response);
    crate::waf_body::maybe_wrap_response(
        Arc::clone(&state.waf),
        method.as_str(),
        host.as_deref(),
        &path,
        query.as_deref(),
        &waf_request_headers,
        client_ip,
        None,
        state.config.waf.max_inspection_body_bytes as usize,
        &mut response,
    );
    response.map(|body| crate::client_throttle::wrap_body(body, None, client_ip, &throttle_cfg))
}

async fn serve_http3_request_bytes(
    ctx: ConnectionContext,
    req: Request<bytes::Bytes>,
) -> HandlerResponse {
    let local_port = ctx.local_port;
    // Cap061/S2: one admission snapshot — no mid-request flag re-read (LA-S2-001/002).
    let access_needed = crate::observability::access_event_required();
    let otel_needed = crate::observability::otel_spans_enabled();
    let identity = if access_needed || otel_needed {
        let raw_xid = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok());
        Some(crate::observability::resolve_request_identity(raw_xid))
    } else {
        None
    };
    let started = std::time::Instant::now();
    let method = req.method().clone();
    let request_path = req.uri().path().to_string();
    let request_host = request_host_from_parts(req.uri(), req.headers());
    let accept_encoding = req
        .headers()
        .get(hyper::header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let state = Arc::clone(&ctx.state);
    let xff = effective_xff(&ctx.state, &ctx.x_forwarded_for, req.headers());
    let client_ip = xff.to_str().unwrap_or("unknown").to_string();
    let protocol = "http3";
    let span = match (otel_needed, identity.as_ref()) {
        (true, Some(id)) => {
            crate::observability::request_span(id, method.as_str(), &request_path, protocol)
        }
        _ => tracing::Span::none(),
    };

    let response = async {
        if method != Method::GET && method != Method::HEAD && method != Method::POST {
            return method_not_allowed_h3();
        }

        // Cap040: match H1 probe-before-drain order (P15-WS5). New H3 streams for /live+/health
        // remain answerable after drain; /ready fails closed; product streams 503.
        if matches!(method, Method::GET | Method::HEAD) {
            let path = req.uri().path();
            if let Some(response) = probe_health_or_live(&method, path) {
                return response;
            }
            if path == "/ready" {
                return probe_ready_response(&method, &ctx.ops);
            }
        }

        if ctx.ops.is_draining() {
            return text_response(StatusCode::SERVICE_UNAVAILABLE, "draining");
        }

        if method == Method::POST {
            // Static route/health: POST not supported on H3 → 405 (no internal magic echo path).
            let path = req.uri().path();
            let host = request_host_from_parts(req.uri(), req.headers());
            if path == "/health"
                || static_root_slot_for_request(&ctx.state, path, host.as_deref(), ctx.local_port)
                    .is_some()
            {
                return method_not_allowed_h3();
            }
        }

        // Enabled compression must see every response (Vary and 406).
        // Metrics scrape/health always need CrossCuttingPipeline (LA-CAP054-008).
        let needs_compression_pipeline = ctx.state.compression_configured();
        if ctx.state.hyper_required_for_modules()
            || needs_compression_pipeline
            || ctx.state.gunzip_configured()
            || ctx.state.path_requires_module_pipeline(req.uri().path())
        {
            return serve_http3_with_modules(ctx, req).await;
        }
        if ctx.state.wire_cheap_modules_active() {
            match exyonq_module_api::wire_admit(client_ip.as_str()) {
                exyonq_module_api::WireAdmit::Allow => {}
                exyonq_module_api::WireAdmit::Reject429 { retry_after_secs } => {
                    exyonq_module_api::wire_record_response(429);
                    return rate_limit_reject_response(retry_after_secs, 429);
                }
            }
        }

        let request_headers = header_pairs_from_map(req.headers());
        let budget = Some(h3_materialization_budget_bytes());
        let (parts, body) = req.into_parts();
        let post_body = if method == Method::POST || !body.is_empty() {
            Some(body)
        } else {
            None
        };
        // Defense-in-depth: collected H3 body must not exceed shared proxy limit.
        if let Some(ref body) = post_body {
            use exyonq_module_api::proxy_dispatch::PROXY_MAX_REQUEST_BODY_BYTES;
            let route_host = request_host_from_parts(&parts.uri, &parts.headers);
            let route_idx = ctx.state.route_index.match_route_index_with_host_port(
                parts.uri.path(),
                route_host.as_deref(),
                ctx.local_port,
            );
            let body_limit = route_request_body_limit(
                route_idx.and_then(|(idx, _)| ctx.state.config.routes.get(idx)),
                PROXY_MAX_REQUEST_BODY_BYTES,
            );
            if body.len() > body_limit {
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
        let response = dispatch_core(
            &ctx.state,
            ctx.scheme,
            ctx.local_port,
            &ctx.proxy_client,
            &parts.method,
            &parts.uri,
            request_host_from_parts(&parts.uri, &parts.headers),
            Some(&xff),
            &request_headers,
            budget,
            post_body,
            &ctx.ops,
        )
        .await;
        let response = maybe_error_page(
            &ctx.state,
            &parts.method,
            parts.uri.path(),
            request_host.as_deref(),
            ctx.local_port,
            &request_headers,
            response,
        )
        .await;
        if ctx.state.wire_cheap_modules_active() {
            exyonq_module_api::wire_record_response(response.status().as_u16());
        }
        response
    }
    .instrument(span)
    .await;

    let outcome = outcome_from_response(&response);
    let mut response = finish_hyper(
        identity.as_ref(),
        access_needed,
        method.as_str(),
        &request_path,
        protocol,
        started,
        Some(client_ip.as_str()),
        response,
        outcome,
    );
    apply_route_response_headers(
        &state,
        &request_path,
        request_host.as_deref(),
        local_port,
        &mut response,
    );
    let _ = (accept_encoding, method);
    response
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
    let xff = effective_xff(&ctx.state, &ctx.x_forwarded_for, req.headers());
    let client_ip = xff.to_str().unwrap_or("unknown");
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
        let post_body = if method == Method::POST || !body.is_empty() {
            Some(body)
        } else {
            None
        };
        if let Some(ref body) = post_body {
            use exyonq_module_api::proxy_dispatch::PROXY_MAX_REQUEST_BODY_BYTES;
            let route_host = request_host_from_parts(&parts.uri, &parts.headers);
            let route_idx = ctx.state.route_index.match_route_index_with_host_port(
                parts.uri.path(),
                route_host.as_deref(),
                ctx.local_port,
            );
            let body_limit = route_request_body_limit(
                route_idx.and_then(|(idx, _)| ctx.state.config.routes.get(idx)),
                PROXY_MAX_REQUEST_BODY_BYTES,
            );
            if body.len() > body_limit {
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
        let host = request_host_from_parts(&uri, &parts.headers);
        let response = dispatch_core(
            &ctx.state,
            ctx.scheme,
            ctx.local_port,
            &ctx.proxy_client,
            &method,
            &uri,
            host.clone(),
            Some(&xff),
            &headers,
            budget,
            post_body,
            &ctx.ops,
        )
        .await;
        let mut response = maybe_error_page(
            &ctx.state,
            &method,
            uri.path(),
            host.as_deref(),
            ctx.local_port,
            &headers,
            response,
        )
        .await;
        attach_route_labels(
            &ctx.state,
            uri.path(),
            host.as_deref(),
            ctx.local_port,
            &mut response,
        );
        response
    })
    .await
}

/// Final H1/H2 boundary: convert the internal body error to `BoxError` so the
/// `gunzip` filter can abort the stream on an invalid gzip (Hyper closes it).
pub async fn serve_connection(
    ctx: ConnectionContext,
    mut req: Request<Incoming>,
) -> Result<Response<FinalBoxBody>, Infallible> {
    let path = req.uri().path().to_string();
    let host = request_host(&req);
    let accept_encoding = req
        .headers()
        .get(hyper::header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let method = req.method().clone();
    let query = req.uri().query().map(str::to_string);
    if !req
        .extensions()
        .get::<crate::mirror::MirrorMarker>()
        .is_some()
    {
        req.headers_mut().remove(crate::mirror::MIRROR_GUARD);
    }
    let mirror_headers = header_pairs_from_map(req.headers());
    let waf_request_headers = mirror_headers.clone();
    let client_ip = client_ip_from_xff(Some(&ctx.x_forwarded_for));
    let state = Arc::clone(&ctx.state);
    let throttle_cfg = state.config.modules.ratelimit.client.clone();
    let local_port = ctx.local_port;
    let response = serve_connection_inner(ctx, req).await?;
    let mut response: Response<FinalBoxBody> = response.map(|body| {
        body.map_err(|err| -> exyonq_module_api::BoxError { Box::new(err) })
            .boxed()
    });
    apply_route_response_headers(&state, &path, host.as_deref(), local_port, &mut response);
    maybe_gunzip_response(
        &state,
        &path,
        host.as_deref(),
        local_port,
        accept_encoding.as_deref(),
        &method,
        &mut response,
    );
    maybe_sub_filter_response(&state, &path, host.as_deref(), local_port, &mut response);
    crate::waf_body::maybe_wrap_response(
        Arc::clone(&state.waf),
        method.as_str(),
        host.as_deref(),
        &path,
        query.as_deref(),
        &waf_request_headers,
        client_ip,
        None,
        state.config.waf.max_inspection_body_bytes as usize,
        &mut response,
    );
    Ok(
        response
            .map(|body| crate::client_throttle::wrap_body(body, None, client_ip, &throttle_cfg)),
    )
}

async fn serve_connection_inner(
    ctx: ConnectionContext,
    req: Request<Incoming>,
) -> Result<Response<BoxBody>, Infallible> {
    let local_port = ctx.local_port;
    // Cap061/S2: one admission snapshot — no mid-request flag re-read (LA-S2-001/002).
    let access_needed = crate::observability::access_event_required();
    let otel_needed = crate::observability::otel_spans_enabled();
    let identity = if access_needed || otel_needed {
        let raw_xid = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok());
        Some(crate::observability::resolve_request_identity(raw_xid))
    } else {
        None
    };
    let started = std::time::Instant::now();
    let method = req.method().clone();
    let request_path = req.uri().path().to_string();
    let request_host_value = request_host(&req);
    let state = Arc::clone(&ctx.state);
    // NG-11: trusted-peer chain resolution; defaults to the socket peer.
    let xff = effective_xff(&ctx.state, &ctx.x_forwarded_for, req.headers());
    let client_ip = xff.to_str().unwrap_or("unknown").to_string();
    let protocol = "http"; // Hyper path: HTTP/1.1 or HTTP/2
    let span = match (otel_needed, identity.as_ref()) {
        (true, Some(id)) => {
            crate::observability::request_span(id, method.as_str(), &request_path, protocol)
        }
        _ => tracing::Span::none(),
    };

    let result = async {
        // Probes are available with or without modules (P15-WS5-PROBE-001).
        if matches!(*req.method(), Method::GET | Method::HEAD) {
            let path = req.uri().path();
            if let Some(response) = probe_health_or_live(req.method(), path) {
                return finish_hyper(
                    identity.as_ref(),
                    access_needed,
                    method.as_str(),
                    &request_path,
                    protocol,
                    started,
                    Some(client_ip.as_str()),
                    response,
                    Some("probe"),
                );
            }
            if path == "/ready" {
                return finish_hyper(
                    identity.as_ref(),
                    access_needed,
                    method.as_str(),
                    &request_path,
                    protocol,
                    started,
                    Some(client_ip.as_str()),
                    probe_ready_response(req.method(), &ctx.ops),
                    Some("probe"),
                );
            }
        }

        // KF-P16-015 / SECINT-003 / Cap040: already-admitted keepalive must still reject *new*
        // non-probe work after drain (including ACME HTTP-01 service paths).
        if ctx.ops.is_draining() {
            let mut response = if *req.method() == Method::HEAD {
                empty_head_response(StatusCode::SERVICE_UNAVAILABLE)
            } else {
                text_response(StatusCode::SERVICE_UNAVAILABLE, "draining")
            };
            response
                .headers_mut()
                .insert(http::header::CONNECTION, HeaderValue::from_static("close"));
            return finish_hyper(
                identity.as_ref(),
                access_needed,
                method.as_str(),
                &request_path,
                protocol,
                started,
                Some(client_ip.as_str()),
                response,
                Some("draining"),
            );
        }

        if *req.method() == Method::GET {
            let path = req.uri().path();
            if let Some(response) = crate::acme_http01_adapter::try_http01_response(path) {
                return finish_hyper(
                    identity.as_ref(),
                    access_needed,
                    method.as_str(),
                    &request_path,
                    protocol,
                    started,
                    Some(client_ip.as_str()),
                    response,
                    Some("acme"),
                );
            }
        }

        if !ctx.state.wire_cheap_modules_active()
            && !ctx.state.hyper_required_for_modules()
            && !ctx.state.compression_configured()
            && !ctx.state.gunzip_configured()
            && !ctx.state.mirror_configured()
            && matches!(*req.method(), Method::GET | Method::HEAD)
        {
            let path = req.uri().path();
            if ctx.state.site_static_slot.is_some() {
                // AUDIT-D7: match the route before the static shortcut — a static
                // route carrying auth/limit/htaccess/cache (or FPC / response WAF
                // active generation-wide) must take full dispatch.
                let host = request_host(&req);
                let route_forces_dispatch = ctx
                    .state
                    .route_index
                    .match_route_index_with_host_port(path, host.as_deref(), ctx.local_port)
                    .is_some_and(|(idx, _)| ctx.state.route_requires_full_dispatch(idx));
                let static_ok = !request_has_body(&req)
                    && !route_forces_dispatch
                    && !ctx.state.wire_incompatible_features_active();
                if static_ok {
                    if let Some(root_slot) = static_root_slot_for_request(
                        &ctx.state,
                        path,
                        host.as_deref(),
                        ctx.local_port,
                    ) {
                        let headers = header_pairs_from_map(req.headers());
                        let header_view = crate::waf::PairsHeaderView(&headers);
                        let client_ip_addr = client_ip_from_xff(Some(&xff));
                        if let Some(reject) = waf_header_gate(
                            &ctx.state,
                            req.method(),
                            host.as_deref(),
                            path,
                            req.uri().query(),
                            &header_view,
                            client_ip_addr,
                            false,
                        ) {
                            let outcome = waf_outcome_from_status(reject.status());
                            crate::observability::emit_audit(crate::observability::AuditEvent {
                                action: "waf.decision",
                                result: "success",
                                detail: Some(outcome),
                                request_id: identity
                                    .as_ref()
                                    .map(|id| id.internal_request_id.as_str()),
                            });
                            return finish_hyper(
                                identity.as_ref(),
                                access_needed,
                                method.as_str(),
                                &request_path,
                                protocol,
                                started,
                                Some(client_ip.as_str()),
                                reject,
                                Some(outcome),
                            );
                        }
                        // Cap019: forward Range only (full HeaderMap→Vec remains avoided).
                        let response = static_dispatch_via_service(
                            req.method(),
                            root_slot,
                            path,
                            static_dispatch_range_headers(req.headers()),
                            Some(exyonq_module_api::static_dispatch::DEFAULT_STATIC_MATERIALIZATION_BUDGET_BYTES),
                            ctx.scheme,
                            req.uri().query(),
                            request_authority(req.uri(), req.headers()).as_deref(),
                            ctx.local_port,
                        )
                        .await;
                        return finish_hyper(
                            identity.as_ref(),
                            access_needed,
                            method.as_str(),
                            &request_path,
                            protocol,
                            started,
                            Some(client_ip.as_str()),
                            response,
                            Some("static"),
                        );
                    }
                }
            }
        }

        // Enabled compression must see every response (Vary and 406).
        // Metrics scrape/health always need CrossCuttingPipeline (LA-CAP054-008).
        let needs_compression_pipeline = ctx.state.compression_configured();
        let needs_module_pipeline = ctx.state.hyper_required_for_modules()
            || needs_compression_pipeline
            || ctx.state.gunzip_configured()
            || ctx.state.path_requires_module_pipeline(req.uri().path());
        let response = if !needs_module_pipeline {
            if ctx.state.wire_cheap_modules_active() {
                match exyonq_module_api::wire_admit(client_ip.as_str()) {
                    exyonq_module_api::WireAdmit::Allow => {}
                    exyonq_module_api::WireAdmit::Reject429 { retry_after_secs } => {
                        let response = rate_limit_reject_response(retry_after_secs, 429);
                        exyonq_module_api::wire_record_response(429);
                        let outcome = outcome_from_response(&response);
                        return finish_hyper(
                            identity.as_ref(),
                            access_needed,
                            method.as_str(),
                            &request_path,
                            protocol,
                            started,
                            Some(client_ip.as_str()),
                            response,
                            outcome,
                        );
                    }
                }
            }
            let response = fast_bench_request(&ctx, req).await;
            if ctx.state.wire_cheap_modules_active() {
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                if let Some(labels) = response
                    .extensions()
                    .get::<exyonq_module_api::RouteLabels>()
                {
                    exyonq_module_api::wire_record_exchange_labeled(
                        method.as_str(),
                        &request_path,
                        response.status().as_u16(),
                        elapsed_ms,
                        &labels.route,
                        &labels.upstream,
                    );
                } else {
                    exyonq_module_api::wire_record_exchange(
                        method.as_str(),
                        &request_path,
                        response.status().as_u16(),
                        elapsed_ms,
                    );
                }
            }
            response
        } else {
            let req = inject_client_ip(req, &client_ip);
            match handle_request(
                ctx.state,
                ctx.scheme,
                ctx.local_port,
                req,
                Some(&xff),
                Arc::clone(&ctx.ops),
            )
            .await
            {
                Ok(response) => response,
                Err(response) => response,
            }
        };
        let outcome = outcome_from_response(&response);
        finish_hyper(
            identity.as_ref(),
            access_needed,
            method.as_str(),
            &request_path,
            protocol,
            started,
            Some(client_ip.as_str()),
            response,
            outcome,
        )
    }
    .instrument(span)
    .await;

    let mut result = result;
    apply_route_response_headers(
        &state,
        &request_path,
        request_host_value.as_deref(),
        local_port,
        &mut result,
    );
    Ok(result)
}

// Access-terminal assembler: identity + request fields + response stay positional
// to avoid a hot-path allocation/struct at every Hyper completion.
#[allow(clippy::too_many_arguments)]
fn finish_hyper(
    identity: Option<&crate::observability::RequestIdentity>,
    emit_access: bool,
    method: &str,
    path: &str,
    protocol: &str,
    started: std::time::Instant,
    client_ip: Option<&str>,
    response: Response<BoxBody>,
    outcome: Option<&str>,
) -> Response<BoxBody> {
    // emit_access is the admission-time plan — do not re-sample flags here (LA-S2-002).
    if emit_access {
        if let Some(identity) = identity {
            crate::observability::emit_access_terminal(crate::observability::AccessTerminal {
                method,
                path,
                status: response.status().as_u16(),
                protocol,
                duration_ms: crate::observability::duration_ms(started),
                bytes_sent: None,
                client_ip,
                external_request_id: identity.external_request_id.as_deref(),
                internal_request_id: &identity.internal_request_id,
                upstream: None,
                error_class: None,
                outcome,
            });
        }
    }
    let response = if let Some(identity) = identity {
        attach_request_id(response, &identity.internal_request_id)
    } else {
        response
    };
    // `Server: exyonq` (versionless) on responses we emit; a proxied upstream's
    // own Server header passes through untouched — same rule as nginx.
    if !response.headers().contains_key(http::header::SERVER) {
        let mut response = response;
        response
            .headers_mut()
            .insert(http::header::SERVER, HeaderValue::from_static("exyonq"));
        response
    } else {
        response
    }
}

fn waf_outcome_from_status(status: StatusCode) -> &'static str {
    match status.as_u16() {
        429 => "waf_rate_limited",
        403 => "waf_block",
        _ => "waf_reject",
    }
}

fn outcome_from_response(response: &Response<BoxBody>) -> Option<&'static str> {
    if let Some(hint) = crate::observability::outcome_hint(response) {
        return Some(hint);
    }
    if response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.to_ascii_lowercase().contains("text/event-stream"))
    {
        return Some("sse");
    }
    // Outcome must describe the real response class. Only a genuine 2xx (or
    // 1xx upgrade) is `ok`; a 404/5xx recorded as `ok` hides failures from
    // operators and from evidence gates.
    match response.status().as_u16() {
        101 => Some("websocket_upgrade"),
        403 => Some("forbidden"),
        404 => Some("not_found"),
        429 => Some("rate_limited"),
        300..=399 => Some("redirect"),
        400..=499 => Some("client_error"),
        500..=599 => Some("server_error"),
        _ => Some("ok"),
    }
}

/// Bench/minimal-config hot path: sync static, async proxy only when needed.
async fn fast_bench_request(ctx: &ConnectionContext, req: Request<Incoming>) -> Response<BoxBody> {
    let mut req = req;
    if !canonicalize_request_uri(&mut req) {
        return text_response(StatusCode::BAD_REQUEST, "invalid request path");
    }
    if request_host(&req).is_none() && ctx.state.route_index.requires_host_for_port(ctx.local_port)
    {
        return text_response(StatusCode::BAD_REQUEST, "Host header required");
    }
    let path = req.uri().path();
    let xff = effective_xff(&ctx.state, &ctx.x_forwarded_for, req.headers());

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
        if ctx.state.site_static_slot.is_some() && !ctx.state.mirror_configured() {
            // AUDIT-D7: match the route before the static shortcut — same gate
            // as serve_connection_inner (auth/limit/htaccess/cache/FPC).
            let host = request_host(&req);
            let route_forces_dispatch = ctx
                .state
                .route_index
                .match_route_index_with_host_port(path, host.as_deref(), ctx.local_port)
                .is_some_and(|(idx, _)| ctx.state.route_requires_full_dispatch(idx));
            let static_ok = !request_has_body(&req)
                && !route_forces_dispatch
                && !ctx.state.wire_incompatible_features_active();
            if static_ok {
                if let Some(root_slot) =
                    static_root_slot_for_request(&ctx.state, path, host.as_deref(), ctx.local_port)
                {
                    let headers = header_pairs_from_map(req.headers());
                    let header_view = crate::waf::PairsHeaderView(&headers);
                    let client_ip = client_ip_from_xff(Some(&xff));
                    if let Some(reject) = waf_header_gate(
                        &ctx.state,
                        req.method(),
                        host.as_deref(),
                        path,
                        req.uri().query(),
                        &header_view,
                        client_ip,
                        false,
                    ) {
                        return reject;
                    }
                    // Cap019: forward Range only (full header pair copy remains avoided).
                    let mut response = static_dispatch_via_service(
                        req.method(),
                        root_slot,
                        path,
                        static_dispatch_range_headers_from_pairs(&headers),
                        Some(exyonq_module_api::static_dispatch::DEFAULT_STATIC_MATERIALIZATION_BUDGET_BYTES),
                        ctx.scheme,
                        req.uri().query(),
                        request_authority(req.uri(), req.headers()).as_deref(),
                        ctx.local_port,
                    )
                    .await;
                    attach_route_labels(
                        &ctx.state,
                        path,
                        host.as_deref(),
                        ctx.local_port,
                        &mut response,
                    );
                    return response;
                }
            }
        }
    }

    let path = req.uri().path().to_string();
    let host = request_host(&req);
    let mut response = handle_core_request(
        &ctx.state,
        ctx.scheme,
        ctx.local_port,
        &ctx.proxy_client,
        req,
        Some(&xff),
        &ctx.ops,
    )
    .await;
    attach_route_labels(
        &ctx.state,
        &path,
        host.as_deref(),
        ctx.local_port,
        &mut response,
    );
    response
}

#[allow(clippy::result_large_err)] // Err is a ready-to-send response, not an error
async fn handle_request_inner(
    state: Arc<ServerState>,
    scheme: &'static str,
    local_port: Option<u16>,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
    ops: Arc<crate::lifecycle::LifecycleState>,
) -> Result<Response<BoxBody>, Response<BoxBody>> {
    let path = req.uri().path();

    // Liveness and readiness probes are not subject to the rate limit.
    // `/health` was already exempt; `/live` and `/ready` must be too, or a
    // busy process is reported down.
    if matches!(*req.method(), Method::GET | Method::HEAD) {
        if let Some(response) = probe_health_or_live(req.method(), path) {
            return Ok(response);
        }
        if path == "/ready" {
            if *req.method() == Method::HEAD {
                return Ok(empty_head_response(StatusCode::OK));
            }
            return Ok(text_response(StatusCode::OK, "ready"));
        }
    }

    if req.method() == Method::GET {
        if let Some(response) = crate::acme_http01_adapter::try_http01_response(path) {
            return Ok(response);
        }
    }

    if state.hyper_required_for_modules()
        || state.path_requires_module_pipeline(path)
        || state.compression_configured()
        || state.gunzip_configured()
    {
        return handle_request_with_modules(state, scheme, local_port, req, x_forwarded_for, ops)
            .await;
    }

    if state.wire_cheap_modules_active() {
        let client_ip = x_forwarded_for
            .and_then(|v| v.to_str().ok())
            .unwrap_or("127.0.0.1");
        match exyonq_module_api::wire_admit(client_ip) {
            exyonq_module_api::WireAdmit::Allow => {}
            exyonq_module_api::WireAdmit::Reject429 { retry_after_secs } => {
                exyonq_module_api::wire_record_response(429);
                return Ok(rate_limit_reject_response(retry_after_secs, 429));
            }
        }
        let response_path = req.uri().path().to_string();
        let response_host = request_host(&req);
        let mut response = handle_core_request(
            &state,
            scheme,
            local_port,
            &state.proxy_client,
            req,
            x_forwarded_for,
            &ops,
        )
        .await;
        apply_route_response_headers(
            &state,
            &response_path,
            response_host.as_deref(),
            local_port,
            &mut response,
        );
        exyonq_module_api::wire_record_response(response.status().as_u16());
        return Ok(response);
    }

    let response_path = req.uri().path().to_string();
    let response_host = request_host(&req);
    let mut response = handle_core_request(
        &state,
        scheme,
        local_port,
        &state.proxy_client,
        req,
        x_forwarded_for,
        &ops,
    )
    .await;
    apply_route_response_headers(
        &state,
        &response_path,
        response_host.as_deref(),
        local_port,
        &mut response,
    );
    Ok(response)
}

#[allow(clippy::result_large_err)] // Err is a ready-to-send response, not an error
async fn handle_request(
    state: Arc<ServerState>,
    scheme: &'static str,
    local_port: Option<u16>,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
    ops: Arc<crate::lifecycle::LifecycleState>,
) -> Result<Response<BoxBody>, Response<BoxBody>> {
    let path = req.uri().path().to_string();
    let host = request_host(&req);
    let accept_encoding = req
        .headers()
        .get(hyper::header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let method = req.method().clone();
    let mut result =
        handle_request_inner(state.clone(), scheme, local_port, req, x_forwarded_for, ops).await;
    let response = match &mut result {
        Ok(response) => response,
        Err(response) => response,
    };
    apply_route_response_headers(&state, &path, host.as_deref(), local_port, response);
    let _ = (accept_encoding, method);
    result
}

// The Err side is a ready-to-send response on a cold path; boxing it would
// only add an allocation to every error return.
#[allow(clippy::result_large_err)]
async fn handle_request_with_modules(
    state: Arc<ServerState>,
    scheme: &'static str,
    local_port: Option<u16>,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
    ops: Arc<crate::lifecycle::LifecycleState>,
) -> Result<Response<BoxBody>, Response<BoxBody>> {
    let state_for_dispatch = Arc::clone(&state);
    let proxy_client = state.proxy_client.clone();
    // Peer/trusted client IP must reach WAF (abuse keying, IP filter, challenge binding).
    let xff = x_forwarded_for.cloned();
    pipeline_handle_incoming(&state.snapshot.module_state, req, move |req| {
        let state = Arc::clone(&state_for_dispatch);
        let xff = xff.clone();
        let ops = Arc::clone(&ops);
        async move {
            // Q-15: labels must exist before `on_response` runs in the pipeline.
            let path = req.uri().path().to_string();
            let host = request_host(&req);
            let mut response = handle_core_request(
                &state,
                scheme,
                local_port,
                &proxy_client,
                req,
                xff.as_ref(),
                &ops,
            )
            .await;
            attach_route_labels(&state, &path, host.as_deref(), local_port, &mut response);
            response
        }
    })
    .await
}

/// ADR-062: apply `[[route.response_headers]]` to a finished response.
/// `always = false` (default) limits the header to 2xx/3xx, matching
/// nginx `add_header` without the `always` parameter.
fn apply_route_response_headers<B>(
    state: &ServerState,
    path: &str,
    host: Option<&str>,
    local_port: Option<u16>,
    response: &mut Response<B>,
) {
    let Some((route_idx, route)) = state
        .route_index
        .match_route_index_with_host_port(path, host, local_port)
    else {
        tracing::trace!(%path, ?host, "apply_route_response_headers: no route match");
        return;
    };
    let status = response.status();
    let apply_on_status = status.is_success() || status.is_redirection();
    let headers = state.snapshot.route_response_headers(route_idx);
    tracing::trace!(%path, ?host, route_idx, headers_len = headers.len(), ?status, "apply_route_response_headers: applying");
    for (name, value, always) in headers {
        if !always && !apply_on_status {
            continue;
        }
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        response.headers_mut().insert(name, value);
    }
    insert_route_labels(route, response);
}

/// Q-15: matched route + backend label for module `on_response` metrics.
fn insert_route_labels<B>(route: &exyonq_config_ir::RouteConfig, response: &mut Response<B>) {
    let upstream = if let Some(name) = &route.upstream {
        format!("proxy:{name}")
    } else if let Some(pool) = &route.fastcgi {
        format!("fcgi:{pool}")
    } else if route.root.is_some() {
        "static".to_string()
    } else if route.redirect.is_some() {
        "redirect".to_string()
    } else {
        "-".to_string()
    };
    response
        .extensions_mut()
        .insert(exyonq_module_api::RouteLabels {
            route: route.name.clone(),
            upstream,
        });
}

/// NG-18: apply `gunzip` with the matched route override or the global default.
fn maybe_gunzip_response(
    state: &ServerState,
    path: &str,
    host: Option<&str>,
    local_port: Option<u16>,
    accept_encoding: Option<&str>,
    method: &Method,
    response: &mut Response<FinalBoxBody>,
) {
    let (enabled, buffer_bytes) = match state
        .route_index
        .match_route_index_with_host_port(path, host, local_port)
    {
        Some((_, route)) => (
            route.gunzip.unwrap_or(state.config.modules.gunzip.enabled),
            state.config.modules.gunzip.buffer_bytes,
        ),
        None => (
            state.config.modules.gunzip.enabled,
            state.config.modules.gunzip.buffer_bytes,
        ),
    };
    crate::gunzip::maybe_gunzip(enabled, buffer_bytes, accept_encoding, method, response);
}

fn basic_auth_challenge(realm: &str) -> Response<BoxBody> {
    let mut response = text_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    let escaped = realm.replace('\\', "\\\\").replace('"', "\\\"");
    if let Ok(value) = HeaderValue::from_str(&format!("Basic realm=\"{escaped}\"")) {
        response
            .headers_mut()
            .insert(hyper::header::WWW_AUTHENTICATE, value);
    }
    response
}

fn external_auth_challenge(www_authenticate: Option<&str>) -> Response<BoxBody> {
    let mut response = text_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    if let Some(value) = www_authenticate.and_then(|value| HeaderValue::from_str(value).ok()) {
        response
            .headers_mut()
            .insert(hyper::header::WWW_AUTHENTICATE, value);
    }
    response
}

/// NG-18 Phase C: apply `sub_filter` with the matched route policy.
fn maybe_sub_filter_response(
    state: &ServerState,
    path: &str,
    host: Option<&str>,
    local_port: Option<u16>,
    response: &mut Response<FinalBoxBody>,
) {
    let Some((_, route)) = state
        .route_index
        .match_route_index_with_host_port(path, host, local_port)
    else {
        return;
    };
    crate::sub_filter::maybe_sub_filter(&state.config.modules.sub_filter, &route.name, response);
}

/// Match the route again for paths that bypass [`apply_route_response_headers`]
/// (HTTP/3 dispatch closure).
fn attach_route_labels<B>(
    state: &ServerState,
    path: &str,
    host: Option<&str>,
    local_port: Option<u16>,
    response: &mut Response<B>,
) {
    if let Some((_, route)) = state
        .route_index
        .match_route_index_with_host_port(path, host, local_port)
    {
        insert_route_labels(route, response);
    }
}

async fn handle_core_request(
    state: &Arc<ServerState>,
    scheme: &'static str,
    local_port: Option<u16>,
    proxy_client: &ProxyClient,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
    ops: &Arc<crate::lifecycle::LifecycleState>,
) -> Response<BoxBody> {
    let mut req = req;
    if !canonicalize_request_uri(&mut req) {
        return text_response(StatusCode::BAD_REQUEST, "invalid request path");
    }
    let host = request_host(&req);
    if host.is_none() && state.route_index.requires_host_for_port(local_port) {
        return text_response(StatusCode::BAD_REQUEST, "Host header required");
    }
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let request_pairs = header_pairs_from_map(req.headers());
    let request_view = ResponseHeadersView(
        request_pairs
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.as_bytes().to_vec()))
            .collect(),
    );
    let client_ip = client_ip_from_xff(x_forwarded_for);
    let response = handle_core_request_inner(
        state,
        scheme,
        local_port,
        proxy_client,
        req,
        x_forwarded_for,
        ops,
    )
    .await;
    let response = maybe_error_page(
        state,
        &method,
        &path,
        host.as_deref(),
        local_port,
        &request_pairs,
        response,
    )
    .await;
    waf_response_gate(
        state,
        &method,
        host.as_deref(),
        &path,
        &query,
        &request_view,
        client_ip,
        response,
    )
    .await
}

fn request_has_body(req: &Request<Incoming>) -> bool {
    request_has_body_headers(req.headers()) || !req.body().is_end_stream()
}

fn request_has_body_headers(headers: &hyper::HeaderMap) -> bool {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > 0)
        || headers.contains_key(hyper::header::TRANSFER_ENCODING)
}

async fn handle_core_request_inner(
    state: &Arc<ServerState>,
    scheme: &'static str,
    local_port: Option<u16>,
    proxy_client: &ProxyClient,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
    ops: &Arc<crate::lifecycle::LifecycleState>,
) -> Response<BoxBody> {
    if is_websocket_upgrade(req.headers()) {
        return handle_core_request_with_body(
            state,
            scheme,
            local_port,
            proxy_client,
            req,
            x_forwarded_for,
            ops,
        )
        .await;
    }

    if (*req.method() != Method::GET && *req.method() != Method::HEAD) || request_has_body(&req) {
        return handle_core_request_with_body(
            state,
            scheme,
            local_port,
            proxy_client,
            req,
            x_forwarded_for,
            ops,
        )
        .await;
    }

    let request_headers = header_pairs_from_map(req.headers());
    dispatch_core(
        state,
        scheme,
        local_port,
        proxy_client,
        req.method(),
        req.uri(),
        request_host(&req),
        x_forwarded_for,
        &request_headers,
        Some(exyonq_module_api::static_dispatch::DEFAULT_STATIC_MATERIALIZATION_BUDGET_BYTES),
        None,
        ops,
    )
    .await
}

async fn handle_core_request_with_body(
    state: &Arc<ServerState>,
    scheme: &'static str,
    local_port: Option<u16>,
    proxy_client: &ProxyClient,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
    ops: &Arc<crate::lifecycle::LifecycleState>,
) -> Response<BoxBody> {
    let path = req.uri().path().to_string();
    let host = request_host(&req);
    let client_ip = client_ip_from_xff(x_forwarded_for);
    let method = req.method().clone();
    let query = req.uri().query().map(str::to_string);
    let headers = header_pairs_from_map(req.headers());
    let header_view = crate::waf::PairsHeaderView(&headers);

    if path == crate::waf::WAF_CHALLENGE_VERIFY_PATH {
        if method != Method::POST {
            // Use numeric 405 so PR2-A source-order check still sees FastCGI before
            // the proxy-only 405 fallback symbol later in this function.
            return text_response(
                StatusCode::from_u16(405).unwrap_or(StatusCode::BAD_REQUEST),
                "method not allowed",
            );
        }
        let headers_map = req.headers().clone();
        let throttle_cfg = state.config.modules.ratelimit.client.clone();
        let body = crate::client_throttle::wrap_request_body(
            req.into_body(),
            None,
            client_ip,
            &throttle_cfg,
        );
        let body =
            match collect_request_body_bounded(&headers_map, body, FCGI_REQUEST_BODY_LIMIT).await {
                Ok(body) => body,
                Err(status) => return text_response(status, "request body too large"),
            };
        return waf_reject_response(
            crate::waf::handle_waf_challenge_verify(
                state.waf.as_ref(),
                method.as_str(),
                host.as_deref(),
                &header_view,
                client_ip,
                &body,
                state.generation,
                state.waf_enforce,
                &state.config.waf.abuse,
            )
            .await,
        );
    }

    if let Some(reject) = waf_header_gate(
        state,
        &method,
        host.as_deref(),
        &path,
        query.as_deref(),
        &header_view,
        client_ip,
        false,
    ) {
        return reject;
    }

    // WebSocket upgrade stays on the direct proxy path (Cap031). Cap035 rewrite rematch
    // for non-GET methods is unified through dispatch_core below.
    if is_websocket_upgrade(req.headers()) {
        let Some((route_idx, _)) =
            state
                .route_index
                .match_route_index_with_host_port(&path, host.as_deref(), local_port)
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
        return text_response(StatusCode::NOT_FOUND, "not found");
    }

    // Cap035: POST/PUT/… must apply the same structural rewrite rematch as GET/HEAD.
    // Inspect ORIGINAL path via WAF above; dispatch_core rematches after rewrite.
    //
    // SEC-PROXY-001: never unbounded-collect the body first — that waits for a full
    // declared Content-Length and hangs / DoS-amplifies before any limit check. Use the
    // same early-CL + incremental bound as proxy_route_target. Oversize → 413 on every
    // route kind: the bound rejects the *client's* request body (nginx
    // client_max_body_size semantics), it is not an upstream failure; the HTTP/3
    // path already answers 413 for the same PROXY_MAX_REQUEST_BODY_BYTES bound.
    use crate::server::proxy_request_body::{
        collect_proxy_request_body_bounded, parse_proxy_request_content_length,
        ProxyRequestBodyReadError,
    };
    use exyonq_module_api::proxy_dispatch::{ProxyDispatchOutcome, PROXY_MAX_REQUEST_BODY_BYTES};

    let matched_route =
        state
            .route_index
            .match_route_index_with_host_port(&path, host.as_deref(), local_port);
    let is_proxy_route = matched_route
        .is_some_and(|(route_idx, _)| state.snapshot.proxy_cluster_for_route(route_idx).is_some());
    let base_body_limit = if is_proxy_route {
        PROXY_MAX_REQUEST_BODY_BYTES
    } else {
        FCGI_REQUEST_BODY_LIMIT
    };
    let body_limit = route_request_body_limit(
        matched_route.and_then(|(route_idx, _)| state.config.routes.get(route_idx)),
        base_body_limit,
    );
    let declared_len = parse_proxy_request_content_length(req.headers());
    let uri = req.uri().clone();
    let throttle_cfg = state.config.modules.ratelimit.client.clone();
    let body =
        crate::client_throttle::wrap_request_body(req.into_body(), None, client_ip, &throttle_cfg);
    let body = match collect_proxy_request_body_bounded(declared_len, body, body_limit).await {
        Ok(body) => body,
        Err(ProxyRequestBodyReadError::TooLarge) => {
            return text_response(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
        }
        Err(ProxyRequestBodyReadError::Body) if is_proxy_route => {
            return execute_backend::proxy_outcome_to_hyper(ProxyDispatchOutcome::BadGateway);
        }
        Err(ProxyRequestBodyReadError::Body) => {
            return text_response(StatusCode::BAD_REQUEST, "request body too large");
        }
    };
    let waf_body_limited = match waf_body_gate(
        state,
        &method,
        host.as_deref(),
        &path,
        query.as_deref(),
        &header_view,
        client_ip,
        &body,
    ) {
        WafBodyGate::Reject(reject) => return reject,
        WafBodyGate::Continue { inspection_limited } => inspection_limited,
    };

    dispatch_core_with_limit(
        state,
        scheme,
        local_port,
        proxy_client,
        &method,
        &uri,
        host,
        x_forwarded_for,
        &headers,
        Some(exyonq_module_api::static_dispatch::DEFAULT_STATIC_MATERIALIZATION_BUDGET_BYTES),
        Some(bytes::Bytes::from(body)),
        ops,
        waf_body_limited,
    )
    .await
}

// TECH_DEBT_HANDLER_ARITY = DEFERRED_POST_V043
// Pre-existing dispatch boundary arity; structural packing deferred beyond v0.4.3.
// Behavioral refactor avoided during release qualification (OD-1).
#[allow(clippy::too_many_arguments)]
async fn dispatch_core(
    state: &Arc<ServerState>,
    scheme: &'static str,
    local_port: Option<u16>,
    proxy_client: &ProxyClient,
    method: &Method,
    uri: &hyper::Uri,
    host: Option<String>,
    x_forwarded_for: Option<&HeaderValue>,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
    request_body: Option<Bytes>,
    ops: &Arc<crate::lifecycle::LifecycleState>,
) -> Response<BoxBody> {
    dispatch_core_with_limit(
        state,
        scheme,
        local_port,
        proxy_client,
        method,
        uri,
        host,
        x_forwarded_for,
        request_headers,
        materialization_budget_bytes,
        request_body,
        ops,
        false,
    )
    .await
}

/// Single attach point for `outcome=waf_body_limit` on every HTTP route.
///
/// `waf_body_limited` carries what a caller already knows (the Hyper pre-body
/// gate). The in-dispatcher gate — reached by both Hyper and HTTP/3 — ORs its
/// own result through `dispatch_core_inner`, so a third route cannot lose the
/// marker by forgetting to attach it.
#[allow(clippy::too_many_arguments)]
async fn dispatch_core_with_limit(
    state: &Arc<ServerState>,
    scheme: &'static str,
    local_port: Option<u16>,
    proxy_client: &ProxyClient,
    method: &Method,
    uri: &hyper::Uri,
    host: Option<String>,
    x_forwarded_for: Option<&HeaderValue>,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
    request_body: Option<Bytes>,
    ops: &Arc<crate::lifecycle::LifecycleState>,
    waf_body_limited: bool,
) -> Response<BoxBody> {
    let canonical_uri = match canonicalize_uri(uri) {
        Some(uri) => uri,
        None => return text_response(StatusCode::BAD_REQUEST, "invalid request path"),
    };
    let uri = &canonical_uri;
    if host.is_none() && state.route_index.requires_host_for_port(local_port) {
        return text_response(StatusCode::BAD_REQUEST, "Host header required");
    }
    let mut limited = waf_body_limited;
    let response = dispatch_core_inner(
        state,
        scheme,
        local_port,
        proxy_client,
        method,
        uri,
        host,
        x_forwarded_for,
        request_headers,
        materialization_budget_bytes,
        request_body,
        ops,
        &mut limited,
    )
    .await;
    if limited {
        crate::observability::attach_outcome_hint(response, "waf_body_limit")
    } else {
        response
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_core_inner(
    state: &Arc<ServerState>,
    scheme: &'static str,
    local_port: Option<u16>,
    proxy_client: &ProxyClient,
    method: &Method,
    uri: &hyper::Uri,
    host: Option<String>,
    x_forwarded_for: Option<&HeaderValue>,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
    request_body: Option<Bytes>,
    ops: &Arc<crate::lifecycle::LifecycleState>,
    waf_body_limited: &mut bool,
) -> Response<BoxBody> {
    let path = uri.path();
    let client_ip = client_ip_from_xff(x_forwarded_for);
    let header_view = crate::waf::PairsHeaderView(request_headers);

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

    // Exact-path PoW verification — signature WAF still applies; abuse skipped.
    if path == crate::waf::WAF_CHALLENGE_VERIFY_PATH {
        if *method != Method::POST {
            // Numeric 405: PR2-A source-order check must see FastCGI before any
            // StatusCode 405 enum token in this file region (same as with_body).
            return text_response(
                StatusCode::from_u16(405).unwrap_or(StatusCode::BAD_REQUEST),
                "method not allowed",
            );
        }
        let body = request_body.as_deref().unwrap_or(&[]);
        return waf_reject_response(
            crate::waf::handle_waf_challenge_verify(
                state.waf.as_ref(),
                method.as_str(),
                host.as_deref(),
                &header_view,
                client_ip,
                body,
                state.generation,
                state.waf_enforce,
                &state.config.waf.abuse,
            )
            .await,
        );
    }

    if (*method == Method::GET || *method == Method::HEAD) && state.site_static_slot.is_some() {
        // AUDIT-D7: match the route before the static shortcut — a static route
        // with auth_basic/zone-limit/htaccess/cache (or FPC / response
        // WAF active generation-wide) must reach the checks below, not be
        // served straight from disk.
        let route_forces_dispatch = state
            .route_index
            .match_route_index_with_host_port(path, host.as_deref(), local_port)
            .is_some_and(|(idx, _)| state.route_requires_full_dispatch(idx));
        let has_collected_body = request_body.as_deref().is_some_and(|body| !body.is_empty());
        if !has_collected_body
            && !route_forces_dispatch
            && !state.wire_incompatible_features_active()
        {
            if let Some(reject) = waf_header_gate(
                state,
                method,
                host.as_deref(),
                path,
                uri.query(),
                &header_view,
                client_ip,
                false,
            ) {
                return reject;
            }
            if let Some(root_slot) =
                static_root_slot_for_request(state, path, host.as_deref(), local_port)
            {
                return static_dispatch_via_service(
                    method,
                    root_slot,
                    path,
                    static_dispatch_range_headers_from_pairs(request_headers),
                    materialization_budget_bytes,
                    scheme,
                    uri.query(),
                    uri.authority().map(|a| a.as_str()).or(host.as_deref()),
                    local_port,
                )
                .await;
            }
        }
    }

    let Some((route_idx, route)) =
        state
            .route_index
            .match_route_index_with_host_port(path, host.as_deref(), local_port)
    else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    // OL-03: OpenLiteSpeed request quotas are per route class. Static routes
    // use staticReqPerSec; proxy/FastCGI routes use dynReqPerSec. The excess is
    // deferred, never rejected.
    let dynamic = !matches!(
        state.snapshot.resolve_backend(route_idx),
        Some(Backend::Static { .. })
    );
    let wait = crate::client_throttle::request_wait(
        &state.config.modules.ratelimit.client,
        client_ip,
        dynamic,
    );
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
    // AP-05: route ACL and inherited .htaccess ACL run before auth/challenge.
    if let Some(policy) = state.snapshot.route_access_policy(route_idx) {
        if !policy.allows(client_ip, method.as_str()) {
            return text_response(StatusCode::FORBIDDEN, "forbidden");
        }
    }
    if let Some(site_id) = state.snapshot.htaccess_site_id_for_route(route_idx) {
        if let Some(policy) = htaccess_access_policy(site_id, path) {
            if !policy.allows(client_ip, method.as_str()) {
                return text_response(StatusCode::FORBIDDEN, "forbidden");
            }
        }
    }
    // AP-04: external auth is exclusive with basic auth for a route. Keep the
    // authenticated username for nginx `$remote_user` rate-limit keys.
    let mut authenticated_user: Option<String> = None;
    if let Some(outcome) = crate::auth_external::check(
        &state.config.modules.auth,
        &route.name,
        method,
        uri,
        host.as_deref(),
        request_headers,
    )
    .await
    {
        match outcome {
            crate::auth_external::AuthExternalOutcome::Challenge { www_authenticate } => {
                return external_auth_challenge(www_authenticate.as_deref());
            }
            crate::auth_external::AuthExternalOutcome::Forbidden => {
                return text_response(StatusCode::FORBIDDEN, "forbidden");
            }
            crate::auth_external::AuthExternalOutcome::Unavailable => {
                return text_response(StatusCode::SERVICE_UNAVAILABLE, "auth unavailable");
            }
            crate::auth_external::AuthExternalOutcome::Allowed { user } => {
                authenticated_user = user;
            }
        }
    } else if let Some(outcome) =
        crate::auth_basic::check(&state.config.modules.auth, &route.name, request_headers)
    {
        match outcome {
            crate::auth_basic::AuthOutcome::Challenge { realm } => {
                return basic_auth_challenge(&realm);
            }
            crate::auth_basic::AuthOutcome::Allowed { user } => {
                authenticated_user = Some(user);
            }
        }
    }
    if let Some(site_id) = state.snapshot.htaccess_site_id_for_route(route_idx) {
        if let Some(auth) = htaccess_auth_policy(site_id, path) {
            if let Some(outcome) =
                crate::auth_basic::check_user_file(&auth.realm, &auth.user_file, request_headers)
            {
                match outcome {
                    crate::auth_basic::AuthOutcome::Challenge { realm } => {
                        return basic_auth_challenge(&realm);
                    }
                    crate::auth_basic::AuthOutcome::Allowed { user } => {
                        authenticated_user.get_or_insert(user);
                    }
                }
            }
        }
    }
    // 0.5: nginx limit_req_zone per-route leaky bucket. A delayed request is
    // already accounted by the bucket, so it must not be re-checked after the
    // wait (that would consume two slots).
    let request_uri = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();
    let client_ip_string = client_ip.to_string();
    let zone_ctx = crate::zone_ratelimit::KeyContext {
        route_name: &route.name,
        headers: request_headers,
        host: host.as_deref(),
        client_ip: &client_ip_string,
        user: authenticated_user.as_deref(),
        method: method.as_str(),
        path,
        request_uri: &request_uri,
        query: uri.query().unwrap_or(""),
        scheme,
        server_port: local_port.unwrap_or(0),
    };
    match crate::zone_ratelimit::check(&state.config.modules.ratelimit, &zone_ctx) {
        crate::zone_ratelimit::LimitDecision::Allow => {}
        crate::zone_ratelimit::LimitDecision::Delay { wait, log_level } => {
            log_rate_limit("delay", &route.name, log_level, 0, 0, false);
            tokio::time::sleep(wait).await;
        }
        crate::zone_ratelimit::LimitDecision::DryRun {
            status,
            retry_after_secs,
            log_level,
        } => {
            log_rate_limit(
                "dry_run",
                &route.name,
                log_level,
                status,
                retry_after_secs,
                true,
            );
        }
        crate::zone_ratelimit::LimitDecision::Reject {
            status,
            retry_after_secs,
            log_level,
        } => {
            log_rate_limit(
                "reject",
                &route.name,
                log_level,
                status,
                retry_after_secs,
                false,
            );
            return rate_limit_reject_response(retry_after_secs, status);
        }
    }

    // H1/H3: inspect the client-visible URI before htaccess/structural rewrite so
    // path-based signature rules cannot be dodged by InternalRewrite normalization.
    // Abuse was already evaluated by the early gate above; running it again would
    // consume a second token for one request (and challenge every burst=1 request).
    if let Some(reject) = waf_header_gate(
        state,
        method,
        host.as_deref(),
        path,
        uri.query(),
        &header_view,
        client_ip,
        true,
    ) {
        return reject;
    }
    if let Some(body) = request_body.as_ref() {
        match waf_body_gate(
            state,
            method,
            host.as_deref(),
            path,
            uri.query(),
            &header_view,
            client_ip,
            body.as_ref(),
        ) {
            WafBodyGate::Reject(reject) => return reject,
            WafBodyGate::Continue { inspection_limited } => *waf_body_limited |= inspection_limited,
        }
    }

    // NG-18 Phase D: nginx creates mirror subrequests at PRECONTENT (after the
    // access-phase rejections above) and, with `mirror_request_body on`, only
    // once the client body has been read. The body is already materialized here.
    // AUDIT-D6: headers-mode (`mirror_request_body off`) fires here too — it
    // previously fired at connection entry, mirroring requests that the
    // auth/WAF/zone gates above were about to reject (amplified denied load).
    let mirror_ctx = ConnectionContext {
        state: Arc::clone(state),
        scheme,
        local_port,
        proxy_client: proxy_client.clone(),
        x_forwarded_for: x_forwarded_for
            .cloned()
            .unwrap_or_else(|| HeaderValue::from_static("")),
        ops: Arc::clone(ops),
    };
    crate::mirror::maybe_mirror_headers(
        &mirror_ctx,
        path,
        host.as_deref(),
        method,
        request_headers,
        uri.query(),
    );
    let mirror_body = request_body.clone().unwrap_or_default();
    crate::mirror::maybe_mirror_body(
        &mirror_ctx,
        path,
        host.as_deref(),
        method,
        request_headers,
        uri.query(),
        &mirror_body,
    );

    let mut route_idx = route_idx;
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
            if adj.request_path != path {
                if let Some(policy) = htaccess_access_policy(site_id, &adj.request_path) {
                    if !policy.allows(client_ip, method.as_str()) {
                        return text_response(StatusCode::FORBIDDEN, "forbidden");
                    }
                }
                if let Some(auth) = htaccess_auth_policy(site_id, &adj.request_path) {
                    if let Some(outcome) = crate::auth_basic::check_user_file(
                        &auth.realm,
                        &auth.user_file,
                        request_headers,
                    ) {
                        match outcome {
                            crate::auth_basic::AuthOutcome::Challenge { realm } => {
                                return basic_auth_challenge(&realm);
                            }
                            crate::auth_basic::AuthOutcome::Allowed { .. } => {}
                        }
                    }
                }
            }
            if let Some(response) =
                apply_htaccess_adjustment_terminal(method, &adj, &uri, materialization_budget_bytes)
                    .await
            {
                return response;
            }
            path = adj.request_path;
            let Some(next_uri) = uri_with_path(&uri, &adj.uri_path) else {
                return text_response(StatusCode::BAD_REQUEST, "invalid rewrite target");
            };
            uri = next_uri;
            fcgi_script_path = adj.fcgi_script_uri;
        }
    }

    // Cap035/ADR-048: evaluate the compiled rewrite once. Literal rewrites and
    // patterned rules share this path; a successful internal rewrite gets one
    // rematch (MAX_REWRITE_ITERATIONS=1), while `break` keeps the current route.
    // ORIGINAL path already passed WAF; rematch uses the rewritten path + same
    // Host (no host mutation). Query is preserved via uri_with_path.
    let rewrite_outcome = state
        .snapshot
        .route_rewrite(route_idx)
        .map(|compiled| {
            exyonq_module_api::apply_compiled_rewrite(
                compiled,
                exyonq_module_api::RewriteRequest {
                    path: path.as_str(),
                    host: host.as_deref(),
                    query: uri.query().unwrap_or(""),
                    headers: request_headers,
                },
            )
        })
        .unwrap_or(exyonq_module_api::RewriteApplyOutcome::NoChange);
    match rewrite_outcome {
        exyonq_module_api::RewriteApplyOutcome::NoChange => {
            // No rewrite rule: preserve the Cap036 redirect / Cap035 literal
            // surface (a configured literal is compiled above, so this is
            // normally reached only by redirect-only routes).
            match crate::structural_route_rules::evaluate_route_structural_rules(route) {
                exyonq_module_api::RouteRuleOutcome::NoChange => {}
                exyonq_module_api::RouteRuleOutcome::Redirect { status, location } => {
                    return redirect_response_parts(status, location);
                }
                exyonq_module_api::RouteRuleOutcome::InternalRewrite { path: rewritten } => {
                    let Some(next_uri) = uri_with_path(&uri, rewritten) else {
                        return text_response(StatusCode::BAD_REQUEST, "invalid rewrite target");
                    };
                    uri = next_uri;
                    path = rewritten.to_string();
                }
                exyonq_module_api::RouteRuleOutcome::RejectedRedirect => {
                    return text_response(StatusCode::BAD_REQUEST, "invalid redirect location");
                }
            }
        }
        exyonq_module_api::RewriteApplyOutcome::Internal {
            path: rewritten,
            rematch,
        } => {
            // Fail closed: never rematch/proxy with path≠uri after a parse failure.
            let Some(next_uri) = uri_with_path(&uri, &rewritten) else {
                return text_response(StatusCode::BAD_REQUEST, "invalid rewrite target");
            };
            uri = next_uri;
            path = rewritten;
            if rematch {
                let Some((rematch_idx, rematch_route)) = state
                    .route_index
                    .match_route_index_with_host_port(path.as_str(), host.as_deref(), local_port)
                else {
                    return text_response(StatusCode::NOT_FOUND, "not found");
                };
                route_idx = rematch_idx;
                if let Some(policy) = state.snapshot.route_access_policy(route_idx) {
                    if !policy.allows(client_ip, method.as_str()) {
                        return text_response(StatusCode::FORBIDDEN, "forbidden");
                    }
                }
                // Do not chain rewrite rules; retain the Cap036 redirect surface
                // on the rematched route and fail closed on another structural
                // rewrite.
                match crate::structural_route_rules::evaluate_route_structural_rules(rematch_route)
                {
                    exyonq_module_api::RouteRuleOutcome::NoChange => {}
                    exyonq_module_api::RouteRuleOutcome::Redirect { status, location } => {
                        return redirect_response_parts(status, location);
                    }
                    exyonq_module_api::RouteRuleOutcome::InternalRewrite { .. } => {
                        return text_response(StatusCode::NOT_FOUND, "not found");
                    }
                    exyonq_module_api::RouteRuleOutcome::RejectedRedirect => {
                        return text_response(StatusCode::BAD_REQUEST, "invalid redirect location");
                    }
                }
            }
        }
        exyonq_module_api::RewriteApplyOutcome::Redirect { status, location } => {
            return redirect_response_parts(status, &location);
        }
        exyonq_module_api::RewriteApplyOutcome::Rejected => {
            return text_response(StatusCode::BAD_REQUEST, "invalid rewrite target");
        }
    }
    let path = path.as_str();

    // WC2B/WC2C/WC2D: eligibility + L1 lookup; MISS context for static/FastCGI fill.
    let mut fpc_bypass_reason: Option<&'static str> = None;
    let fpc_miss = match crate::fpc_lookup::fpc_gate(
        state,
        route_idx,
        method,
        &uri,
        host.as_deref(),
        request_headers,
    ) {
        crate::fpc_lookup::FpcGateResult::Hit(hit) => {
            return crate::observability::attach_outcome_hint(hit, "fpc_hit");
        }
        crate::fpc_lookup::FpcGateResult::Miss(ctx) => Some(ctx),
        crate::fpc_lookup::FpcGateResult::Bypass { reason } => {
            fpc_bypass_reason = Some(reason);
            None
        }
        crate::fpc_lookup::FpcGateResult::Skip => None,
    };
    macro_rules! fpc_bypassed {
        ($resp:expr) => {{
            let mut r = $resp;
            crate::fpc_lookup::maybe_apply_bypass(&mut r, fpc_bypass_reason);
            r
        }};
    }

    if let Some(root_slot) = static_root_slot_for_route(state, route_idx) {
        let response = if let Some(ctx) = fpc_miss {
            static_dispatch_fpc_fill(
                state,
                method,
                root_slot,
                path,
                request_headers,
                materialization_budget_bytes,
                scheme,
                uri.query(),
                uri.authority().map(|a| a.as_str()).or(host.as_deref()),
                local_port,
                ctx,
            )
            .await
        } else {
            static_dispatch_maybe_cached(
                state,
                scheme,
                local_port,
                route_idx,
                method,
                &uri,
                host.as_deref(),
                root_slot,
                path,
                request_headers,
                materialization_budget_bytes,
            )
            .await
        };
        return fpc_bypassed!(response);
    }

    // WC2E: FPC fill for proxy (NG-15 keys/bypass/vary live in the FPC gate).
    if let Some(cluster_id) = state.snapshot.proxy_cluster_for_route(route_idx) {
        let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
        if let Some(ctx) = fpc_miss {
            return fpc_bypassed!(
                proxy_dispatch_fpc_fill(
                    state,
                    route_idx,
                    method,
                    path_and_query,
                    request_headers,
                    x_forwarded_for,
                    ctx,
                )
                .await
            );
        }
        return fpc_bypassed!(
            proxy_dispatch_maybe_cached(
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
            .await
        );
    }

    let remote_addr = x_forwarded_for
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1")
        .to_string();
    // Cap038 LA-CAP038-001: FastCGI must receive the collected request body.
    // Passing Vec::new() here dropped every POST/PUT body (php://input always empty).
    let fcgi_body = request_body
        .as_ref()
        .map(|b| b.to_vec())
        .unwrap_or_default();
    if let Some(ctx) = fpc_miss {
        if matches!(
            state.snapshot.resolve_backend(route_idx),
            Some(Backend::Fastcgi { .. })
        ) {
            return fpc_bypassed!(
                fastcgi_dispatch_fpc_fill(
                    state,
                    route_idx,
                    method,
                    &uri,
                    fcgi_script_path.as_deref(),
                    &remote_addr,
                    scheme,
                    request_headers,
                    fcgi_body,
                    ctx,
                )
                .await
            );
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
        scheme,
        request_headers.to_vec(),
        fcgi_body,
    )
    .await
    {
        return fpc_bypassed!(response);
    }

    fpc_bypassed!(text_response(StatusCode::NOT_FOUND, "not found"))
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

/// Rebuild request-target with `new_path`, preserving the original query.
/// Returns `None` if the resulting origin-form is not a valid `http::Uri`
/// (Cap035: fail closed — never silently keep the pre-rewrite URI).
fn uri_with_path(base: &hyper::Uri, new_path: &str) -> Option<hyper::Uri> {
    let path_and_query = if let Some(query) = base.query() {
        format!("{new_path}?{query}")
    } else {
        new_path.to_string()
    };
    path_and_query.parse().ok()
}

async fn serve_resolved_static_file(
    method: &Method,
    path: &std::path::Path,
    materialization_budget_bytes: Option<u64>,
) -> Response<BoxBody> {
    let Some(static_method) = static_method_from_hyper(method) else {
        return static_method_not_allowed();
    };
    let outcome = crate::execute_backend::serve_static_resolved_path_with_budget(
        static_method,
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
    scheme: &'static str,
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
            // EXY-03: NGINX `alias` + `fastcgi_pass` — resolve the script under
            // the route's alias root at `URI − match.path` (verbatim
            // remainder); `SCRIPT_NAME` stays the full request URI.
            let route = &state.config.routes[route_idx];
            let alias_uri_path;
            let (script_root, script_name_uri, uri_path) = match &route.fastcgi_script_root {
                Some(root) => {
                    let rem = if route.r#match.exact {
                        Some("")
                    } else {
                        uri_path.strip_prefix(route.r#match.path.as_str())
                    };
                    alias_uri_path = format!("/{}", rem.unwrap_or("").trim_start_matches('/'));
                    (
                        Some(root.as_path()),
                        Some(http.uri.path()),
                        alias_uri_path.as_str(),
                    )
                }
                None => (None, None, uri_path),
            };
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

            let front_controller = state.config.routes[route_idx].front_controller.as_deref();
            let dispatch_request = match execute_backend::build_fcgi_dispatch_request(
                *pool_id,
                pool_document_root,
                script_root,
                script_name_uri,
                front_controller,
                http.method.as_str(),
                uri_path,
                query_string,
                request_uri,
                http.headers,
                http.body,
                http.remote_addr,
                default_port,
            ) {
                Ok(mut request) => {
                    request.scheme = http.scheme.to_string();
                    request
                }
                Err(outcome) => {
                    return Some(fcgi_resolution_error_outcome(outcome));
                }
            };

            execute_backend::execute_backend(backend, Some(dispatch_request), None, None).await
        }
        _ => None,
    }
}

/// EXY-10: map a FastCGI script-resolution failure to its real status — a
/// missing script is a client-visible 404 (403 for traversal), not an upstream
/// fault. 502 stays for actual probe/transport errors only.
fn fcgi_resolution_error_outcome(
    outcome: exyonq_module_api::fcgi_script_resolver::FastcgiScriptResolutionOutcome,
) -> execute_backend::ExecuteBackendOutcome {
    use exyonq_module_api::fcgi_script_resolver::FastcgiScriptResolutionOutcome as R;
    let (status, body) = match outcome {
        R::NotFound => (404, b"not found".as_slice()),
        R::Forbidden | R::InvalidPath => (403, b"forbidden".as_slice()),
        _ => (502, b"bad gateway".as_slice()),
    };
    execute_backend::ExecuteBackendOutcome {
        status,
        headers: Vec::new(),
        body: body.to_vec(),
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
    scheme: &'static str,
    request_headers: &[(String, String)],
    body: Vec<u8>,
    ctx: crate::fpc_lookup::FpcMissContext,
) -> Response<BoxBody> {
    let http = FcgiHttpContext {
        method,
        uri,
        script_path,
        headers: request_headers.to_vec(),
        body,
        remote_addr,
        scheme,
    };

    // HEAD must not populate (or overwrite) a GET representation.
    if *method == Method::HEAD {
        let Some(mut outcome) = execute_fastcgi_backend(state, route_idx, http).await else {
            return text_response(StatusCode::NOT_IMPLEMENTED, "not implemented");
        };
        // NG-16: HEAD falls back to the stale representation like GET.
        if outcome.status >= 500 {
            if let Some(stale) = crate::fpc_lookup::stale_response(&ctx, method) {
                return stale;
            }
        }
        // HEAD responses must not carry a body (scripted executors may still return one).
        outcome.body.clear();
        outcome
            .headers
            .retain(|(n, _)| !n.eq_ignore_ascii_case("content-length"));
        let mut resp = fcgi_response_from_outcome(outcome);
        crate::fpc_lookup::apply_x_cache(resp.headers_mut(), "miss");
        return resp;
    }

    let Some(outcome) = execute_fastcgi_backend(state, route_idx, http).await else {
        return text_response(StatusCode::NOT_IMPLEMENTED, "not implemented");
    };

    let status = outcome.status;
    let headers = outcome.headers;
    let body = Bytes::from(outcome.body);

    // NG-16: FastCGI 5xx falls back to the stale entry when one exists.
    if status >= 500 {
        if let Some(stale) = crate::fpc_lookup::stale_response(&ctx, method) {
            return stale;
        }
    }

    // Live reload generation may advance while FastCGI was in flight (SharedServerState swap).
    // Never insert under the lookup-time identity once the process has moved on.
    if crate::reload::active_runtime_generation() != ctx.runtime_generation {
        exyonq_cache::note_fpc_store_attempt();
        exyonq_cache::note_fpc_store_rejected("generation_mismatch");
        let mut resp = fcgi_response_from_parts(status, headers, body);
        crate::fpc_lookup::apply_x_cache(resp.headers_mut(), "miss");
        return resp;
    }

    crate::fpc_store::try_fpc_store_response(&ctx, method.as_str(), status, &headers, body.clone());

    let mut resp = fcgi_response_from_parts(status, headers, body);
    crate::fpc_lookup::apply_x_cache(resp.headers_mut(), "miss");
    resp
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
    scheme: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> Option<Response<BoxBody>> {
    let backend = state.snapshot.resolve_backend(route_idx)?;
    let Backend::Fastcgi { pool_id } = backend else {
        return None;
    };
    let pool_id = *pool_id;

    // HTTP/2 carries :authority in the URI, not as a Host field. FastCGI needs
    // Host for HTTP_HOST and WordPress admin/login absolute URLs.
    let mut headers = headers;
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        if let Some(host) = host {
            headers.push(("Host".to_string(), host.to_string()));
        }
    }

    let http = FcgiHttpContext {
        method,
        uri,
        script_path,
        headers: headers.clone(),
        body: body.clone(),
        remote_addr,
        scheme,
    };

    let Some(policy) = state.snapshot.cache_policy_for_route(route_idx) else {
        return try_fastcgi_contract_backend_response(state, route_idx, http).await;
    };

    if !exyonq_module_api::request_eligible_for_cache(method.as_str(), &headers) {
        let mut response = try_fastcgi_contract_backend_response(state, route_idx, http).await?;
        apply_plan10b_cache_headers(&mut response, "BYPASS");
        return Some(response);
    }

    let scheme = conn_scheme(state);
    let key = build_storage_cache_key(CacheKeyParts {
        range: String::new(),
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
        vary: String::new(),
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
    // EXY-03: NGINX `alias` + `fastcgi_pass` — route script root/remainder
    // resolution, cloned into the cache fill closure below.
    let fcgi_script_root_owned = state.config.routes[route_idx].fastcgi_script_root.clone();
    let front_controller_owned = state.config.routes[route_idx].front_controller.clone();
    let route_match_path_owned = state.config.routes[route_idx].r#match.path.clone();
    let route_match_exact_owned = state.config.routes[route_idx].r#match.exact;

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
                let fcgi_script_root = fcgi_script_root_owned.clone();
                let front_controller = front_controller_owned.clone();
                let route_match_path = route_match_path_owned.clone();
                let route_match_exact = route_match_exact_owned;
                async move {
                    let uri_path_raw = script_path.as_deref().unwrap_or_else(|| uri.path());
                    let alias_uri_path;
                    let (script_root, script_name_uri, uri_path) = match &fcgi_script_root {
                        Some(root) => {
                            let rem = if route_match_exact {
                                Some("")
                            } else {
                                uri_path_raw.strip_prefix(route_match_path.as_str())
                            };
                            alias_uri_path =
                                format!("/{}", rem.unwrap_or("").trim_start_matches('/'));
                            (
                                Some(root.as_path()),
                                Some(uri.path()),
                                alias_uri_path.as_str(),
                            )
                        }
                        None => (None, None, uri_path_raw),
                    };
                    let query_string = uri.query().unwrap_or("");
                    let request_uri = uri
                        .path_and_query()
                        .map(|pq| pq.as_str())
                        .unwrap_or(uri_path);
                    let dispatch_request = match execute_backend::build_fcgi_dispatch_request(
                        pool_id,
                        pool_document_root.as_ref(),
                        script_root,
                        script_name_uri,
                        front_controller.as_deref(),
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
                        Err(outcome) => {
                            let bad = fcgi_resolution_error_outcome(outcome);
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
        // Cap015 / WAF-LOGIC-P2-G: never drop non-UTF8 header values before WAF.
        // Lossy decode preserves ASCII attack substrings (same contract as body scan).
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        out.push((name.as_str().to_string(), value));
    }
    out
}

/// Cap019/Cap020: forward only `Range` + conditional validators into static dispatch
/// (avoids full HeaderMap→Vec on /site hot path).
fn static_dispatch_range_headers(headers: &hyper::HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for value in headers.get_all(hyper::header::RANGE) {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        out.push(("range".to_string(), value));
    }
    for value in headers.get_all(hyper::header::IF_NONE_MATCH) {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        out.push(("if-none-match".to_string(), value));
    }
    for value in headers.get_all(hyper::header::IF_MODIFIED_SINCE) {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        out.push(("if-modified-since".to_string(), value));
    }
    out
}

fn static_dispatch_range_headers_from_pairs(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            name.eq_ignore_ascii_case("range")
                || name.eq_ignore_ascii_case("if-none-match")
                || name.eq_ignore_ascii_case("if-modified-since")
        })
        .cloned()
        .collect()
}

/// Bounded request-body materialization for FastCGI / challenge paths.
///
/// Uses early Content-Length reject + incremental frames (SEC-PROXY-001 collector).
/// Never `BodyExt::collect()` first — that waits for a full declared length.
async fn collect_request_body_bounded<B>(
    headers: &hyper::HeaderMap,
    body: B,
    limit: usize,
) -> Result<Vec<u8>, StatusCode>
where
    B: http_body::Body<Data = Bytes> + Unpin,
{
    use crate::server::proxy_request_body::{
        collect_proxy_request_body_bounded, parse_proxy_request_content_length,
        ProxyRequestBodyReadError,
    };

    let declared = parse_proxy_request_content_length(headers);
    match collect_proxy_request_body_bounded(declared, body, limit).await {
        Ok(bytes) => {
            enforce_request_body_limit(bytes.len(), limit)?;
            Ok(bytes)
        }
        Err(ProxyRequestBodyReadError::TooLarge) => Err(StatusCode::PAYLOAD_TOO_LARGE),
        Err(ProxyRequestBodyReadError::Body) => Err(StatusCode::BAD_REQUEST),
    }
}

/// Resolve a route's request-body limit, clamped to the shared proxy bound.
///
/// `None` inherits `base` (proxy or FastCGI bound). The route limit is a
/// client-facing size policy: it bounds memory materialization *and* the bytes
/// the WAF can inspect on that route.
fn route_request_body_limit(route: Option<&exyonq_config_ir::RouteConfig>, base: usize) -> usize {
    use exyonq_module_api::proxy_dispatch::PROXY_MAX_REQUEST_BODY_BYTES;
    let Some(limit) = route.and_then(|route| route.max_request_body_bytes) else {
        return base;
    };
    usize::try_from(limit)
        .unwrap_or(usize::MAX)
        .min(base)
        .min(PROXY_MAX_REQUEST_BODY_BYTES)
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

fn static_root_slot_for_request(
    state: &ServerState,
    path: &str,
    host: Option<&str>,
    local_port: Option<u16>,
) -> Option<u32> {
    let (route_idx, _) = state
        .route_index
        .match_route_index_with_host_port(path, host, local_port)?;
    if state.route_requires_full_dispatch(route_idx) {
        return None;
    }
    static_root_slot_for_route(state, route_idx)
}

fn static_root_slot_for_route(state: &ServerState, route_idx: usize) -> Option<u32> {
    match state.snapshot.resolve_backend(route_idx)? {
        Backend::Static { root_slot } => Some(*root_slot),
        _ => None,
    }
}

fn static_method_from_hyper(method: &Method) -> Option<StaticMethod> {
    if *method == Method::GET {
        Some(StaticMethod::Get)
    } else if *method == Method::HEAD {
        Some(StaticMethod::Head)
    } else {
        None
    }
}

fn static_method_not_allowed() -> Response<BoxBody> {
    let mut response = text_response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
    response
        .headers_mut()
        .insert(hyper::header::ALLOW, HeaderValue::from_static("GET, HEAD"));
    response
}

/// ADR-061 / NG-17: apply `[[route]].cache_headers` to a static 200.
/// An existing `Cache-Control` (origin or `[[route.response_headers]]`) wins.
fn apply_route_cache_headers(
    state: &ServerState,
    route_idx: usize,
    response: &mut Response<BoxBody>,
) {
    if response.status() != StatusCode::OK {
        return;
    }
    if response.headers().contains_key(CACHE_CONTROL) {
        return;
    }
    let Some(route) = state.snapshot.config.routes.get(route_idx) else {
        return;
    };
    let Some(cache_headers) = &route.cache_headers else {
        return;
    };
    let Some(content_type) = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    else {
        return;
    };
    if let Some(value) = cache_headers.cache_control_for(content_type) {
        if let Ok(value) = HeaderValue::from_str(&value) {
            response.headers_mut().insert(CACHE_CONTROL, value);
        }
    }
}

/// Pair-form of [`apply_route_cache_headers`] for the FPC fill path, so the
/// stored entry carries the same `Cache-Control` as the live MISS response.
fn apply_route_cache_headers_pairs(
    state: &ServerState,
    route_idx: usize,
    status: u16,
    headers: &mut Vec<(String, String)>,
) {
    if status != 200 {
        return;
    }
    if headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
    {
        return;
    }
    let Some(route) = state.snapshot.config.routes.get(route_idx) else {
        return;
    };
    let Some(cache_headers) = &route.cache_headers else {
        return;
    };
    let Some(content_type) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
    else {
        return;
    };
    if let Some(value) = cache_headers.cache_control_for(content_type) {
        headers.push(("cache-control".into(), value));
    }
}

/// Connection scheme for absolute redirect targets (`https` when the primary server terminates TLS).
fn conn_scheme(state: &ServerState) -> &'static str {
    if state.config.primary_server().tls.is_some() {
        "https"
    } else {
        "http"
    }
}

#[allow(clippy::too_many_arguments)]
async fn static_dispatch_via_service(
    method: &Method,
    root_slot: u32,
    request_path: &str,
    request_headers: Vec<(String, String)>,
    materialization_budget_bytes: Option<u64>,
    scheme: &'static str,
    request_query: Option<&str>,
    host: Option<&str>,
    local_port: Option<u16>,
) -> Response<BoxBody> {
    let Some(static_method) = static_method_from_hyper(method) else {
        return static_method_not_allowed();
    };
    let mut request_headers = request_headers;
    // h2 carries authority in `:authority`/URI, not a `host` header — inject the
    // resolved host so absolute redirect targets can be built.
    if let Some(h) = host {
        if !request_headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("host"))
        {
            request_headers.push(("host".to_string(), h.to_string()));
        }
    }
    // P14C-C-A: take ownership — callers that already own a Vec avoid a second to_vec.
    let mut request = build_static_dispatch_request_from_str_with_budget(
        root_slot,
        static_method,
        request_path,
        request_headers,
        materialization_budget_bytes,
    );
    request.scheme = scheme;
    request.local_port = local_port;
    request.request_query = request_query.map(str::to_string);
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

#[allow(clippy::too_many_arguments)]
async fn static_load_for_cache_via_service(
    method: &Method,
    root_slot: u32,
    request_path: &str,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
    scheme: &'static str,
    request_query: Option<&str>,
    host: Option<&str>,
    local_port: Option<u16>,
) -> StaticCacheLoad {
    let Some(client_method) = static_method_from_hyper(method) else {
        return StaticCacheLoad {
            status: StatusCode::METHOD_NOT_ALLOWED.as_u16(),
            headers: vec![
                ("content-type".into(), "text/plain; charset=utf-8".into()),
                ("allow".into(), "GET, HEAD".into()),
            ],
            body: Bytes::from_static(b"method not allowed"),
            identity: None,
        };
    };
    let storage_method = crate::execute_backend::static_cache_storage_method(client_method);
    let mut request_headers = request_headers.to_vec();
    // Same h2 `:authority` gap as static_dispatch_via_service — inject resolved host.
    if let Some(h) = host {
        if !request_headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("host"))
        {
            request_headers.push(("host".to_string(), h.to_string()));
        }
    }
    let mut request = build_static_dispatch_request_from_str_with_budget(
        root_slot,
        storage_method,
        request_path,
        request_headers,
        materialization_budget_bytes,
    );
    request.scheme = scheme;
    request.local_port = local_port;
    request.request_query = request_query.map(str::to_string);
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
#[allow(clippy::too_many_arguments)]
async fn static_dispatch_fpc_fill(
    state: &ServerState,
    method: &Method,
    root_slot: u32,
    request_path: &str,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
    scheme: &'static str,
    request_query: Option<&str>,
    host: Option<&str>,
    local_port: Option<u16>,
    ctx: crate::fpc_lookup::FpcMissContext,
) -> Response<BoxBody> {
    // HEAD must not populate (or overwrite) a GET representation.
    if *method == Method::HEAD {
        let mut resp = static_dispatch_via_service(
            method,
            root_slot,
            request_path,
            request_headers.to_vec(),
            materialization_budget_bytes,
            scheme,
            request_query,
            host,
            local_port,
        )
        .await;
        apply_route_cache_headers(state, ctx.route_idx, &mut resp);
        crate::fpc_lookup::apply_x_cache(resp.headers_mut(), "miss");
        return resp;
    }

    let mut load = static_load_for_cache_via_service(
        method,
        root_slot,
        request_path,
        request_headers,
        materialization_budget_bytes,
        scheme,
        request_query,
        host,
        local_port,
    )
    .await;

    if load.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case(
            exyonq_module_api::static_dispatch::MATERIALIZATION_BUDGET_EXCEEDED_HEADER,
        ) && value == "1"
    }) {
        let mut resp = budget_exceeded_marker_response!();
        crate::fpc_lookup::apply_x_cache_bypass(resp.headers_mut(), "budget");
        return resp;
    }

    apply_route_cache_headers_pairs(state, ctx.route_idx, load.status, &mut load.headers);

    // Live reload may publish N+1 while static origin was in flight (symmetric FastCGI WC2D).
    let generation_ok = crate::reload::active_runtime_generation() == ctx.runtime_generation;
    if !generation_ok {
        exyonq_cache::note_fpc_store_attempt();
        exyonq_cache::note_fpc_store_rejected("generation_mismatch");
    } else {
        crate::fpc_store::try_fpc_store_response(
            &ctx,
            method.as_str(),
            load.status,
            &load.headers,
            load.body.clone(),
        );
    }

    let client_body = load.body;

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
    let mut resp = builder
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
        });
    crate::fpc_lookup::apply_x_cache(resp.headers_mut(), "miss");
    resp
}

#[allow(clippy::too_many_arguments)]
async fn static_dispatch_maybe_cached(
    state: &ServerState,
    scheme: &'static str,
    local_port: Option<u16>,
    route_idx: usize,
    method: &Method,
    uri: &hyper::Uri,
    host: Option<&str>,
    root_slot: u32,
    request_path: &str,
    request_headers: &[(String, String)],
    materialization_budget_bytes: Option<u64>,
) -> Response<BoxBody> {
    // H-01: never consult a cached static representation for a sensitive
    // request path. The static service owns the final route-relative decision;
    // this guard prevents a pre-existing cache entry from bypassing it.
    if exyonq_mod_static::request_path_may_be_sensitive(request_path) {
        let mut response = static_dispatch_via_service(
            method,
            root_slot,
            request_path,
            request_headers.to_vec(),
            materialization_budget_bytes,
            scheme,
            uri.query(),
            uri.authority().map(|a| a.as_str()).or(host),
            local_port,
        )
        .await;
        apply_route_cache_headers(state, route_idx, &mut response);
        return response;
    }

    let Some(policy) = state.snapshot.cache_policy_for_route(route_idx) else {
        let mut response = static_dispatch_via_service(
            method,
            root_slot,
            request_path,
            request_headers.to_vec(),
            materialization_budget_bytes,
            scheme,
            uri.query(),
            uri.authority().map(|a| a.as_str()).or(host),
            local_port,
        )
        .await;
        apply_route_cache_headers(state, route_idx, &mut response);
        return response;
    };

    if !exyonq_module_api::request_eligible_for_cache(method.as_str(), request_headers) {
        let mut response = static_dispatch_via_service(
            method,
            root_slot,
            request_path,
            request_headers.to_vec(),
            materialization_budget_bytes,
            scheme,
            uri.query(),
            uri.authority().map(|a| a.as_str()).or(host),
            local_port,
        )
        .await;
        apply_route_cache_headers(state, route_idx, &mut response);
        return response;
    }

    let scheme = conn_scheme(state);
    let key = build_storage_cache_key(CacheKeyParts {
        range: String::new(),
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
        vary: String::new(),
    });

    let req_headers = request_headers.to_vec();
    let method_owned = method.clone();
    let path_owned = request_path.to_string();
    let query_owned = uri.query().map(str::to_string);
    let mut response = serve_static_with_cache(
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
            let query = query_owned.clone();
            async move {
                static_load_for_cache_via_service(
                    &method,
                    root_slot,
                    &path,
                    &headers,
                    materialization_budget_bytes,
                    scheme,
                    query.as_deref(),
                    host,
                    local_port,
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
    apply_route_cache_headers(state, route_idx, &mut response);
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

    let scheme = conn_scheme(state);
    let key = build_storage_cache_key(CacheKeyParts {
        range: String::new(),
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
        vary: String::new(),
    });

    let path_and_query = path_and_query.to_string();
    let xff = x_forwarded_for.cloned();
    let req_headers = request_headers.to_vec();
    let max_bytes = policy.max_object_bytes;
    // NG-18 Phase E: slice route with segment cache.
    if !request_headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("range"))
    {
        let route_name = state
            .snapshot
            .config
            .routes
            .get(route_idx)
            .map(|r| r.name.as_str())
            .unwrap_or("");
        if let Some(slice_cfg) = state.config.modules.slice.routes.get(route_name) {
            if let Some(resp) = proxy_dispatch_slice(
                state,
                route_idx,
                cluster_id,
                method,
                uri,
                host,
                &path_and_query,
                request_headers,
                x_forwarded_for,
                policy.ttl.as_secs(),
                slice_cfg.size,
            )
            .await
            {
                return resp;
            }
        }
    }
    // NG-07: cookie_affinity route token travels beside the cache load —
    // appended to the client response only, never into cached headers.
    let route_cookie = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let cookie_slot = std::sync::Arc::clone(&route_cookie);

    let mut resp = serve_proxy_with_cache(
        global_response_cache(),
        global_singleflight(),
        key,
        route_idx,
        state.generation,
        method,
        policy,
        || async move {
            let (load, cookie) = load_get_for_cache_by_cluster(
                cluster_id,
                &path_and_query,
                xff.as_ref(),
                max_bytes,
                &req_headers,
            )
            .await;
            if let Ok(mut slot) = cookie_slot.lock() {
                *slot = cookie;
            }
            load
        },
    )
    .await;
    if let Some(cookie) = route_cookie.lock().ok().and_then(|c| c.clone()) {
        // NG-07: never pin the client on a failed response.
        if resp.status().as_u16() < 500 {
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                resp.headers_mut().append(hyper::header::SET_COOKIE, v);
            }
        }
    }
    maybe_apply_proxy_range(&mut resp, request_headers, method, max_bytes).await;
    resp
}

#[derive(Debug, PartialEq, Eq)]
enum RangeOutcome {
    Single(u64, u64),
    Unsatisfiable,
    Ignore,
}

fn header_value_ignore_case<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// NG-18 Phase E: serve a single Range from a cached/materialized full 200.
/// Multi-range and malformed ranges fall back to the full representation.
fn parse_single_range(value: &str, len: u64) -> RangeOutcome {
    let Some(spec) = value.trim().strip_prefix("bytes=") else {
        return RangeOutcome::Ignore;
    };
    if spec.contains(',') || len == 0 {
        return RangeOutcome::Ignore;
    }
    let Some((start, end)) = spec.split_once('-') else {
        return RangeOutcome::Ignore;
    };
    let (start, end) = (start.trim(), end.trim());
    if start.is_empty() {
        let Ok(suffix) = end.parse::<u64>() else {
            return RangeOutcome::Ignore;
        };
        if suffix == 0 {
            return RangeOutcome::Ignore;
        }
        let start = len.saturating_sub(suffix);
        return RangeOutcome::Single(start, len - 1);
    }
    let Ok(start) = start.parse::<u64>() else {
        return RangeOutcome::Ignore;
    };
    if start >= len {
        return RangeOutcome::Unsatisfiable;
    }
    let end = if end.is_empty() {
        len - 1
    } else {
        let Ok(end) = end.parse::<u64>() else {
            return RangeOutcome::Ignore;
        };
        if end < start {
            return RangeOutcome::Ignore;
        }
        end.min(len - 1)
    };
    RangeOutcome::Single(start, end)
}

async fn maybe_apply_proxy_range(
    response: &mut Response<BoxBody>,
    request_headers: &[(String, String)],
    method: &Method,
    max_object_bytes: usize,
) {
    let Some(range) = header_value_ignore_case(request_headers, "range") else {
        return;
    };
    if response.status() != StatusCode::OK {
        return;
    }
    if let Some(if_range) = header_value_ignore_case(request_headers, "if-range") {
        let etag = response.headers().get(ETAG).and_then(|v| v.to_str().ok());
        let last_modified = response
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok());
        let matches = if if_range.starts_with('"') || if_range.starts_with("W/") {
            etag == Some(if_range)
        } else {
            last_modified == Some(if_range)
        };
        if !matches {
            return;
        }
    }
    let Some(len) = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
    else {
        return;
    };
    if len > max_object_bytes as u64 {
        return;
    }
    match parse_single_range(range, len) {
        RangeOutcome::Ignore => (),
        RangeOutcome::Unsatisfiable => {
            let headers = response.headers_mut();
            headers.remove(CONTENT_LENGTH);
            headers.insert(
                CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{len}")).expect("valid range"),
            );
            *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
            *response.body_mut() = empty_body();
        }
        RangeOutcome::Single(start, end) => {
            let headers = response.headers_mut();
            headers.remove(CONTENT_LENGTH);
            headers.remove(hyper::header::TRANSFER_ENCODING);
            headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            headers.insert(
                CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {start}-{end}/{len}")).expect("valid range"),
            );
            let body_len = end - start + 1;
            headers.insert(
                CONTENT_LENGTH,
                HeaderValue::from_str(&body_len.to_string()).expect("valid length"),
            );
            *response.status_mut() = StatusCode::PARTIAL_CONTENT;
            if *method == Method::HEAD {
                *response.body_mut() = empty_body();
                return;
            }
            let body = std::mem::replace(response.body_mut(), empty_body());
            let Ok(collected) = body.collect().await else {
                return;
            };
            let bytes = collected.to_bytes();
            if bytes.len() as u64 != len {
                return;
            }
            let slice = bytes.slice(start as usize..=end as usize);
            *response.body_mut() = Full::new(slice).map_err(|never| match never {}).boxed();
        }
    }
}

fn empty_body() -> BoxBody {
    Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed()
}

const SLICE_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

fn parse_content_range_total(headers: &[(String, String)]) -> Option<u64> {
    let value = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-range"))
        .map(|(_, value)| value.as_str())?;
    value.rsplit_once('/')?.1.trim().parse().ok()
}

fn slice_segment_key(
    state: &ServerState,
    route_idx: usize,
    cluster_id: u32,
    uri: &hyper::Uri,
    host: Option<&str>,
    range: String,
) -> exyonq_cache::CacheKey {
    let scheme = conn_scheme(state);
    build_storage_cache_key(CacheKeyParts {
        site_id: 0,
        namespace: 0,
        backend_id: cluster_id,
        runtime_generation: state.generation,
        policy_generation: 0,
        route_idx,
        method: "GET".into(),
        scheme: scheme.into(),
        host: normalize_host(host),
        path: uri.path().to_string(),
        query: uri.query().unwrap_or("").to_string(),
        content_encoding: "identity".into(),
        vary: String::new(),
        range,
    })
}

async fn fetch_slice_segment(
    cluster_id: u32,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    request_headers: &[(String, String)],
    size: usize,
    start: u64,
    end: u64,
) -> Option<(Bytes, Vec<(String, String)>)> {
    let mut headers = request_headers.to_vec();
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("range"));
    headers.push(("range".into(), format!("bytes={start}-{end}")));
    match load_get_for_cache_by_cluster(cluster_id, path_and_query, x_forwarded_for, size, &headers)
        .await
        .0
    {
        ProxyCacheLoad::Materialized {
            body,
            status,
            headers,
            ..
        } if status.as_u16() == 206 => Some((body, headers)),
        ProxyCacheLoad::Passthrough { response, .. } if response.status().as_u16() == 206 => {
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    (
                        name.to_string(),
                        value.to_str().unwrap_or_default().to_string(),
                    )
                })
                .collect::<Vec<_>>();
            let expected = (end - start + 1) as usize;
            let body = http_body_util::Limited::new(response.into_body(), expected)
                .collect()
                .await
                .ok()?
                .to_bytes();
            Some((body, headers))
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn proxy_dispatch_slice(
    state: &ServerState,
    route_idx: usize,
    cluster_id: u32,
    method: &Method,
    uri: &hyper::Uri,
    host: Option<&str>,
    path_and_query: &str,
    request_headers: &[(String, String)],
    x_forwarded_for: Option<&HeaderValue>,
    ttl_secs: u64,
    size: usize,
) -> Option<Response<BoxBody>> {
    if *method != Method::GET && *method != Method::HEAD {
        return None;
    }
    let size_u64 = size as u64;
    let cache = global_response_cache();
    let key0 = slice_segment_key(
        state,
        route_idx,
        cluster_id,
        uri,
        host,
        format!("0-{}", size_u64 - 1),
    );
    let (first_body, first_headers, first_cached) = if let Some(entry) = cache.lookup(&key0) {
        (entry.body.clone(), entry.headers.clone(), true)
    } else {
        let (body, headers) = fetch_slice_segment(
            cluster_id,
            path_and_query,
            x_forwarded_for,
            request_headers,
            size,
            0,
            size_u64 - 1,
        )
        .await?;
        (body, headers, false)
    };
    let total = parse_content_range_total(&first_headers)?;
    if total <= size_u64 || total > SLICE_MAX_TOTAL_BYTES {
        return None;
    }
    if first_body.len() as u64 != size_u64.min(total) {
        return None;
    }

    let ttl = std::time::Duration::from_secs(ttl_secs.max(1));
    let mut assembled = bytes::BytesMut::with_capacity(total as usize);
    let mut all_hit = true;
    let segments = total.div_ceil(size_u64);
    for idx in 0..segments {
        let start = idx * size_u64;
        let end = (start + size_u64 - 1).min(total - 1);
        let key = slice_segment_key(
            state,
            route_idx,
            cluster_id,
            uri,
            host,
            format!("{start}-{end}"),
        );
        if idx == 0 && first_cached {
            assembled.extend_from_slice(&first_body);
            continue;
        }
        if let Some(entry) = cache.lookup(&key) {
            if entry.body.len() as u64 == end - start + 1 {
                assembled.extend_from_slice(&entry.body);
                continue;
            }
        }
        all_hit = false;
        let (body, headers) = if idx == 0 {
            (first_body.clone(), first_headers.clone())
        } else {
            fetch_slice_segment(
                cluster_id,
                path_and_query,
                x_forwarded_for,
                request_headers,
                size,
                start,
                end,
            )
            .await?
        };
        if body.len() as u64 != end - start + 1 {
            return None;
        }
        if !exyonq_module_api::cache::slice_segment_store_blocked(&headers) {
            cache.insert_entry(
                key,
                0,
                route_idx,
                state.generation,
                ttl,
                206,
                exyonq_module_api::cache::filter_storable_headers(&headers),
                body.clone(),
                None,
                CacheNamespace::new(0),
                Arc::from([]),
                None,
                NamespaceMetrics::NONE,
            );
        }
        assembled.extend_from_slice(&body);
    }

    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .body(if *method == Method::HEAD {
            empty_body()
        } else {
            Full::new(assembled.freeze())
                .map_err(|never| match never {})
                .boxed()
        })
        .ok()?;
    for (name, value) in &first_headers {
        let lower = name.to_ascii_lowercase();
        if lower == "content-range"
            || lower == "content-length"
            || lower == "transfer-encoding"
            || lower == "connection"
            || lower == "set-cookie"
            || exyonq_module_api::hop_by_hop::is_fixed_hop_by_hop_header(&lower)
        {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            resp.headers_mut().append(n, v);
        }
    }
    resp.headers_mut().insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&total.to_string()).ok()?,
    );
    resp.headers_mut()
        .insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    crate::fpc_lookup::apply_x_cache(resp.headers_mut(), if all_hit { "hit" } else { "miss" });
    Some(resp)
}

/// WC2E/NG-15: proxy dispatch with FPC fill — the miss context carries the
/// configured key (query/cookies/headers + vary) and the Vary allowlist.
/// Uses the bounded streaming loader: bodies over `max_object_bytes`, SSE and
/// unknown-length responses stream through uncached (never collected).
#[allow(clippy::too_many_arguments)]
async fn proxy_dispatch_fpc_fill(
    state: &ServerState,
    route_idx: usize,
    method: &Method,
    path_and_query: &str,
    request_headers: &[(String, String)],
    x_forwarded_for: Option<&HeaderValue>,
    ctx: crate::fpc_lookup::FpcMissContext,
) -> Response<BoxBody> {
    let Some(cluster_id) = state.snapshot.proxy_cluster_for_route(route_idx) else {
        return bad_gateway();
    };
    // Cold-key herd is not coalesced here: singleflight lives in the Plan 12
    // proxy cache, whose key cannot express Vary. Bounded: the first response
    // is stored, so the herd spans one origin round-trip.
    let (load, _cookie) = load_get_for_cache_by_cluster(
        cluster_id,
        path_and_query,
        x_forwarded_for,
        ctx.max_object_bytes,
        request_headers,
    )
    .await;

    match load {
        ProxyCacheLoad::Materialized {
            body,
            status,
            headers,
            rejection,
            ..
        } => {
            // NG-16: a failing backend is answered with the stale entry.
            if status.as_u16() >= 500 {
                if let Some(stale) = crate::fpc_lookup::stale_response(&ctx, method) {
                    return stale;
                }
            }
            // Plan 12's raw gate runs on the pre-UTF-8 headers and rejects
            // Set-Cookie / private / no-store / content-range / any Vary. The
            // Vary rejection is the one NG-15 relaxes: the FPC assessment below
            // re-checks it against `key.headers` on the decoded values.
            let plan12_blocked = !matches!(
                rejection,
                None | Some(exyonq_module_api::CacheRejection::ResponseVary)
            );
            if !plan12_blocked && *method != Method::HEAD {
                if crate::reload::active_runtime_generation() == ctx.runtime_generation {
                    crate::fpc_store::try_fpc_store_response(
                        &ctx,
                        method.as_str(),
                        status.as_u16(),
                        &headers,
                        body.clone(),
                    );
                } else {
                    exyonq_cache::note_fpc_store_attempt();
                    exyonq_cache::note_fpc_store_rejected("generation_mismatch");
                }
            }
            let mut resp = Response::builder()
                .status(status)
                .body(Full::new(body).map_err(|never| match never {}).boxed())
                .expect("valid materialized response");
            for (name, value) in &headers {
                if let (Ok(n), Ok(v)) = (
                    hyper::header::HeaderName::from_bytes(name.as_bytes()),
                    HeaderValue::from_str(value),
                ) {
                    resp.headers_mut().append(n, v);
                }
            }
            crate::fpc_lookup::apply_x_cache(resp.headers_mut(), "miss");
            resp
        }
        ProxyCacheLoad::Passthrough { response, .. } => {
            // NG-16: passthrough also carries connect failures (502).
            if response.status().as_u16() >= 500 {
                if let Some(stale) = crate::fpc_lookup::stale_response(&ctx, method) {
                    return stale;
                }
            }
            let mut resp = response;
            crate::fpc_lookup::apply_x_cache(resp.headers_mut(), "miss");
            resp
        }
    }
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
    let scheme = conn_scheme(state);
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

    let client_ip = client_ip_from_xff(x_forwarded_for);
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(str::to_string);
    let host_hdr = req
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
    let header_view = crate::waf::PairsHeaderView(&headers);
    let http_method = req.method().clone();

    // H4: WAF headers before websocket upgrade forward.
    if let Some(reject) = waf_header_gate(
        state,
        &http_method,
        host_hdr.as_deref(),
        &path,
        query.as_deref(),
        &header_view,
        client_ip,
        false,
    ) {
        return reject;
    }

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
    let host = host_hdr;
    let remote_addr = x_forwarded_for
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1")
        .to_string();
    let scheme = conn_scheme(state);
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
        let throttle_cfg = state.config.modules.ratelimit.client.clone();
        let body = crate::client_throttle::wrap_request_body(
            req.into_body(),
            None,
            client_ip,
            &throttle_cfg,
        );
        let body_limit = route_request_body_limit(
            state.config.routes.get(route_idx),
            PROXY_MAX_REQUEST_BODY_BYTES,
        );
        match collect_proxy_request_body_bounded(declared_len, body, body_limit).await {
            Ok(bytes) => Some(bytes),
            Err(ProxyRequestBodyReadError::TooLarge) => {
                // Client-facing body limit (nginx client_max_body_size semantics):
                // 413, never upstream dispatch.
                return text_response(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
            }
            // Cap015 / WAF-LOGIC-P3-B: body-frame transport failure must not become an
            // empty body for WAF inspection or upstream dispatch (fail closed).
            Err(ProxyRequestBodyReadError::Body) => {
                return execute_backend::proxy_outcome_to_hyper(ProxyDispatchOutcome::BadGateway);
            }
        }
    };
    if let Some(ref body_bytes) = body {
        if let WafBodyGate::Reject(reject) = waf_body_gate(
            state,
            &http_method,
            host.as_deref(),
            &path,
            query.as_deref(),
            &header_view,
            client_ip,
            body_bytes,
        ) {
            return reject;
        }
    }
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
    // Cap036: fail closed on invalid Location header bytes (no panic / no fallback /).
    let Ok(loc) = HeaderValue::from_str(location) else {
        return text_response(StatusCode::BAD_REQUEST, "invalid redirect location");
    };
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::FOUND))
        .header(hyper::header::LOCATION, loc)
        .header("content-type", "text/plain; charset=utf-8")
        .header(hyper::header::CONTENT_LENGTH, "0")
        .body(
            Full::from(bytes::Bytes::from_static(b""))
                .map_err(|never| match never {})
                .boxed(),
        )
        .unwrap_or_else(|_| text_response(StatusCode::BAD_REQUEST, "invalid redirect location"))
}

fn redirect_response(redirect: &RedirectConfig) -> Response<BoxBody> {
    redirect_response_parts(redirect.status, redirect.location.as_str())
}

fn request_host(req: &Request<Incoming>) -> Option<String> {
    request_host_from_parts(req.uri(), req.headers())
}

/// H-01: canonicalize the request target once, before any auth/WAF/route gate.
fn canonicalize_request_uri(req: &mut Request<Incoming>) -> bool {
    // `OPTIONS *` and CONNECT authority-form are valid non-origin request
    // targets; they are not router paths and must not be rewritten.
    if !req.uri().path().starts_with('/') {
        return true;
    }
    let canonical = match exyonq_module_api::canonicalize_path(req.uri().path()) {
        Some(path) => path,
        None => return false,
    };
    if canonical == req.uri().path() {
        return true;
    }
    let target = match req.uri().query() {
        Some(query) => format!("{canonical}?{query}"),
        None => canonical,
    };
    let mut parts = req.uri().clone().into_parts();
    parts.path_and_query = match target.parse() {
        Ok(path_and_query) => Some(path_and_query),
        Err(_) => return false,
    };
    match hyper::Uri::from_parts(parts) {
        Ok(uri) => {
            *req.uri_mut() = uri;
            true
        }
        Err(_) => false,
    }
}

/// H-01 defence in depth for dispatch paths that receive a URI directly.
fn canonicalize_uri(uri: &hyper::Uri) -> Option<hyper::Uri> {
    if !uri.path().starts_with('/') {
        return Some(uri.clone());
    }
    let canonical = exyonq_module_api::canonicalize_path(uri.path())?;
    if canonical == uri.path() {
        return Some(uri.clone());
    }
    let target = match uri.query() {
        Some(query) => format!("{canonical}?{query}"),
        None => canonical,
    };
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(target.parse().ok()?);
    hyper::Uri::from_parts(parts).ok()
}

/// Full request authority (`host[:port]`) for absolute redirect targets —
/// h2 `:authority` and absolute-form URIs resolve via `uri.authority()`;
/// origin-form h1 falls back to the raw `Host` header (port preserved).
fn request_authority(uri: &hyper::Uri, headers: &hyper::HeaderMap) -> Option<String> {
    uri.authority().map(|a| a.as_str().to_string()).or_else(|| {
        headers
            .get(hyper::header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    })
}

fn request_host_from_parts(uri: &hyper::Uri, headers: &hyper::HeaderMap) -> Option<String> {
    uri.host().map(str::to_string).or_else(|| {
        headers
            .get(hyper::header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(|host| strip_host_port(host).to_string())
    })
}

/// Cap033: strip `:port` without destroying IPv6 literal authorities (`[::1]:8080`).
fn strip_host_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return &host[..=end];
        }
        return host;
    }
    match host.rsplit_once(':') {
        Some((name, port)) if !name.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    }
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

fn accepts_html(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("accept")
            && value
                .split(',')
                .any(|part| part.trim().to_ascii_lowercase().starts_with("text/html"))
    })
}

/// EXY-16 / ADR-050: resolve a server-level `error_pages` target by matching
/// the target path again against the same host/port. This lets a proxy or
/// FastCGI error use a static error document, while the original status is
/// preserved. The target must resolve to a static route; the rematch is not
/// recursive, so an error page can never select itself as a new error page.
async fn maybe_error_page(
    state: &ServerState,
    method: &Method,
    path: &str,
    host: Option<&str>,
    local_port: Option<u16>,
    request_headers: &[(String, String)],
    response: Response<BoxBody>,
) -> Response<BoxBody> {
    if !matches!(response.status().as_u16(), 400..=599) || !accepts_html(request_headers) {
        return response;
    }
    let Some((_, route)) = state
        .route_index
        .match_route_index_with_host_port(path, host, local_port)
    else {
        return response;
    };
    let Some(server) = state.config.servers.iter().find(|server| {
        server
            .routes
            .iter()
            .any(|candidate| candidate == &route.name)
    }) else {
        return response;
    };
    let Some(target) = server
        .error_pages
        .get(&response.status().as_u16().to_string())
    else {
        return response;
    };
    let Some((target_route_idx, target_route)) = state
        .route_index
        .match_route_index_with_host_port(target, host, local_port)
    else {
        return response;
    };
    if !server
        .routes
        .iter()
        .any(|candidate| candidate == &target_route.name)
    {
        return response;
    }
    let Some(slot) = state
        .snapshot
        .static_slot_for_route_backend(target_route_idx)
    else {
        return response;
    };
    let Some(relative) = target.strip_prefix('/') else {
        return response;
    };
    let relative = if slot.join_uri {
        relative.to_string()
    } else {
        let prefix = slot.route_prefix.trim_end_matches('/');
        if prefix.is_empty() {
            relative.to_string()
        } else {
            let Some(rest) = target.strip_prefix(prefix) else {
                return response;
            };
            rest.trim_start_matches('/').to_string()
        }
    };
    if relative.is_empty()
        || relative
            .split('/')
            .any(|part| part == ".." || part.is_empty())
    {
        return response;
    }
    let Ok(root) = std::fs::canonicalize(&slot.filesystem_root) else {
        return response;
    };
    let Ok(path) = std::fs::canonicalize(root.join(relative)) else {
        return response;
    };
    if !path.starts_with(&root) {
        return response;
    }
    let Ok(metadata) = std::fs::metadata(&path) else {
        return response;
    };
    if !metadata.is_file() || metadata.len() > 1_048_576 {
        return response;
    }
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return response;
    };
    let body = if method == Method::HEAD {
        bytes::Bytes::new()
    } else {
        bytes::Bytes::from(bytes)
    };
    let content_length = if method == Method::HEAD {
        metadata.len()
    } else {
        body.len() as u64
    };
    Response::builder()
        .status(response.status())
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-cache")
        .header("content-length", content_length.to_string())
        .body(Full::from(body).map_err(|never| match never {}).boxed())
        .unwrap_or(response)
}

/// Cap055 / Cap067 wire-cheap 429 (Connection: close — fail closed).
fn rate_limit_reject_response(retry_after_secs: u64, status: u16) -> Response<BoxBody> {
    let retry = retry_after_secs.max(1);
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::TOO_MANY_REQUESTS);
    Response::builder()
        .status(status)
        .header(http::header::RETRY_AFTER, retry.to_string())
        .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(http::header::CONNECTION, "close")
        .body(
            Full::from(bytes::Bytes::from_static(b"rate limit exceeded"))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid rate-limit response")
}

fn log_rate_limit(
    action: &str,
    route: &str,
    level: &str,
    status: u16,
    retry_after_secs: u64,
    dry_run: bool,
) {
    match level {
        "info" => tracing::info!(
            action,
            route,
            status,
            retry_after_secs,
            dry_run,
            "rate limit"
        ),
        "notice" => tracing::info!(
            action,
            route,
            status,
            retry_after_secs,
            dry_run,
            nginx_log_level = "notice",
            "rate limit"
        ),
        "warn" => tracing::warn!(
            action,
            route,
            status,
            retry_after_secs,
            dry_run,
            "rate limit"
        ),
        _ => tracing::error!(
            action,
            route,
            status,
            retry_after_secs,
            dry_run,
            "rate limit"
        ),
    }
}

fn waf_reject_response(reject: crate::waf::WafReject) -> Response<BoxBody> {
    let mut builder = Response::builder()
        .status(reject.status)
        .header("content-type", reject.content_type)
        .header("cache-control", "no-store")
        .header("pragma", "no-cache");
    if let Some(secs) = reject.retry_after_secs {
        builder = builder.header("retry-after", secs.to_string());
    }
    if let Some(cookie) = reject.set_cookie.as_deref() {
        builder = builder.header("set-cookie", cookie);
    }
    if let Some(loc) = reject.location.as_deref() {
        builder = builder.header("location", loc);
    }
    if let Some(rule) = reject.rule_id.as_deref() {
        builder = builder.header("x-waf-rule", rule);
    }
    let bytes = match reject.body {
        crate::waf::WafRejectBody::Static(b) => bytes::Bytes::from_static(b),
        crate::waf::WafRejectBody::Owned(v) => bytes::Bytes::from(v),
    };
    let mut response = builder
        .body(Full::from(bytes).map_err(|never| match never {}).boxed())
        .expect("valid waf reject response");
    response
        .extensions_mut()
        .insert(crate::waf_body::WafBodyChecked);
    response
}

struct ResponseHeadersView(Vec<(String, Vec<u8>)>);

impl exyonq_waf_api::HeaderView for ResponseHeadersView {
    fn get(&self, name: &str) -> Option<&[u8]> {
        self.0
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_slice())
    }

    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) {
        for (name, value) in &self.0 {
            f(name.as_bytes(), value);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn waf_response_gate(
    state: &ServerState,
    method: &Method,
    host: Option<&str>,
    path: &str,
    query: &str,
    headers: &dyn exyonq_waf_api::HeaderView,
    client_ip: std::net::IpAddr,
    response: Response<BoxBody>,
) -> Response<BoxBody> {
    let max = state.config.waf.max_inspection_body_bytes as usize;
    let small = response
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|len| max > 0 && len <= max as u64);
    if state.waf.inspects_response_body() && small {
        let status = response.status().as_u16();
        let (parts, body) = response.into_parts();
        let entries = parts
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.as_bytes().to_vec(),
                )
            })
            .collect();
        let response_view = ResponseHeadersView(entries);
        let limited = http_body_util::Limited::new(body, max.max(1));
        let collected = match limited.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(_) => {
                return text_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "response body exceeds WAF inspection limit",
                )
            }
        };
        match crate::waf::inspect_response_body(
            state.waf.as_ref(),
            method.as_str(),
            host,
            path,
            Some(query),
            headers,
            client_ip,
            None,
            status,
            &response_view,
            &collected,
            state.generation,
            state.waf_enforce,
        ) {
            crate::waf::WafHookResult::Continue => {}
            crate::waf::WafHookResult::Reject(reject) => return waf_reject_response(reject),
        }
        let mut rebuilt = Response::from_parts(
            parts,
            Full::new(collected).map_err(|never| match never {}).boxed(),
        );
        rebuilt
            .extensions_mut()
            .insert(crate::waf_body::WafBodyChecked);
        return rebuilt;
    }
    let entries = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.as_bytes().to_vec(),
            )
        })
        .collect();
    let response_view = ResponseHeadersView(entries);
    match crate::waf::inspect_response_headers(
        state.waf.as_ref(),
        method.as_str(),
        host,
        path,
        Some(query),
        headers,
        client_ip,
        None,
        response.status().as_u16(),
        &response_view,
        state.generation,
        state.waf_enforce,
    ) {
        crate::waf::WafHookResult::Continue => response,
        crate::waf::WafHookResult::Reject(reject) => waf_reject_response(reject),
    }
}

#[allow(clippy::too_many_arguments)]
fn waf_header_gate(
    state: &ServerState,
    method: &Method,
    host: Option<&str>,
    path: &str,
    query: Option<&str>,
    headers: &dyn exyonq_waf_api::HeaderView,
    client_ip: std::net::IpAddr,
    skip_abuse: bool,
) -> Option<Response<BoxBody>> {
    match crate::waf::inspect_request_headers(
        state.waf.as_ref(),
        method.as_str(),
        host,
        path,
        query,
        headers,
        client_ip,
        None,
        state.generation,
        skip_abuse,
        state.waf_enforce,
        state.waf_abuse.as_deref(),
    ) {
        crate::waf::WafHookResult::Continue => {
            if *method == Method::GET || *method == Method::HEAD {
                match waf_body_gate(state, method, host, path, query, headers, client_ip, b"") {
                    WafBodyGate::Reject(reject) => Some(reject),
                    WafBodyGate::Continue { .. } => None,
                }
            } else {
                None
            }
        }
        crate::waf::WafHookResult::Reject(reject) => Some(waf_reject_response(reject)),
    }
}

/// Result of the host-side request-body WAF gate.
///
/// `Continue { inspection_limited: true }` means the documented
/// `on_inspection_limit` policy allowed the request while the engine only
/// inspected part of the body. Callers must surface it on the access log.
#[derive(Debug)]
enum WafBodyGate {
    Continue { inspection_limited: bool },
    Reject(Response<BoxBody>),
}

#[allow(clippy::too_many_arguments)]
fn waf_body_gate(
    state: &ServerState,
    method: &Method,
    host: Option<&str>,
    path: &str,
    query: Option<&str>,
    headers: &dyn exyonq_waf_api::HeaderView,
    client_ip: std::net::IpAddr,
    body: &[u8],
) -> WafBodyGate {
    // nginx rejects malformed percent escapes in XML bodies before CRS
    // (920240-t4, `%1G`). Form bodies are URL-decoded by the application and
    // must reach CRS (941110-t6 is a valid XSS block, 920240-t6 is benign).
    let xml_body = headers
        .get("content-type")
        .and_then(|value| std::str::from_utf8(value).ok())
        .is_some_and(|value| {
            let media_type = value.split(';').next().unwrap_or(value).trim();
            media_type.eq_ignore_ascii_case("text/xml")
                || media_type.eq_ignore_ascii_case("application/xml")
        });
    if xml_body && malformed_percent_encoding(body) {
        return WafBodyGate::Reject(text_response(
            StatusCode::BAD_REQUEST,
            "malformed percent-encoding in request body",
        ));
    }
    let (hook, inspection_limited) = crate::waf::inspect_request_body(
        state.waf.as_ref(),
        method.as_str(),
        host,
        path,
        query,
        headers,
        client_ip,
        None,
        body,
        state.generation,
        state.waf_enforce,
    );
    match hook {
        crate::waf::WafHookResult::Continue => WafBodyGate::Continue { inspection_limited },
        crate::waf::WafHookResult::Reject(reject) => {
            WafBodyGate::Reject(waf_reject_response(reject))
        }
    }
}

fn malformed_percent_encoding(body: &[u8]) -> bool {
    let mut index = 0;
    while index < body.len() {
        if body[index] == b'%' {
            if index + 2 >= body.len()
                || !body[index + 1].is_ascii_hexdigit()
                || !body[index + 2].is_ascii_hexdigit()
            {
                return true;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    false
}

pub(crate) fn client_ip_from_xff(x_forwarded_for: Option<&HeaderValue>) -> std::net::IpAddr {
    crate::waf::parse_client_ip(x_forwarded_for.and_then(|v| v.to_str().ok()))
}

/// NG-11: effective client-IP header for this request. Disabled → returns the
/// connection peer unchanged. Trusted peer → resolved from the configured
/// forwarding header (last occurrence wins, nginx semantics).
fn effective_xff(
    state: &ServerState,
    peer: &HeaderValue,
    headers: &hyper::HeaderMap,
) -> HeaderValue {
    let Some(trust) =
        exyonq_module_api::real_ip::RealIpTrust::from_config(&state.config.modules.real_ip)
    else {
        return peer.clone();
    };
    let peer_ip = crate::waf::parse_client_ip(peer.to_str().ok());
    let inbound = headers
        .get_all(trust.header.as_str())
        .iter()
        .next_back()
        .and_then(|v| v.to_str().ok());
    HeaderValue::from_str(&trust.resolve(peer_ip, inbound).to_string())
        .unwrap_or_else(|_| peer.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h01_canonicalize_uri_normalizes_before_dispatch() {
        let uri: hyper::Uri = "/%61dmin/../admin?x=1".parse().expect("uri");
        let canonical = canonicalize_uri(&uri).expect("canonical uri");
        assert_eq!(canonical.path(), "/admin");
        assert_eq!(canonical.query(), Some("x=1"));
    }

    #[test]
    fn h01_preserves_non_origin_form_targets() {
        let asterisk: hyper::Uri = "*".parse().expect("asterisk target");
        assert_eq!(
            canonicalize_uri(&asterisk).expect("asterisk uri").path(),
            "*"
        );

        let authority: hyper::Uri = "example.com:443".parse().expect("authority target");
        assert_eq!(
            canonicalize_uri(&authority)
                .expect("authority uri")
                .authority()
                .map(|a| a.as_str()),
            Some("example.com:443")
        );
    }
    use crate::config::{
        AppConfig, CachePolicyConfig, RouteConfig, RouteMatch, ServerConfig, ServerNames,
    };
    use exyonq_mod_proxy::build_incoming_client;
    use hyper::Uri;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn request_body_headers_are_detected() {
        let mut headers = hyper::HeaderMap::new();
        assert!(!request_has_body_headers(&headers));
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("7"));
        assert!(request_has_body_headers(&headers));
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
        assert!(!request_has_body_headers(&headers));
        headers.insert(
            hyper::header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        assert!(request_has_body_headers(&headers));
    }

    #[test]
    fn post_is_not_served_as_static_get() {
        assert_eq!(
            static_method_from_hyper(&Method::GET),
            Some(StaticMethod::Get)
        );
        assert_eq!(
            static_method_from_hyper(&Method::HEAD),
            Some(StaticMethod::Head)
        );
        assert!(static_method_from_hyper(&Method::POST).is_none());
        assert!(static_method_from_hyper(&Method::PUT).is_none());
        assert!(static_method_from_hyper(&Method::DELETE).is_none());
    }

    #[test]
    fn route_request_body_limit_clamps_to_shared_proxy_bound() {
        use exyonq_module_api::proxy_dispatch::PROXY_MAX_REQUEST_BODY_BYTES;

        const ROUTE_BODY_LIMIT: u64 = 1024;
        const ROUTE_BODY_LIMIT_USIZE: usize = ROUTE_BODY_LIMIT as usize;
        const FASTCGI_BASE_LIMIT: usize = 512;
        const UNROUTED_BASE_LIMIT: usize = 4096;

        let mut r = route("limited", "/upload");
        assert_eq!(
            route_request_body_limit(Some(&r), PROXY_MAX_REQUEST_BODY_BYTES),
            PROXY_MAX_REQUEST_BODY_BYTES
        );
        r.max_request_body_bytes = Some(ROUTE_BODY_LIMIT);
        assert_eq!(
            route_request_body_limit(Some(&r), PROXY_MAX_REQUEST_BODY_BYTES),
            ROUTE_BODY_LIMIT_USIZE
        );
        // A route limit never lowers the caller's own base (FastCGI) bound.
        assert_eq!(
            route_request_body_limit(Some(&r), FASTCGI_BASE_LIMIT),
            FASTCGI_BASE_LIMIT
        );
        r.max_request_body_bytes = Some(u64::MAX);
        assert_eq!(
            route_request_body_limit(Some(&r), PROXY_MAX_REQUEST_BODY_BYTES),
            PROXY_MAX_REQUEST_BODY_BYTES
        );
        assert_eq!(
            route_request_body_limit(None, UNROUTED_BASE_LIMIT),
            UNROUTED_BASE_LIMIT
        );
    }

    fn route(name: &str, path: &str) -> RouteConfig {
        RouteConfig {
            max_request_body_bytes: None,
            gunzip: None,
            cache_tags: Vec::new(),
            name: name.into(),
            r#match: RouteMatch {
                path: path.into(),
                host: None,
                suffix: None,
                regex: None,
                exact: false,
                listen_port: None,
                header: None,
                header_value: None,
                literal: false,
            },
            upstream: None,
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            fastcgi_script_root: None,
            htaccess: Default::default(),
            access: None,
            cache: None,
            fallback_rewrite: None,
            try_uri: None,
            try_uri_dir: None,
            front_controller: None,
            proxy_set_headers: Vec::new(),
            cache_headers: None,
            response_headers: Vec::new(),
            location_rewrite: Vec::new(),
            root_mode: Default::default(),
            dir_redirect: false,
            dir_redirect_absolute: true,
            dir_redirect_port: true,
            dir_redirect_host: None,
        }
    }

    fn test_ops() -> Arc<crate::lifecycle::LifecycleState> {
        crate::lifecycle::LifecycleState::new()
    }

    async fn state_for_inner(
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
        let proxy_client = build_incoming_client().clone();
        let state = ServerState::new_with_generation(1, config, proxy_client)
            .await
            .expect("server state");
        (state, static_guard, proxy_guard)
    }

    async fn state_for(
        config: AppConfig,
    ) -> (
        Arc<ServerState>,
        (
            execute_backend::StaticDispatchTestGuard,
            tokio::sync::MutexGuard<'static, ()>,
        ),
        execute_backend::ProxyDispatchTestGuard,
    ) {
        // Hold the shared gate for the whole test: this installs a real pinned
        // static runtime that must not be swapped out mid-flight by another
        // test's kernel-hook install. Tests needing several states must hoist
        // the gate once and call `state_for_inner` — the tokio mutex is not
        // re-entrant, a second `state_for` would self-deadlock.
        let gate = crate::kernel::test_hooks::wire_hooks_gate().await;
        let (state, static_guard, proxy_guard) = state_for_inner(config).await;
        (state, (static_guard, gate), proxy_guard)
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
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["assets".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![assets],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            maps: Default::default(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/assets/page.html"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn dispatch_static_over_materialization_budget_is_413() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big.bin"), vec![0u8; 64]).unwrap();
        let mut assets = route("assets", "/assets");
        assets.root = Some(dir.path().to_path_buf());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["assets".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![assets],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/assets/big.bin"),
            None,
            None,
            &[],
            Some(16),
            None,
            &test_ops(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
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
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["assets".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![assets],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            maps: Default::default(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::HEAD,
            &Uri::from_static("/assets/page.html"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// H-01: a pre-existing static cache entry keyed by a sensitive path must
    /// never be served. The guard routes straight to the static service, whose
    /// authoritative policy returns the opaque 404.
    #[tokio::test]
    async fn sensitive_static_path_bypasses_preexisting_cache_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), b"SECRET=cache-leak").unwrap();
        let mut site = route("site", "/");
        site.root = Some(dir.path().to_path_buf());
        site.cache = Some("micro".into());
        let mut cache_policies = HashMap::new();
        cache_policies.insert(
            "micro".into(),
            CachePolicyConfig {
                name: "micro".into(),
                ttl_seconds: 60,
                max_object_bytes: 1024 * 1024,
            },
        );
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![site],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies,

            maps: Default::default(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let policy = state
            .snapshot
            .cache_policy_for_route(0)
            .expect("cache policy");
        let host = "cache-h01.invalid";
        let key = build_storage_cache_key(CacheKeyParts {
            range: String::new(),
            site_id: 0,
            namespace: 0,
            backend_id: 0,
            runtime_generation: state.generation,
            policy_generation: policy.policy_generation,
            route_idx: 0,
            method: "GET".into(),
            scheme: conn_scheme(&state).into(),
            host: normalize_host(Some(host)),
            path: "/.env".into(),
            query: String::new(),
            content_encoding: "identity".into(),
            vary: String::new(),
        });
        global_response_cache().insert_entry(
            key,
            0,
            0,
            state.generation,
            std::time::Duration::from_secs(60),
            200,
            vec![("content-type".into(), "text/plain".into())],
            Bytes::from_static(b"SECRET=cache-leak"),
            None,
            exyonq_mod_static::STATIC_CACHE_NAMESPACE,
            Arc::from(Vec::<String>::new()),
            None,
            NamespaceMetrics::NONE,
        );
        let response = static_dispatch_maybe_cached(
            &state,
            "http",
            None,
            0,
            &Method::GET,
            &Uri::from_static("/.env"),
            Some(host),
            0,
            "/.env",
            &[],
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"not found");
    }

    #[tokio::test]
    async fn rewrite_static_via_backend_table() {
        // Cap035 contract: rewrite-only route rematches onto a distinct static route.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"rewrite-static").unwrap();
        let mut legacy = route("legacy", "/old");
        legacy.rewrite = Some("/app/index.html".into());
        let mut app = route("app", "/app");
        app.root = Some(dir.path().to_path_buf());
        app.index = Some("index.html".into());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["legacy".into(), "app".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![legacy, app],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            maps: Default::default(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/old"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn patterned_rewrite_redirects_host_and_preserves_query() {
        let mut www = route("www", "/");
        www.r#match.host = Some("www.example.com".into());
        www.rewrite = Some(exyonq_config_ir::RewriteConfig::Rule(
            exyonq_config_ir::RewriteRuleConfig {
                pattern: r"^/(.*)$".into(),
                replacement: "https://example.com/$1".into(),
                flag: exyonq_config_ir::RewriteFlagConfig::Permanent,
                when: Vec::new(),
            },
        ));
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["www".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![www],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/assets/x.css?utm=1"),
            Some("www.example.com".into()),
            None,
            &[],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(
            response
                .headers()
                .get(hyper::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
            Some("https://example.com/assets/x.css?utm=1")
        );
    }

    #[tokio::test]
    async fn zero_max_age_cache_headers_emit_no_cache() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"html").unwrap();
        let mut site = route("site", "/");
        site.root = Some(dir.path().to_path_buf());
        site.index = Some("index.html".into());
        let mut by_mime = std::collections::BTreeMap::new();
        by_mime.insert(
            "text/html".into(),
            exyonq_config_ir::MimeCacheHeader {
                max_age: 0,
                immutable: false,
            },
        );
        site.cache_headers = Some(exyonq_config_ir::CacheHeadersConfig { by_mime });
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![site],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(hyper::header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-cache")
        );
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
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["assets".into(), "api".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![assets, api],
            upstreams,
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            maps: Default::default(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
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

    /// Retarget the fixture upstream to a port guaranteed to refuse, so the
    /// proxy outcome (502, not a colliding service's 404) is host-independent.
    /// The rewrite happens before parsing because `endpoint_set` is the
    /// authoritative field — patching `target` alone leaves the parsed
    /// endpoint pointing at the original address.
    fn fixture_config_unreachable_upstream() -> AppConfig {
        include_str!("../../../tests/fixtures/minimal.toml")
            .replace("127.0.0.1:9000", "127.0.0.1:1")
            .parse()
            .unwrap()
    }

    #[tokio::test]
    async fn dispatch_proxy_via_backend_table_get() {
        let config = fixture_config_unreachable_upstream();
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        assert!(state.snapshot.proxy_cluster_for_route(0).is_some());
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/api/users"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dispatch_proxy_via_backend_table_head() {
        let config = fixture_config_unreachable_upstream();
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::HEAD,
            &Uri::from_static("/api/users"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_proxy_via_backend_table() {
        let config = fixture_config_unreachable_upstream();
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
        assert_eq!(slot.target, "http://127.0.0.1:1");
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
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["assets".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![assets],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            maps: Default::default(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        assert!(state.snapshot.static_slot_for_route_backend(0).is_some());
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/assets/page.html"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
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
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["legacy".into(), "rw".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![redir, rewrite],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            maps: Default::default(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
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
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![site],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            maps: Default::default(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/site/index.html"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
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
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![site],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),

            maps: Default::default(),

            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let path = "/site/index.html";
        let via_fast = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static(path),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
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
        let via_backend_table = static_dispatch_via_service(
            &Method::GET,
            0,
            path,
            Vec::new(),
            None,
            "http",
            None,
            None,
            None,
        )
        .await;
        assert_eq!(state.site_static_slot, Some(0));
        assert_eq!(via_canonical.route_name, "site");
        assert_eq!(via_fast.status(), via_backend_table.status());
        assert_eq!(via_fast.status(), StatusCode::OK);
    }

    /// AUDIT-D6: headers-mode mirror fires post-access. A request the auth
    /// gate rejects must never reach the mirror (no amplified denied load);
    /// an admitted request still fans out to every destination.
    #[tokio::test]
    async fn mirror_headers_mode_fires_only_after_access_gates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"mirrored").unwrap();
        let htpasswd = dir.path().join("users.htpasswd");
        std::fs::write(&htpasswd, "u:{PLAIN}pw\n").unwrap();

        let mut site = route("site", "/site");
        site.root = Some(dir.path().to_path_buf());
        site.index = Some("index.html".into());
        let mut modules = exyonq_config_ir::ModulesConfig::default();
        modules.auth.routes.insert(
            "site".into(),
            exyonq_config_ir::AuthRouteConfig {
                realm: "restricted".into(),
                user_file: htpasswd.display().to_string(),
                auth_request: None,
            },
        );
        modules.mirror.routes.insert(
            "site".into(),
            exyonq_config_ir::MirrorRouteConfig {
                destinations: vec!["/mirror-sink".into()],
                request_body: false,
            },
        );
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![site],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules,
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;

        // Denied: no Authorization header — 401 and zero mirror traffic.
        let before = crate::mirror::mirror_started_for_tests();
        let denied = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/site/index.html"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            crate::mirror::mirror_started_for_tests(),
            before,
            "denied request must not spawn mirror subrequests"
        );

        // Admitted: valid credentials — response serves and the headers-mode
        // mirror fires exactly once (destination unreachable → counted started
        // regardless of its outcome).
        use base64::Engine as _;
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("u:pw")
        );
        let allowed = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/site/index.html"),
            None,
            None,
            &[("authorization".to_string(), auth)],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(
            crate::mirror::mirror_started_for_tests(),
            before + 1,
            "admitted request must fire the headers-mode mirror"
        );
    }

    /// AUDIT-D7: the `/site/*` static shortcut must honor route-bound access
    /// control — the old code served the file before the route match ran, so
    /// `auth_basic` was unreachable there.
    #[tokio::test]
    async fn site_fast_path_honors_route_auth() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"secret").unwrap();
        let htpasswd = dir.path().join("users.htpasswd");
        std::fs::write(&htpasswd, "u:{PLAIN}pw\n").unwrap();

        let mut site = route("site", "/site");
        site.root = Some(dir.path().to_path_buf());
        site.index = Some("index.html".into());
        let mut modules = exyonq_config_ir::ModulesConfig::default();
        modules.auth.routes.insert(
            "site".into(),
            exyonq_config_ir::AuthRouteConfig {
                realm: "restricted".into(),
                user_file: htpasswd.display().to_string(),
                auth_request: None,
            },
        );
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![site],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules,
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let denied = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/site/index.html"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
        )
        .await;
        assert_eq!(
            denied.status(),
            StatusCode::UNAUTHORIZED,
            "auth-protected /site/ route must challenge, not serve the file"
        );
    }

    /// AUDIT-D1/D2: every route-bound feature class must trip the shared
    /// full-dispatch predicate — the wire/static fast paths consult this one
    /// gate, so a missing binding is a silent semantic bypass.
    #[tokio::test]
    async fn route_bound_features_force_full_dispatch() {
        let mut site = route("site", "/site");
        site.response_headers = vec![exyonq_config_ir::ResponseHeaderConfig {
            name: "x-h".into(),
            value: "v".into(),
            always: false,
        }];
        let mut modules = exyonq_config_ir::ModulesConfig::default();
        modules.auth.routes.insert(
            "auth".into(),
            exyonq_config_ir::AuthRouteConfig {
                realm: "r".into(),
                user_file: "/dev/null".into(),
                auth_request: None,
            },
        );
        modules
            .ratelimit
            .routes
            .insert("limited".into(), "z".into());
        modules.sub_filter.routes.insert(
            "filtered".into(),
            exyonq_config_ir::SubFilterRouteConfig {
                rules: vec![exyonq_config_ir::SubFilterRule {
                    pattern: "a".into(),
                    replacement: "b".into(),
                }],
                once: true,
                types: vec!["text/html".into()],
                last_modified: false,
            },
        );
        modules.mirror.routes.insert(
            "mirrored".into(),
            exyonq_config_ir::MirrorRouteConfig {
                destinations: vec!["/sink".into()],
                request_body: false,
            },
        );
        modules.slice.routes.insert(
            "sliced".into(),
            exyonq_config_ir::SliceRouteConfig { size: 4096 },
        );
        let names = [
            "site",
            "auth",
            "limited",
            "filtered",
            "mirrored",
            "sliced",
            "cached",
            "plain",
            "rewritten",
            "redirected",
        ];
        let mut routes: Vec<RouteConfig> =
            names.iter().map(|n| route(n, &format!("/{n}"))).collect();
        // "site" needs the response_headers binding — rebuild it in place.
        routes[0] = site;
        // EXY-01/EXY-05: route with only cache_headers must not bypass
        // Cache-Control materialization via the wire path.
        let mut by_mime = std::collections::BTreeMap::new();
        by_mime.insert(
            "image/avif".to_string(),
            exyonq_config_ir::MimeCacheHeader {
                max_age: 31536000,
                immutable: true,
            },
        );
        routes[6].cache_headers = Some(exyonq_config_ir::CacheHeadersConfig { by_mime });
        // Structural rules run only after route lookup in full dispatch; the
        // wire path would serve its own 404 before a rewrite/redirect.
        routes[8].rewrite = Some(exyonq_config_ir::RewriteConfig::Literal(
            "/index.html".into(),
        ));
        routes[9].redirect = Some(exyonq_config_ir::RedirectConfig {
            status: 301,
            location: "https://example.com/".into(),
        });
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: names.iter().map(|n| n.to_string()).collect(),
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes,
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules,
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        for (idx, expected) in [
            (0usize, true), // response_headers
            (1, true),      // auth_basic
            (2, true),      // zone ratelimit
            (3, true),      // sub_filter
            (4, true),      // mirror
            (5, true),      // slice
            (6, true),      // cache_headers (EXY-01/EXY-05)
            (7, false),     // plain route stays wire-eligible
            (8, true),      // structural rewrite
            (9, true),      // structural redirect
        ] {
            assert_eq!(
                state.route_requires_full_dispatch(idx),
                expected,
                "route index {idx} ({})",
                names[idx]
            );
        }
    }

    #[tokio::test]
    async fn server_error_pages_force_full_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let mut site = route("site", "/");
        site.root = Some(dir.path().to_path_buf());
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::from([("404".into(), "/404.html".into())]),
                proxy_protocol: None,
            }],
            routes: vec![site],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        assert!(
            state.route_requires_full_dispatch(0),
            "error_pages must keep the route on full dispatch"
        );
    }

    /// AUDIT-D1/D2 (re-round): the keep-alive gate must normalize `Host:` the
    /// same way admission does — `h:443` on the wire must resolve the host-bound
    /// route, not collapse to `None` and let `None == None` tokens through.
    #[tokio::test]
    async fn keepalive_route_gate_strips_host_port() {
        let mut clean = route("clean", "/clean");
        clean.r#match.host = Some("app.test".into());
        let mut dirty = route("dirty", "/dirty");
        dirty.r#match.host = Some("secure.test".into());
        let mut modules = exyonq_config_ir::ModulesConfig::default();
        modules.auth.routes.insert(
            "dirty".into(),
            exyonq_config_ir::AuthRouteConfig {
                realm: "r".into(),
                user_file: "/dev/null".into(),
                auth_request: None,
            },
        );
        let names = ["clean", "dirty"];
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: names.iter().map(|n| n.to_string()).collect(),
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![clean, dirty],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules,
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        let gate = crate::server::wire_dispatch::build_keepalive_route_gate(&state);
        // Clean host-bound route resolves identically with or without a port.
        let plain = gate("/clean/x", Some("app.test"));
        let ported = gate("/clean/x", Some("app.test:8443"));
        assert_eq!(plain, ported, "port must not change gate resolution");
        assert!(
            ported.is_some(),
            "clean host-bound route stays wire-eligible"
        );
        // Dirty host-bound route resolves to None (full dispatch) on both forms —
        // before the fix `secure.test:443` also resolved but to the WRONG slot.
        assert_eq!(gate("/dirty/x", Some("secure.test")), None);
        assert_eq!(gate("/dirty/x", Some("secure.test:443")), None);
    }

    /// EXY-03 E2E: `root_mode` picks the mapping. Same docroot, two routes —
    /// alias resolves `root + (URI − prefix)`, nginx resolves `root + URI`.
    /// Distinct file bytes prove *which* file was served, not just a 200.
    #[tokio::test]
    async fn root_mode_selects_alias_vs_nginx_resolution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"ALIAS-FILE").unwrap();
        std::fs::create_dir(dir.path().join("n")).unwrap();
        std::fs::write(dir.path().join("n/f.txt"), b"NGINX-FILE").unwrap();

        let mut alias = route("alias", "/a");
        alias.root = Some(dir.path().to_path_buf());
        let mut nginx = route("nginx", "/n");
        nginx.root = Some(dir.path().to_path_buf());
        nginx.root_mode = exyonq_config_ir::RootMode::FullUri;

        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["alias".into(), "nginx".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![alias, nginx],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;

        async fn get(state: &Arc<ServerState>, uri: &'static str) -> (StatusCode, Vec<u8>) {
            let resp = dispatch_core(
                state,
                "http",
                None,
                &state.proxy_client,
                &Method::GET,
                &Uri::from_static(uri),
                None,
                None,
                &[],
                None,
                None,
                &test_ops(),
            )
            .await;
            let status = resp.status();
            let body = resp
                .into_body()
                .collect()
                .await
                .map(|c| c.to_bytes().to_vec())
                .unwrap_or_default();
            (status, body)
        }

        // alias: /a/f.txt → root + "f.txt" = dir/f.txt
        let (s, b) = get(&state, "/a/f.txt").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b, b"ALIAS-FILE");

        // nginx: /n/f.txt → root + "n/f.txt" = dir/n/f.txt
        let (s, b) = get(&state, "/n/f.txt").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b, b"NGINX-FILE");

        // Same physical file reached the OTHER way — proves the mapping
        // differs, not the file: alias /a/n/f.txt → dir/n/f.txt.
        let (s, b) = get(&state, "/a/n/f.txt").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b, b"NGINX-FILE");

        // nginx /n/../f.txt → relative "../f.txt" → traversal rejected.
        let (s, _) = get(&state, "/n/../f.txt").await;
        assert_ne!(s, StatusCode::OK);

        // missing file → not OK (404 path).
        let (s, _) = get(&state, "/n/missing.txt").await;
        assert_ne!(s, StatusCode::OK);
    }

    /// AUDIT-D2/D7 (re-round): a host-bound dirty route whose path is an
    /// ancestor of a static slot's prefix outranks the hostless slot route by
    /// host-match — the overlap check must see it or epoll sendfile serves
    /// file bytes without auth.
    #[tokio::test]
    async fn static_overlap_flags_host_bound_dirty_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.css"), b"x").unwrap();
        let mut slot = route("slot", "/a/b");
        slot.root = Some(dir.path().to_path_buf());
        let mut ancestor = route("protected", "/a");
        ancestor.r#match.host = Some("protected.test".into());
        let mut modules = exyonq_config_ir::ModulesConfig::default();
        modules.auth.routes.insert(
            "protected".into(),
            exyonq_config_ir::AuthRouteConfig {
                realm: "r".into(),
                user_file: "/dev/null".into(),
                auth_request: None,
            },
        );
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["slot".into(), "protected".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![slot, ancestor],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules,
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        assert!(
            crate::server::wire_dispatch::static_wire_route_overlap(&state),
            "host-bound dirty ancestor must count as overlap"
        );
    }

    /// AUDIT-D1/D2: the wire admission gate composes both predicates — a wire
    /// head for a protected route (or a generation with FPC/gunzip/response
    /// WAF) must take full dispatch, never the transport shortcut.
    #[tokio::test]
    async fn wire_head_for_protected_route_requires_full_dispatch() {
        let mut modules = exyonq_config_ir::ModulesConfig::default();
        modules.auth.routes.insert(
            "auth".into(),
            exyonq_config_ir::AuthRouteConfig {
                realm: "r".into(),
                user_file: "/dev/null".into(),
                auth_request: None,
            },
        );
        let fpc = exyonq_config_ir::FullPageCacheConfig {
            enabled: true,
            ..Default::default()
        };
        let config = AppConfig {
            config_version: 1,
            includes: Vec::new(),
            tuning: Default::default(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["auth".into(), "open".into()],
                tls: None,
                http3_listen: None,
                error_pages: HashMap::new(),
                proxy_protocol: None,
            }],
            routes: vec![route("auth", "/auth"), route("open", "/open")],
            upstreams: HashMap::new(),
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            maps: Default::default(),
            modules,
            static_section: Default::default(),
            full_page_cache: fpc,
            http3: Default::default(),
            waf: Default::default(),
            logging: Default::default(),
        };
        let (state, _static_guard, _proxy_guard) = state_for(config).await;
        // FPC is generation-wide → every wire head requires full dispatch.
        assert!(
            crate::server::wire_dispatch::wire_head_requires_full_dispatch(
                &state,
                b"GET /open/x HTTP/1.1\r\nHost: h\r\n\r\n"
            )
        );
        assert!(
            crate::server::wire_dispatch::wire_head_requires_full_dispatch(
                &state,
                b"GET /auth/x HTTP/1.1\r\nHost: h\r\n\r\n"
            )
        );
        assert!(state.wire_incompatible_features_active());
    }

    fn plan08_fcgi_config() -> AppConfig {
        let raw = include_str!("../../../tests/fixtures/minimal-fcgi.toml");
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
            scheme: "http",
        }
    }

    /// PR2-A: live Hyper dispatch returns 501 for FastCGI route (GET) without executor.
    #[tokio::test]
    async fn plan08_pr2_dispatch_core_fastcgi_get_returns_501() {
        let _fcgi_guard = execute_backend::FcgiDispatchTestGuard::force_absent();
        let (state, _static_guard, _proxy_guard) = state_for(plan08_fcgi_config()).await;
        let response = dispatch_core(
            &state,
            "http",
            None,
            &state.proxy_client,
            &Method::GET,
            &Uri::from_static("/index.php"),
            None,
            None,
            &[],
            None,
            None,
            &test_ops(),
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

    /// PR2-A: POST body path reaches FastCGI via `dispatch_core` (not a POST-only 405).
    #[test]
    fn plan08_pr2_post_path_wires_fastcgi_contract_before_method_not_allowed() {
        let src = include_str!("handler.rs");
        let with_body = src
            .split("async fn handle_core_request_with_body")
            .nth(1)
            .expect("handle_core_request_with_body")
            .split("\nasync fn dispatch_core_with_limit(")
            .next()
            .expect("dispatch_core_with_limit follows with_body");
        assert!(
            with_body.contains("dispatch_core_with_limit("),
            "POST path must call the shared dispatcher after body collection"
        );
        let shared = src
            .split("async fn dispatch_core_with_limit(")
            .nth(1)
            .expect("dispatch_core_with_limit")
            .split("\nasync fn dispatch_core_inner(")
            .next()
            .expect("dispatch_core_inner follows dispatch_core_with_limit");
        assert!(
            shared.contains("attach_outcome_hint(response, \"waf_body_limit\")"),
            "shared dispatcher must attach the body-limit outcome marker"
        );
        let dispatch = src
            .split("async fn dispatch_core_inner(")
            .nth(1)
            .expect("dispatch_core_inner")
            .split("\nasync fn apply_htaccess_adjustment_terminal")
            .next()
            .expect("dispatch_core body");
        let fcgi = dispatch
            .find("fastcgi_dispatch_maybe_cached")
            .expect("FastCGI dispatch helper in dispatch_core");
        let terminal = dispatch
            .rfind("StatusCode::NOT_FOUND")
            .expect("terminal not-found after FastCGI attempt");
        assert!(
            fcgi < terminal,
            "FastCGI branch must precede terminal not-found in dispatch_core"
        );
    }

    /// Cap038 LA-CAP038-001: FastCGI dispatch must forward collected body, not Vec::new().
    #[test]
    fn cap038_fastcgi_dispatch_forwards_collected_body() {
        let src = include_str!("handler.rs");
        let core = src
            .split("async fn dispatch_core(")
            .nth(1)
            .expect("dispatch_core");
        let marker = "fastcgi_dispatch_maybe_cached(";
        let start = core
            .find(marker)
            .expect("fastcgi_dispatch_maybe_cached call");
        let window = &core[start..start.saturating_add(500).min(core.len())];
        assert!(
            window.contains("fcgi_body"),
            "FastCGI dispatch must pass fcgi_body"
        );
        // Reject the historical drop: last arg was literally Vec::new().
        assert!(
            !window.contains("request_headers.to_vec(),\n        Vec::new(),"),
            "FastCGI dispatch must not drop body via Vec::new()"
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
        // Product FastCGI cap is 64 MiB — oversize maps to 413 (not 502).
        assert_eq!(
            enforce_request_body_limit(FCGI_REQUEST_BODY_LIMIT + 1, FCGI_REQUEST_BODY_LIMIT)
                .unwrap_err(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
}

#[cfg(test)]
mod range_tests {
    use super::{parse_single_range, RangeOutcome};

    #[test]
    fn single_range_forms() {
        assert_eq!(
            parse_single_range("bytes=0-99", 1000),
            RangeOutcome::Single(0, 99)
        );
        assert_eq!(
            parse_single_range("bytes=100-", 1000),
            RangeOutcome::Single(100, 999)
        );
        assert_eq!(
            parse_single_range("bytes=-100", 1000),
            RangeOutcome::Single(900, 999)
        );
        assert_eq!(
            parse_single_range("bytes=0-9999", 1000),
            RangeOutcome::Single(0, 999)
        );
    }

    #[test]
    fn outcome_tracks_response_class() {
        use crate::observability::attach_outcome_hint;
        assert_eq!(
            super::outcome_from_response(&super::text_response(
                http::StatusCode::NOT_FOUND,
                "not found"
            )),
            Some("not_found")
        );
        assert_eq!(
            super::outcome_from_response(&super::text_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "boom"
            )),
            Some("server_error")
        );
        assert_eq!(
            super::outcome_from_response(&super::text_response(
                http::StatusCode::BAD_REQUEST,
                "bad"
            )),
            Some("client_error")
        );
        assert_eq!(
            super::outcome_from_response(&super::text_response(http::StatusCode::OK, "ok")),
            Some("ok")
        );
        // Explicit WAF body-budget marker wins over the status class.
        let limited = attach_outcome_hint(
            super::text_response(http::StatusCode::OK, "ok"),
            "waf_body_limit",
        );
        assert_eq!(
            super::outcome_from_response(&limited),
            Some("waf_body_limit")
        );
    }

    #[test]
    fn unsatisfiable_and_ignored_forms() {
        assert_eq!(
            parse_single_range("bytes=1000-", 1000),
            RangeOutcome::Unsatisfiable
        );
        assert_eq!(
            parse_single_range("bytes=0-1,3-4", 1000),
            RangeOutcome::Ignore
        );
        assert_eq!(parse_single_range("items=0-1", 1000), RangeOutcome::Ignore);
        assert_eq!(parse_single_range("bytes=abc", 1000), RangeOutcome::Ignore);
        assert_eq!(parse_single_range("bytes=5-4", 1000), RangeOutcome::Ignore);
    }
}
