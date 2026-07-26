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
//! Shared HTTP/1.1 wire fast-path dispatch (plain TCP + TLS).
//!
//! **Wire static (PR-6d3b):** `might_use_static_wire` + `site_static` is a permanent
//! byte-prefix fast path — not a legacy debt path. Runtime does **not** call
//! `RouteIndex` or `BackendTable` here; semantic alignment to `Backend::Static` for route
//! `"site"` is enforced at compile time (PR-6d3a invariants) and in offline tests below.
#![cfg_attr(target_os = "linux", allow(dead_code))]

use crate::kernel::generation::GenerationView;
use crate::kernel::wire::{plan_wire_decision, WirePlanDecision};
use crate::lifecycle::{self};
use crate::server::drain_probes;
use crate::server::io::{self, PrefixedStream};
use crate::server::state::ServerState;
use crate::Backend;
use bytes::Bytes;
use exyonq_mod_proxy::ProxyClient;
use exyonq_module_api::proxy_wire;
use exyonq_module_api::static_wire;
use hyper::header::HeaderValue;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

const BAD_REQUEST_RESPONSE: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";

/// P15-WS5-PROBE-002: on drain reject, classify probes from request head when available.
async fn write_drain_boundary_response<S>(stream: &mut S, head: Option<&[u8]>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let owned;
    let bytes = if let Some(h) = head {
        h
    } else {
        let mut buf = [0u8; 1024];
        owned = match tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf)).await
        {
            Ok(Ok(n)) if n > 0 => buf[..n].to_vec(),
            _ => Vec::new(),
        };
        owned.as_slice()
    };
    let resp = drain_probes::drain_boundary_response(bytes);
    let _ = stream.write_all(resp).await;
    let _ = stream.shutdown().await;
}

/// Linux epoll/sync accept predicate; Tokio wire path uses `plan_wire_decision` instead.
#[cfg(target_os = "linux")]
pub(crate) fn might_use_static_wire(head: &[u8]) -> bool {
    static_wire::might_use_static_wire(head)
}

fn resolve_proxy_cluster_id(state: &ServerState, path: &str) -> Option<u32> {
    let (route_idx, _) = state.route_index.match_route_index_with_host(path, None)?;
    let Backend::Proxy { cluster_id } = state.snapshot.resolve_backend(route_idx)? else {
        return None;
    };
    Some(*cluster_id)
}

fn parse_wire_get_path(head: &[u8]) -> Option<&str> {
    if !head.starts_with(b"GET ") {
        return None;
    }
    let rest = &head[4..];
    let end = rest.iter().position(|&b| b == b' ')?;
    std::str::from_utf8(&rest[..end]).ok()
}

enum ProxyWireOutcome<S> {
    Handled,
    Fallback(S, Bytes, Bytes),
}

/// Wire proxy fast path; returns `Fallback` when Hyper must complete the request.
async fn serve_proxy_wire<S>(
    state: &ServerState,
    generation: u64,
    stream: S,
    head: Bytes,
    rest: Bytes,
    x_forwarded_for: HeaderValue,
) -> ProxyWireOutcome<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Defense: WebSocket / ineligible heads must complete via Hyper, not GET wire.
    if !proxy_wire::might_use_proxy_wire(head.as_ref()) {
        return ProxyWireOutcome::Fallback(stream, head, rest);
    }
    let Some(path_and_query) = parse_wire_get_path(head.as_ref()) else {
        return ProxyWireOutcome::Fallback(stream, head, rest);
    };
    let path = path_and_query.split('?').next().unwrap_or(path_and_query);
    let Some(cluster_id) = resolve_proxy_cluster_id(state, path) else {
        return ProxyWireOutcome::Fallback(stream, head, rest);
    };
    let xff = x_forwarded_for.to_str().unwrap_or("127.0.0.1").to_string();
    let boxed = exyonq_mod_proxy::box_wire_stream(stream);
    if let Err(err) =
        proxy_wire::serve_proxy_wire(cluster_id, generation, xff, boxed, head, rest).await
    {
        debug!(%err, "proxy wire serve ended");
    }
    ProxyWireOutcome::Handled
}

#[cfg(target_os = "linux")]
pub(crate) fn epoll_keep_alive_eligible(head: &[u8]) -> bool {
    static_wire::epoll_keep_alive_eligible(head)
}

#[cfg(target_os = "linux")]
pub(crate) fn is_sendfile_bench_head(head: &[u8]) -> bool {
    static_wire::is_sendfile_bench_head(head)
}

#[cfg(target_os = "linux")]
pub(crate) fn epoll_sendfile_eligible(head: &[u8]) -> bool {
    static_wire::epoll_sendfile_eligible(head)
}

/// Routes that must use Tokio accept/dispatch (proxy, sendfile, modules, unknown).
#[cfg(target_os = "linux")]
pub(crate) fn tokio_accept_required(head: &[u8]) -> bool {
    head.starts_with(b"GET /api/")
        || head.starts_with(b"GET /site/64k.bin ")
        || head.starts_with(b"HEAD /site/64k.bin ")
        || head.starts_with(b"GET /site/1m.bin ")
        || head.starts_with(b"HEAD /site/1m.bin ")
        || !might_use_static_wire(head)
}

