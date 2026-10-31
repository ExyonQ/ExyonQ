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
//! NG-18 / ADR-064 Phase D — bounded nginx `mirror` subrequests.
//!
//! Reference: nginx 1.27 `ngx_http_mirror_module`. Mirror subrequests are
//! background (the main request never waits), the response is discarded, and a
//! mirror failure never changes the main response. `mirror_request_body on`
//! (the nginx default) reads the client body *before* creating subrequests, so
//! the body is materialized once by the main path and reused here.

use crate::server::handler::{serve_http3_request, ConnectionContext};
use bytes::Bytes;
use hyper::{Method, Request};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub(crate) const MIRROR_GUARD: &str = "x-exyonq-mirror";

static MIRROR_STARTED: AtomicU64 = AtomicU64::new(0);
static MIRROR_COMPLETED: AtomicU64 = AtomicU64::new(0);
static MIRROR_FAILED: AtomicU64 = AtomicU64::new(0);
static MIRROR_CANCELLED: AtomicU64 = AtomicU64::new(0);

/// ADR-064:56 — mirror lifecycle counters. `cancelled` stays 0 today: spawned
/// subrequests are detached (nginx fires and forgets), so no cancel path exists.
pub fn append_mirror_prometheus(out: &mut String) {
    let started = MIRROR_STARTED.load(Ordering::Relaxed);
    let completed = MIRROR_COMPLETED.load(Ordering::Relaxed);
    let failed = MIRROR_FAILED.load(Ordering::Relaxed);
    let cancelled = MIRROR_CANCELLED.load(Ordering::Relaxed);
    out.push_str(
        "# HELP exyonq_mirror_subrequests_total Mirror subrequests by outcome.\n\
         # TYPE exyonq_mirror_subrequests_total counter\n",
    );
    out.push_str(&format!(
        "exyonq_mirror_subrequests_total{{outcome=\"started\"}} {started}\n"
    ));
    out.push_str(&format!(
        "exyonq_mirror_subrequests_total{{outcome=\"completed\"}} {completed}\n"
    ));
    out.push_str(&format!(
        "exyonq_mirror_subrequests_total{{outcome=\"failed\"}} {failed}\n"
    ));
    out.push_str(&format!(
        "exyonq_mirror_subrequests_total{{outcome=\"cancelled\"}} {cancelled}\n"
    ));
}

/// Internal extension marker — a client can spoof the guard header to
/// suppress mirroring, but cannot inject a request extension.
#[derive(Clone)]
pub(crate) struct MirrorMarker;

/// Mirror routes with `mirror_request_body off` (empty body). Fired from the
/// header hook so it also covers paths answered before body dispatch.
pub(crate) fn maybe_mirror_headers(
    ctx: &ConnectionContext,
    path: &str,
    host: Option<&str>,
    method: &Method,
    request_headers: &[(String, String)],
    query: Option<&str>,
) {
    fire(
        ctx,
        path,
        host,
        method,
        request_headers,
        query,
        false,
        &Bytes::new(),
    );
}

/// Mirror routes with `mirror_request_body on`. Fired once the main request
/// body is materialized (nginx reads it before creating mirror subrequests).
pub(crate) fn maybe_mirror_body(
    ctx: &ConnectionContext,
    path: &str,
    host: Option<&str>,
    method: &Method,
    request_headers: &[(String, String)],
    query: Option<&str>,
    body: &Bytes,
) {
    fire(ctx, path, host, method, request_headers, query, true, body);
}

#[allow(clippy::too_many_arguments)]
fn fire(
    ctx: &ConnectionContext,
    path: &str,
    host: Option<&str>,
    method: &Method,
    request_headers: &[(String, String)],
    query: Option<&str>,
    want_body: bool,
    body: &Bytes,
) {
    if request_headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(MIRROR_GUARD))
    {
        return;
    }
    let Some((_, route)) = ctx
        .state
        .route_index
        .match_route_index_with_host(path, host)
    else {
        return;
    };
    let Some(config) = ctx.state.config.modules.mirror.routes.get(&route.name) else {
        return;
    };
    if config.request_body != want_body {
        return;
    }
    for destination in &config.destinations {
        // nginx: `ngx_http_subrequest(r, &name[i], &r->args, ...)` — the parent
        // query string is appended to every mirror destination.
        let uri = match query {
            Some(q) if !q.is_empty() => format!("{destination}?{q}"),
            _ => destination.clone(),
        };
        let state = Arc::clone(&ctx.state);
        let proxy_client = ctx.proxy_client.clone();
        let xff = ctx.x_forwarded_for.clone();
        let ops = Arc::clone(&ctx.ops);
        let method = method.clone();
        let host = host.map(str::to_string);
        let headers = request_headers.to_vec();
        let body = body.clone(); // Bytes is refcounted: no body copy per destination.
        MIRROR_STARTED.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let mut builder = Request::builder()
                .method(method)
                .uri(&uri)
                .header(MIRROR_GUARD, "1")
                .extension(MirrorMarker);
            if let Some(host) = host {
                builder = builder.header("host", host);
            }
            for (name, value) in headers {
                let lower = name.to_ascii_lowercase();
                // The mirrored body is re-framed by hyper from the exact bytes
                // (Content-Length), so inherited framing headers are dropped.
                if lower == "content-length"
                    || lower == "transfer-encoding"
                    || lower == "connection"
                    || lower == "keep-alive"
                    || lower == "upgrade"
                    || lower == MIRROR_GUARD
                {
                    continue;
                }
                builder = builder.header(name, value);
            }
            let Ok(req) = builder.body(body) else {
                MIRROR_FAILED.fetch_add(1, Ordering::Relaxed);
                return;
            };
            let mirror_ctx = ConnectionContext {
                state,
                proxy_client,
                x_forwarded_for: xff,
                ops,
            };
            match serve_http3_request(mirror_ctx, req).await.status() {
                s if s.is_server_error() => {
                    MIRROR_FAILED.fetch_add(1, Ordering::Relaxed);
                }
                _ => {
                    MIRROR_COMPLETED.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
    }
}