pub(crate) struct WireDispatchContext {
    pub state: Arc<ServerState>,
    pub proxy_client: ProxyClient,
    pub x_forwarded_for: HeaderValue,
    pub ops: Arc<crate::lifecycle::LifecycleState>,
    /// Pinned at handoff/admit time (Phase 0); proxy wire uses this instead of re-reading.
    pub pinned_generation: u64,
}

enum WirePlan<S> {
    Proxy(S, Bytes, Bytes),
    Static(S, Bytes, Bytes),
    Hyper(S, Bytes, Bytes),
    /// Ambiguous/malformed framing: emit 400 and close — never route.
    Reject(S),
}

async fn read_wire_plan<S>(mut stream: S, state: &ServerState) -> Option<WirePlan<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Ok((head, rest)) = io::read_until_headers_async(&mut stream).await else {
        return None;
    };
    plan_wire_after_headers(stream, state, head, rest)
}

fn generation_view_from_state(state: &ServerState) -> GenerationView {
    GenerationView {
        generation: state.generation,
        modules_enabled: state.modules_enabled(),
        site_static_slot: state.site_static_slot,
    }
}

fn plan_wire_after_headers<S>(
    stream: S,
    state: &ServerState,
    head: Bytes,
    rest: Bytes,
) -> Option<WirePlan<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Fail closed: never route/static/proxy on ambiguous HTTP/1 framing.
    if !io::request_headers_safe_for_wire(head.as_ref()) {
        return Some(WirePlan::Reject(stream));
    }
    let view = generation_view_from_state(state);
    let decision = plan_wire_decision(&view, head.as_ref()).ok()?;
    Some(match decision {
        WirePlanDecision::Proxy => WirePlan::Proxy(stream, head, rest),
        WirePlanDecision::Static => WirePlan::Static(stream, head, rest),
        WirePlanDecision::Hyper => WirePlan::Hyper(stream, head, rest),
    })
}

async fn execute_wire_plan(plan: WirePlan<TcpStream>, ctx: WireDispatchContext) {
    match plan {
        WirePlan::Proxy(stream, head, rest) => {
            match serve_proxy_wire(
                &ctx.state,
                ctx.pinned_generation,
                stream,
                head,
                rest,
                ctx.x_forwarded_for.clone(),
            )
            .await
            {
                ProxyWireOutcome::Handled => {}
                ProxyWireOutcome::Fallback(stream, head, rest) => {
                    serve_hyper_fallback(stream, head, rest, ctx).await;
                }
            }
        }
        WirePlan::Static(stream, head, rest) => {
            if let Some(site_slot) = ctx.state.site_static_slot {
                let _ = serve_static_tcp(site_slot, stream, head, rest).await;
            }
        }
        WirePlan::Hyper(stream, head, rest) => {
            serve_hyper_fallback(stream, head, rest, ctx).await;
        }
        WirePlan::Reject(mut stream) => {
            let _ = stream.write_all(BAD_REQUEST_RESPONSE).await;
            let _ = stream.shutdown().await;
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) async fn dispatch_tcp_prefixed(
    mut stream: TcpStream,
    head: Bytes,
    rest: Bytes,
    ctx: WireDispatchContext,
) {
    let Ok(token) = ctx.ops.try_enter() else {
        write_drain_boundary_response(&mut stream, Some(head.as_ref())).await;
        return;
    };
    lifecycle::run_with_connection_token(token, async move {
        dispatch_tcp_prefixed_inner(stream, head, rest, ctx).await;
    })
    .await;
}

#[cfg(target_os = "linux")]
pub(crate) async fn dispatch_tcp_prefixed_inner(
    stream: TcpStream,
    head: Bytes,
    rest: Bytes,
    ctx: WireDispatchContext,
) {
    if ctx.state.modules_enabled() || ctx.ops.is_draining() {
        super::serve_hyper(
            PrefixedStream::new(io::concat_bytes(head, rest), stream),
            ctx.state,
            ctx.proxy_client,
            ctx.x_forwarded_for.clone(),
            ctx.ops,
        )
        .await;
        return;
    }

    let Some(plan) = plan_wire_after_headers(stream, &ctx.state, head, rest) else {
        return;
    };
    execute_wire_plan(plan, ctx).await;
}

async fn serve_static_tcp(
    site_slot: u32,
    stream: TcpStream,
    head: Bytes,
    rest: Bytes,
) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    if !epoll_sendfile_eligible(head.as_ref())
        && static_wire::static_tcp_blocking_pool_admission(head.as_ref())
    {
        let std_stream = stream.into_std().map_err(std::io::Error::other)?;
        match static_wire::try_spawn_blocking_static(site_slot, std_stream, head, rest) {
            static_wire::BlockingAdmission::Accepted => return Ok(()),
            static_wire::BlockingAdmission::Rejected(std_stream) => {
                static_wire::shed_blocking_admission(std_stream).await;
                return Ok(());
            }
        }
    }
    static_wire::serve_wire_site_tcp(site_slot, stream, head, rest).await
}

async fn serve_hyper_fallback<S>(stream: S, head: Bytes, rest: Bytes, ctx: WireDispatchContext)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let prefixed = PrefixedStream::new(io::concat_bytes(head, rest), stream);
    super::serve_hyper(
        prefixed,
        ctx.state,
        ctx.proxy_client,
        ctx.x_forwarded_for,
        ctx.ops,
    )
    .await;
}

pub(crate) async fn dispatch_tcp(mut stream: TcpStream, ctx: WireDispatchContext) {
    let Ok(token) = ctx.ops.try_enter() else {
        write_drain_boundary_response(&mut stream, None).await;
        return;
    };
    lifecycle::run_with_connection_token(token, async move {
        dispatch_tcp_inner(stream, ctx).await;
    })
    .await;
}

pub(crate) async fn dispatch_tcp_inner(stream: TcpStream, ctx: WireDispatchContext) {
    if ctx.state.modules_enabled() || ctx.ops.is_draining() {
        super::serve_hyper(
            stream,
            ctx.state,
            ctx.proxy_client,
            ctx.x_forwarded_for.clone(),
            ctx.ops,
        )
        .await;
        return;
    }

    let Some(plan) = read_wire_plan(stream, &ctx.state).await else {
        return;
    };
    execute_wire_plan(plan, ctx).await;
}

pub(crate) async fn dispatch_stream<S>(stream: S, ctx: WireDispatchContext)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut stream = stream;
    let Ok(token) = ctx.ops.try_enter() else {
        write_drain_boundary_response(&mut stream, None).await;
        return;
    };
    lifecycle::run_with_connection_token(token, async move {
        dispatch_stream_inner(stream, ctx).await;
    })
    .await;
}

pub(crate) async fn dispatch_stream_inner<S>(stream: S, ctx: WireDispatchContext)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    if ctx.state.modules_enabled() || ctx.ops.is_draining() {
        super::serve_hyper(
            stream,
            ctx.state,
            ctx.proxy_client,
            ctx.x_forwarded_for.clone(),
            ctx.ops,
        )
        .await;
        return;
    }

    let Some(plan) = read_wire_plan(stream, &ctx.state).await else {
        return;
    };

    match plan {
        WirePlan::Proxy(stream, head, rest) => {
            match serve_proxy_wire(
                &ctx.state,
                ctx.pinned_generation,
                stream,
                head,
                rest,
                ctx.x_forwarded_for.clone(),
            )
            .await
            {
                ProxyWireOutcome::Handled => {}
                ProxyWireOutcome::Fallback(stream, head, rest) => {
                    serve_hyper_fallback(stream, head, rest, ctx).await;
                }
            }
        }
        WirePlan::Static(stream, head, rest) => {
            serve_hyper_fallback(stream, head, rest, ctx).await;
        }
        WirePlan::Hyper(stream, head, rest) => {
            serve_hyper_fallback(stream, head, rest, ctx).await;
        }
        WirePlan::Reject(mut stream) => {
            let _ = stream.write_all(BAD_REQUEST_RESPONSE).await;
            let _ = stream.shutdown().await;
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{
        epoll_keep_alive_eligible, epoll_sendfile_eligible, is_sendfile_bench_head,
        might_use_static_wire, plan_wire_after_headers, tokio_accept_required, WirePlan,
    };
    use crate::config::{
        AppConfig, RouteConfig, RouteMatch, ServerConfig, ServerNames, UpstreamConfig,
    };
    use crate::execute_backend;
    use crate::server::state::ServerState;
    use crate::Backend;
    use bytes::Bytes;
    use exyonq_mod_proxy::build_incoming_client;
    use std::collections::HashMap;
    use std::sync::{Arc, Once};

    fn ensure_wire_hooks() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            exyonq_mod_static::install_kernel_hooks(Arc::new(
                exyonq_mod_static::StaticRuntime::new(),
            ));
            let proxy_rt = Arc::new(exyonq_mod_proxy::ProxyRuntime::new());
            exyonq_mod_proxy::install_kernel_hooks(proxy_rt);
        });
    }
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    /// Wire static fast-path heads (PR-6d3a/6d3b): bench assets, health, metrics.
    const WIRE_STATIC_HEADS: &[&[u8]] = &[
        b"GET /health HTTP/1.1\r\n",
        b"HEAD /health HTTP/1.1\r\n",
        b"GET /site/1k.bin HTTP/1.1\r\n",
        b"HEAD /site/1k.bin HTTP/1.1\r\n",
        b"GET /site/64k.bin HTTP/1.1\r\n",
        b"HEAD /site/64k.bin HTTP/1.1\r\n",
        b"GET /site/1m.bin HTTP/1.1\r\n",
        b"HEAD /site/1m.bin HTTP/1.1\r\n",
        b"GET /site/routes/route050.bin HTTP/1.1\r\n",
        b"HEAD /site/routes/route050.bin HTTP/1.1\r\n",
        b"GET /metrics HTTP/1.1\r\n",
        b"GET /metrics\r\n",
    ];

    #[test]
    fn wire_static_predicate_paths_are_site_or_health_or_metrics() {
        ensure_wire_hooks();
        for head in WIRE_STATIC_HEADS {
            assert!(
                might_use_static_wire(head),
                "expected wire static predicate: {}",
                String::from_utf8_lossy(head)
            );
            let path = wire_path_prefix(head);
            assert!(
                path.starts_with("/site/") || path == "/health" || path.starts_with("/metrics"),
                "wire static head must target site bench, health, or metrics: {path}"
            );
        }
    }

    #[test]
    fn wire_static_predicates_exclude_proxy_paths() {
        ensure_wire_hooks();
        assert!(!might_use_static_wire(b"GET /api/health HTTP/1.1\r\n"));
        assert!(!might_use_static_wire(b"GET /api/ HTTP/1.1\r\n"));
        assert!(!might_use_static_wire(
            b"GET /assets/page.html HTTP/1.1\r\n"
        ));
    }

    fn bench_site_config(root: Option<std::path::PathBuf>) -> AppConfig {
        let site = RouteConfig {
            name: "site".into(),
            r#match: RouteMatch {
                path: "/site".into(),
                host: None,
            },
            upstream: None,
            root,
            index: Some("index.html".into()),
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        };
        let api = RouteConfig {
            name: "api".into(),
            r#match: RouteMatch {
                path: "/api".into(),
                host: None,
            },
            upstream: Some("backend".into()),
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        };
        let mut upstreams = HashMap::new();
        upstreams.insert(
            "backend".into(),
            UpstreamConfig {
                name: "backend".into(),
                target: "http://127.0.0.1:9000".into(),
                timeout_ms: 5000,
            },
        );
        AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: "127.0.0.1:8080".into(),
                server_name: ServerNames::None,
                routes: vec!["site".into(), "api".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![site, api],
            upstreams,
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        }
    }

    async fn state_for(
        config: AppConfig,
    ) -> (Arc<ServerState>, execute_backend::StaticDispatchTestGuard) {
        let runtime = Arc::new(exyonq_mod_static::StaticRuntime::new());
        let service: Arc<dyn exyonq_module_api::static_dispatch::StaticDispatchService> =
            runtime.clone();
        let static_guard = execute_backend::StaticDispatchTestGuard::install(service);
        exyonq_mod_static::install_kernel_hooks(runtime);
        ensure_wire_hooks();
        let proxy_client = build_incoming_client();
        let state = ServerState::new_with_generation(1, config, proxy_client)
            .await
            .expect("server state");
        (state, static_guard)
    }

    /// `/site/*` wire heads must match route `"site"` and resolve to `Backend::Static` offline.
    #[tokio::test]
    async fn wire_static_site_heads_align_with_site_route_backend() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _static_guard) =
            state_for(bench_site_config(Some(dir.path().to_path_buf()))).await;
        assert!(state.site_static_slot.is_some());

        for head in WIRE_STATIC_HEADS {
            if !might_use_static_wire(head) {
                continue;
            }
            let path = wire_path_prefix(head);
            if path == "/health" || path.starts_with("/metrics") {
                continue;
            }
            assert!(
                path.starts_with("/site/"),
                "expected /site bench path, got {path}"
            );
            let (route_idx, route) = state
                .route_index
                .match_route_index_with_host(path, None)
                .unwrap_or_else(|| panic!("RouteIndex miss for wire static path {path}"));
            assert_eq!(route.name, "site");
            let backend = state
                .snapshot
                .resolve_backend(route_idx)
                .unwrap_or_else(|| panic!("no backend for site route idx {route_idx}"));
            assert!(matches!(backend, Backend::Static { .. }));
            let via_table = state
                .snapshot
                .static_slot_for_route_backend(route_idx)
                .expect("static slot for site route");
            assert_eq!(via_table.route_name, "site");
            assert_eq!(state.site_static_slot, Some(0));
        }
    }

    /// Health and metrics are wire/service paths — not normal static route matches.
    #[tokio::test]
    async fn wire_static_health_and_metrics_are_service_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _static_guard) =
            state_for(bench_site_config(Some(dir.path().to_path_buf()))).await;

        for head in [
            b"GET /health HTTP/1.1\r\n".as_slice(),
            b"GET /metrics HTTP/1.1\r\n".as_slice(),
        ] {
            assert!(might_use_static_wire(head));
            let path = wire_path_prefix(head);
            assert!(
                state
                    .route_index
                    .match_route_index_with_host(path, None)
                    .is_none(),
                "service path {path} must not match IR site/api routes"
            );
        }
    }

    struct PlanStream;

    impl AsyncRead for PlanStream {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for PlanStream {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<Result<usize, std::io::Error>> {
            std::task::Poll::Ready(Ok(0))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn wire_static_plan_requires_site_static() {
        let dir = tempfile::tempdir().unwrap();
        let (with_site, _static_guard) =
            state_for(bench_site_config(Some(dir.path().to_path_buf()))).await;
        let head = Bytes::from_static(b"GET /site/1k.bin HTTP/1.1\r\n");
        let plan = plan_wire_after_headers(PlanStream, &with_site, head.clone(), Bytes::new())
            .expect("plan");
        assert!(matches!(plan, WirePlan::Static(..)));

        let (api_only, _static_guard2) = state_for(bench_site_config(None)).await;
        assert!(api_only.site_static_slot.is_none());
        let plan =
            plan_wire_after_headers(PlanStream, &api_only, head, Bytes::new()).expect("plan");
        assert!(matches!(plan, WirePlan::Hyper(..)));
    }

    fn wire_path_prefix(head: &[u8]) -> &str {
        let s = std::str::from_utf8(head).expect("ascii head");
        let after_method = s.split_once(' ').map(|(_, rest)| rest).unwrap_or(s);
        after_method
            .split_once(' ')
            .map(|(path, _)| path)
            .unwrap_or(after_method)
    }

    #[test]
    fn blocking_pool_facade_matches_module_hooks() {
        ensure_wire_hooks();
        let heads = [
            b"GET /site/1k.bin HTTP/1.1\r\n".as_slice(),
            b"HEAD /site/1k.bin HTTP/1.1\r\n",
            b"GET /site/64k.bin HTTP/1.1\r\n",
            b"GET /site/1m.bin HTTP/1.1\r\n",
            b"GET /api/ HTTP/1.1\r\n",
            b"GET /assets/x HTTP/1.1\r\n",
        ];
        for head in heads {
            assert_eq!(
                exyonq_module_api::static_wire::static_use_blocking_pool(head),
                exyonq_module_api::static_wire::static_wire_use_blocking_pool(head)
                    || exyonq_module_api::static_wire::static_sendfile_use_blocking_pool(head),
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }

    #[test]
    fn tcp_admission_facade_matches_module_hooks() {
        ensure_wire_hooks();
        for head in [
            b"GET /site/1k.bin HTTP/1.1\r\n".as_slice(),
            b"GET /site/1m.bin HTTP/1.1\r\n",
            b"GET /api/ HTTP/1.1\r\n",
        ] {
            assert_eq!(
                exyonq_module_api::static_wire::static_tcp_blocking_pool_admission(head),
                exyonq_module_api::static_wire::static_use_blocking_pool(head)
                    || exyonq_module_api::static_wire::static_one_m_sendfile_use_blocking(head),
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }

    /// Core call-site contract (KD2D.1): sync bench vs TCP admission stay intentionally distinct.
    #[test]
    fn core_blocking_path_semantics_use_module_api_facades() {
        ensure_wire_hooks();
        let heads = [
            b"GET /site/1k.bin HTTP/1.1\r\n".as_slice(),
            b"HEAD /site/1k.bin HTTP/1.1\r\n",
            b"GET /site/64k.bin HTTP/1.1\r\n",
            b"GET /site/1m.bin HTTP/1.1\r\n",
            b"GET /api/ HTTP/1.1\r\n",
        ];
        for head in heads {
            let sync = exyonq_module_api::static_wire::static_use_blocking_pool(head);
            let tcp = exyonq_module_api::static_wire::static_tcp_blocking_pool_admission(head);
            let one_m = exyonq_module_api::static_wire::static_one_m_sendfile_use_blocking(head);
            assert_eq!(tcp, sync || one_m, "head={}", String::from_utf8_lossy(head));
            let sendfile_ok = exyonq_module_api::static_wire::epoll_sendfile_eligible(head);
            if sendfile_ok {
                // When sendfile FSM owns the path, blocking admission is bypassed at runtime.
                let _ = sendfile_ok;
            }
        }
    }

    #[test]
    fn sendfile_bench_head_predicate_matches_p2_p3() {
        ensure_wire_hooks();
        assert!(is_sendfile_bench_head(b"GET /site/64k.bin HTTP/1.1\r\n"));
        assert!(is_sendfile_bench_head(b"HEAD /site/64k.bin HTTP/1.1\r\n"));
        assert!(is_sendfile_bench_head(b"GET /site/1m.bin HTTP/1.1\r\n"));
        assert!(is_sendfile_bench_head(b"HEAD /site/1m.bin HTTP/1.1\r\n"));
        assert!(!is_sendfile_bench_head(b"GET /site/1k.bin HTTP/1.1\r\n"));
        assert!(!is_sendfile_bench_head(b"GET /health HTTP/1.1\r\n"));
    }

    #[test]
    fn sendfile_eligibility_off_by_default() {
        ensure_wire_hooks();
        // No flags set in the test env → the gated FSM path must be inactive, so P2/P3
        // keep their default (blocking) routing. Opt-in only.
        assert!(!epoll_sendfile_eligible(b"GET /site/64k.bin HTTP/1.1\r\n"));
        assert!(!epoll_sendfile_eligible(b"GET /site/1m.bin HTTP/1.1\r\n"));
    }

    #[test]
    fn epoll_eligible_p1_p7_health() {
        ensure_wire_hooks();
        assert!(epoll_keep_alive_eligible(b"GET /health HTTP/1.1\r\n"));
        assert!(epoll_keep_alive_eligible(b"GET /site/1k.bin HTTP/1.1\r\n"));
        assert!(epoll_keep_alive_eligible(
            b"GET /site/routes/route050.bin HTTP/1.1\r\n"
        ));
    }

    #[test]
    fn epoll_not_eligible_proxy_sendfile() {
        ensure_wire_hooks();
        assert!(!epoll_keep_alive_eligible(b"GET /api/ HTTP/1.1\r\n"));
        assert!(!epoll_keep_alive_eligible(b"GET /api/stream HTTP/1.1\r\n"));
        assert!(!epoll_keep_alive_eligible(
            b"GET /site/64k.bin HTTP/1.1\r\n"
        ));
        assert!(!epoll_keep_alive_eligible(b"GET /site/1m.bin HTTP/1.1\r\n"));
    }

    #[test]
    fn tokio_required_proxy_sendfile_and_unknown() {
        ensure_wire_hooks();
        assert!(tokio_accept_required(b"GET /api/ HTTP/1.1\r\n"));
        assert!(tokio_accept_required(b"GET /api/stream HTTP/1.1\r\n"));
        assert!(tokio_accept_required(b"GET /site/64k.bin HTTP/1.1\r\n"));
        assert!(tokio_accept_required(b"GET /site/1m.bin HTTP/1.1\r\n"));
        assert!(tokio_accept_required(b"POST /site/1k.bin HTTP/1.1\r\n"));
    }

    #[test]
    fn tokio_not_required_p1_p7_health() {
        ensure_wire_hooks();
        assert!(!tokio_accept_required(b"GET /health HTTP/1.1\r\n"));
        assert!(!tokio_accept_required(b"GET /site/1k.bin HTTP/1.1\r\n"));
        assert!(!tokio_accept_required(
            b"GET /site/routes/route001.bin HTTP/1.1\r\n"
        ));
    }
}

#[cfg(test)]
mod ws_tunnel_tests {
    use super::{dispatch_tcp, WireDispatchContext};
    use crate::config::{
        AppConfig, RouteConfig, RouteMatch, ServerConfig, ServerNames, UpstreamConfig,
    };
    use crate::execute_backend::StaticDispatchTestGuard;
    use crate::server::state::ServerState;
    use exyonq_mod_proxy::{build_incoming_client, install_kernel_hooks, ProxyRuntime};
    use exyonq_module_api::static_dispatch::StaticDispatchService;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn ws_frame_text(text: &str) -> Vec<u8> {
        let payload = text.as_bytes();
        let mut frame = vec![0x81, 0x80 | payload.len() as u8];
        let mask = [0x11, 0x22, 0x33, 0x44];
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ mask[i % 4])
                .collect::<Vec<_>>(),
        );
        frame
    }

    fn parse_ws_text(payload: &[u8]) -> Option<String> {
        if payload.len() < 2 {
            return None;
        }
        let masked = (payload[1] & 0x80) != 0;
        let mut idx = 2usize;
        let mut length = (payload[1] & 0x7f) as usize;
        if length == 126 {
            length = u16::from_be_bytes([payload[2], payload[3]]) as usize;
            idx = 4;
        }
        let mask = if masked {
            let m = &payload[idx..idx + 4];
            idx += 4;
            Some(m)
        } else {
            None
        };
        let data = &payload[idx..idx + length];
        let unmasked = if let Some(m) = mask {
            data.iter()
                .enumerate()
                .map(|(i, b)| b ^ m[i % 4])
                .collect::<Vec<_>>()
        } else {
            data.to_vec()
        };
        String::from_utf8(unmasked).ok()
    }

    async fn spawn_ws_echo_upstream() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    continue;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let key = text.lines().find_map(|line| {
                        let (k, v) = line.split_once(':')?;
                        k.trim()
                            .eq_ignore_ascii_case("sec-websocket-key")
                            .then_some(v.trim().to_string())
                    });
                    let Some(key) = key else {
                        return;
                    };
                    let accept_val = {
                        use base64::Engine;
                        use sha1::{Digest, Sha1};
                        let mut hasher = Sha1::new();
                        hasher.update(key.as_bytes());
                        hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
                        base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
                    };
                    let resp = format!(
                        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept_val}\r\n\r\n"
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    loop {
                        let n = stream.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        if let Some(msg) = parse_ws_text(&buf[..n]) {
                            let mut out = vec![0x81, msg.len() as u8];
                            out.extend_from_slice(msg.as_bytes());
                            let _ = stream.write_all(&out).await;
                        }
                    }
                });
            }
        });
        port
    }

    fn minimal_proxy_config(listen_port: u16, upstream_port: u16) -> AppConfig {
        let api = RouteConfig {
            name: "api".into(),
            r#match: RouteMatch {
                path: "/api".into(),
                host: None,
            },
            upstream: Some("backend".into()),
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        };
        let mut upstreams = HashMap::new();
        upstreams.insert(
            "backend".into(),
            UpstreamConfig {
                name: "backend".into(),
                target: format!("http://127.0.0.1:{upstream_port}"),
                timeout_ms: 5000,
            },
        );
        AppConfig {
            config_version: 1,
            includes: Vec::new(),
            servers: vec![ServerConfig {
                listen: format!("127.0.0.1:{listen_port}"),
                server_name: ServerNames::None,
                routes: vec!["api".into()],
                tls: None,
                http3_listen: None,
            }],
            routes: vec![api],
            upstreams,
            pools_fcgi: HashMap::new(),
            cache_policies: HashMap::new(),
            modules: Default::default(),
            static_section: Default::default(),
            full_page_cache: Default::default(),
            http3: Default::default(),
        }
    }

    struct WireTunnelTestHooks {
        _gate: tokio::sync::MutexGuard<'static, ()>,
        _static: StaticDispatchTestGuard,
        _proxy: crate::execute_backend::ProxyDispatchTestGuard,
    }

    static WS_TUNNEL_TEST_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn install_cli_hooks() -> WireTunnelTestHooks {
        let gate = WS_TUNNEL_TEST_GATE.lock().await;
        let static_runtime = Arc::new(exyonq_mod_static::StaticRuntime::new());
        let static_service: Arc<dyn StaticDispatchService> = static_runtime.clone();
        crate::execute_backend::clear_global_static_dispatch_for_register_once_test();
        crate::register_static_dispatch_service(static_service).expect("static register");
        exyonq_mod_static::install_kernel_hooks(Arc::clone(&static_runtime));
        let proxy_runtime = Arc::new(ProxyRuntime::new());
        install_kernel_hooks(Arc::clone(&proxy_runtime));
        let proxy_service: Arc<dyn exyonq_module_api::proxy_dispatch::ProxyDispatchService> =
            proxy_runtime.clone();
        crate::execute_backend::clear_global_proxy_dispatch_for_register_once_test();
        crate::register_proxy_dispatch_service(Arc::clone(&proxy_service)).expect("proxy register");
        WireTunnelTestHooks {
            _gate: gate,
            _static: StaticDispatchTestGuard::install(
                static_runtime as Arc<dyn StaticDispatchService>,
            ),
            _proxy: crate::execute_backend::ProxyDispatchTestGuard::install(proxy_service),
        }
    }

    async fn install_hooks() -> WireTunnelTestHooks {
        let gate = WS_TUNNEL_TEST_GATE.lock().await;
        let static_runtime = Arc::new(exyonq_mod_static::StaticRuntime::new());
        exyonq_mod_static::install_kernel_hooks(Arc::clone(&static_runtime));
        let static_service: Arc<dyn StaticDispatchService> = static_runtime.clone();
        crate::execute_backend::clear_global_static_dispatch_for_register_once_test();
        let _ = crate::register_static_dispatch_service(static_service);
        let proxy_runtime = Arc::new(ProxyRuntime::new());
        install_kernel_hooks(Arc::clone(&proxy_runtime));
        let proxy_service: Arc<dyn exyonq_module_api::proxy_dispatch::ProxyDispatchService> =
            proxy_runtime.clone();
        crate::execute_backend::clear_global_proxy_dispatch_for_register_once_test();
        let _ = crate::register_proxy_dispatch_service(Arc::clone(&proxy_service));
        WireTunnelTestHooks {
            _gate: gate,
            _static: StaticDispatchTestGuard::install(
                static_runtime as Arc<dyn StaticDispatchService>,
            ),
            _proxy: crate::execute_backend::ProxyDispatchTestGuard::install(proxy_service),
        }
    }

    async fn ws_echo_through_dispatch(coalesce_request: bool) -> String {
        let _hooks = install_hooks().await;
        let upstream_port = spawn_ws_echo_upstream().await;
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_port = proxy_listener.local_addr().unwrap().port();
        let proxy_client = build_incoming_client();
        let config = minimal_proxy_config(listen_port, upstream_port);
        let state = ServerState::new_with_generation(1, config, proxy_client.clone())
            .await
            .expect("server state");
        let ops = Arc::new(crate::lifecycle::LifecycleState::new());

        tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = proxy_listener.accept().await else {
                    continue;
                };
                let ctx = WireDispatchContext {
                    state: Arc::clone(&state),
                    proxy_client: proxy_client.clone(),
                    x_forwarded_for: hyper::header::HeaderValue::from_str(&peer.ip().to_string())
                        .unwrap_or_else(|_| hyper::header::HeaderValue::from_static("127.0.0.1")),
                    ops: Arc::clone(&ops),
                    pinned_generation: state.generation,
                };
                tokio::spawn(async move {
                    dispatch_tcp(stream, ctx).await;
                });
            }
        });

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let mut req = format!(
            "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .into_bytes();
        if coalesce_request {
            req.extend(ws_frame_text("ping-kd34"));
        }
        stream.write_all(&req).await.unwrap();

        let mut buf = vec![0u8; 4096];
        let mut resp = Vec::new();
        while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0, "unexpected eof before 101");
            resp.extend_from_slice(&buf[..n]);
        }
        assert!(
            String::from_utf8_lossy(&resp).contains("101"),
            "missing 101: {}",
            String::from_utf8_lossy(&resp)
        );

        if !coalesce_request {
            stream.write_all(&ws_frame_text("ping-kd34")).await.unwrap();
        }

        let body_start = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(resp.len());
        let mut pending = resp[body_start..].to_vec();
        while pending.len() < 2 {
            let n = stream.read(&mut buf).await.unwrap();
            pending.extend_from_slice(&buf[..n]);
        }
        let hdr = &pending[..2];
        let mut rest = pending[2..].to_vec();
        let length = (hdr[1] & 0x7f) as usize;
        while rest.len() < length {
            let n = stream.read(&mut buf).await.unwrap();
            rest.extend_from_slice(&buf[..n]);
        }
        let mut frame = hdr.to_vec();
        frame.extend_from_slice(&rest[..length]);
        parse_ws_text(&frame).expect("echo frame")
    }

    #[tokio::test]
    async fn wire_dispatch_ws_echo_exact_bytes() {
        let echo = ws_echo_through_dispatch(false).await;
        assert_eq!(echo, "ping-kd34");
    }

    #[tokio::test]
    async fn wire_dispatch_ws_echo_coalesced_request() {
        let echo = ws_echo_through_dispatch(true).await;
        assert_eq!(echo, "ping-kd34");
    }

    async fn ws_echo_through_run_on(coalesce_request: bool) -> String {
        let _hooks = install_hooks().await;
        let upstream_port = spawn_ws_echo_upstream().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = minimal_proxy_config(listen_port, upstream_port);
        let listen_addr: std::net::SocketAddr = format!("127.0.0.1:{listen_port}").parse().unwrap();
        tokio::spawn(async move {
            let _ = crate::server::run_on(listen_addr, config).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let mut req = format!(
            "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .into_bytes();
        if coalesce_request {
            req.extend(ws_frame_text("ping-kd34"));
        }
        stream.write_all(&req).await.unwrap();

        let mut buf = vec![0u8; 4096];
        let mut resp = Vec::new();
        while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0);
            resp.extend_from_slice(&buf[..n]);
        }
        assert!(String::from_utf8_lossy(&resp).contains("101"));

        if !coalesce_request {
            stream.write_all(&ws_frame_text("ping-kd34")).await.unwrap();
        }

        let body_start = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(resp.len());
        let mut pending = resp[body_start..].to_vec();
        while pending.len() < 2 {
            let n = stream.read(&mut buf).await.unwrap();
            pending.extend_from_slice(&buf[..n]);
        }
        let hdr = &pending[..2];
        let mut rest = pending[2..].to_vec();
        let length = (hdr[1] & 0x7f) as usize;
        while rest.len() < length {
            let n = stream.read(&mut buf).await.unwrap();
            rest.extend_from_slice(&buf[..n]);
        }
        let mut frame = hdr.to_vec();
        frame.extend_from_slice(&rest[..length]);
        parse_ws_text(&frame).expect("echo frame")
    }

    #[tokio::test]
    async fn run_on_ws_echo_with_cli_registration() {
        let _hooks = install_cli_hooks().await;
        let upstream_port = spawn_ws_echo_upstream().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = minimal_proxy_config(listen_port, upstream_port);
        tokio::spawn(async move {
            let _ = crate::server::run(config).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        stream
            .write_all(format!(
                "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            ).as_bytes())
            .await
            .unwrap();
        let mut buf = vec![0u8; 4096];
        let mut resp = Vec::new();
        while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            resp.extend_from_slice(&buf[..n]);
        }
        stream.write_all(&ws_frame_text("ping-kd34")).await.unwrap();
        let body_start = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(resp.len());
        let mut pending = resp[body_start..].to_vec();
        while pending.len() < 2 {
            let n = stream.read(&mut buf).await.unwrap();
            pending.extend_from_slice(&buf[..n]);
        }
        let hdr = &pending[..2];
        let mut rest = pending[2..].to_vec();
        let length = (hdr[1] & 0x7f) as usize;
        while rest.len() < length {
            let n = stream.read(&mut buf).await.unwrap();
            rest.extend_from_slice(&buf[..n]);
        }
        let mut frame = hdr.to_vec();
        frame.extend_from_slice(&rest[..length]);
        let echo = parse_ws_text(&frame).expect("echo");
        assert_eq!(echo, "ping-kd34");
    }

    #[tokio::test]
    async fn run_on_ws_echo_exact_bytes() {
        let echo = ws_echo_through_run_on(false).await;
        assert_eq!(echo, "ping-kd34");
    }

    #[tokio::test]
    async fn run_on_ws_echo_after_health_get() {
        let _hooks = install_hooks().await;
        let upstream_port = spawn_ws_echo_upstream().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = minimal_proxy_config(listen_port, upstream_port);
        let listen_addr: std::net::SocketAddr = format!("127.0.0.1:{listen_port}").parse().unwrap();
        tokio::spawn(async move {
            let _ = crate::server::run_on(listen_addr, config).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let mut health = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        health
            .write_all(
                format!(
                    "GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut buf = [0u8; 512];
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(2), health.read(&mut buf)).await;
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", listen_port))
            .await
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        stream
            .write_all(format!(
                "GET /api/ws-echo HTTP/1.1\r\nHost: 127.0.0.1:{listen_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            ).as_bytes())
            .await
            .unwrap();
        let mut buf = vec![0u8; 4096];
        let mut resp = Vec::new();
        while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0, "unexpected eof before ws upgrade response");
            resp.extend_from_slice(&buf[..n]);
        }
        assert!(
            String::from_utf8_lossy(&resp).contains("101"),
            "missing 101: {}",
            String::from_utf8_lossy(&resp)
        );
        stream.write_all(&ws_frame_text("ping-kd34")).await.unwrap();
        let body_start = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(resp.len());
        let mut pending = resp[body_start..].to_vec();
        while pending.len() < 2 {
            let n = stream.read(&mut buf).await.unwrap();
            pending.extend_from_slice(&buf[..n]);
        }
        let hdr = &pending[..2];
        let mut rest = pending[2..].to_vec();
        let length = (hdr[1] & 0x7f) as usize;
        while rest.len() < length {
            let n = stream.read(&mut buf).await.unwrap();
            rest.extend_from_slice(&buf[..n]);
        }
        let mut frame = hdr.to_vec();
        frame.extend_from_slice(&rest[..length]);
        let echo = parse_ws_text(&frame).expect("echo");
        assert_eq!(echo, "ping-kd34");
    }
}
