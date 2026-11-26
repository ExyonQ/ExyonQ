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
//! HTTP server lifecycle.

pub(crate) mod accepted_connection;
#[cfg(target_os = "linux")]
pub(crate) mod conn_pool;
pub(crate) mod connection_errors;
#[cfg(target_os = "linux")]
pub(crate) mod connection_executor;
pub(crate) mod drain_probes;
pub mod handler;
#[cfg(target_os = "linux")]
mod hyper_handoff;
pub(crate) mod io;
pub mod proxy_protocol;
pub(crate) mod proxy_request_body;
#[cfg(target_os = "linux")]
pub use hyper_handoff::{spawn_hyper_handoff, xff_from_peer};
pub mod epoll_start;
pub mod io_uring_start;
#[cfg(target_os = "linux")]
pub mod os_worker_guard;
pub mod runtime_parallelism;
#[cfg(target_os = "linux")]
pub mod sdp_start;
pub mod state;
pub mod sync_accept_start;
mod wire_dispatch;

pub use wire_dispatch::install_static_wire_access_bridge;
pub use wire_dispatch::wire_reject_response_write_errors;
pub use wire_dispatch::wire_waf_reject_response_write_errors;

#[cfg(all(test, target_os = "linux"))]
mod ps2_iu2_tests;
#[cfg(all(test, target_os = "linux"))]
mod ps3a_epoll_fsm_twin_tests;

pub use epoll_start::{epoll_listen_env_enabled, epoll_static_env_enabled};
#[cfg(target_os = "linux")]
pub use epoll_start::{
    register_epoll_keepalive_enqueue, register_epoll_keepalive_prepare,
    register_epoll_keepalive_signal_stop, register_epoll_keepalive_stop,
    register_epoll_listen_start,
};
pub use io::header_read_timeout;
#[cfg(any(test, feature = "test-utils"))]
#[doc(hidden)]
pub use io::reset_header_read_timeout_cache_for_tests;
pub use io_uring_start::io_uring_env_enabled;
#[cfg(target_os = "linux")]
pub use io_uring_start::register_io_uring_start;
#[cfg(target_os = "linux")]
pub use os_worker_guard::OsWorkerGuard;
pub use runtime_parallelism::{
    default_runtime_parallelism, resolve_accept_workers, resolve_accept_workers_with,
    resolve_epoll_pool_threads, resolve_tokio_worker_threads, MAX_WORKERS, MIN_WORKERS,
};
#[cfg(target_os = "linux")]
pub use sync_accept_start::register_sync_accept_start;
pub use sync_accept_start::sync_accept_env_enabled;

#[cfg(test)]
mod ps3a_f3_i0_goldens;

use crate::certificate_publication_port::CoreCertificatePublicationPort;
use crate::config::AppConfig;
use crate::discovery_overlay;
use crate::http3_runtime_registry;
use crate::kernel_control_port::{self, CoreKernelControlPort};
use crate::lifecycle::{self, LifecycleState};
use crate::reload;
use exyonq_mod_proxy::{build_incoming_client, ProxyClient};
use exyonq_mod_tls::{install_rustls_provider, SharedTlsAcceptor, TlsSessionCache, TlsSettings};
use exyonq_module_api::acme_integration::acme_integration_service;
use exyonq_module_api::kernel_control::control_plane_service;
use exyonq_module_api::reload_runtime::reload_runtime_service;
use handler::{serve_connection, ConnectionContext};
use hyper::header::{HeaderValue, CONNECTION};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use state::ServerState;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket};
use tracing::{info, warn};
use wire_dispatch::{dispatch_stream, dispatch_tcp, WireDispatchContext};

/// Run the configured HTTP listener until shutdown.
pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    install_rustls_provider();
    let listen = config.primary_listen_addr()?;
    run_on(listen, config).await
}

async fn bind_tuned(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = if addr.is_ipv6() {
        TcpSocket::new_v6()?
    } else {
        TcpSocket::new_v4()?
    };
    socket.set_reuseaddr(true)?;
    #[cfg(all(unix, not(target_os = "solaris")))]
    {
        let _ = socket.set_reuseport(true);
    }
    socket.set_nodelay(true)?;
    socket.bind(addr)?;
    socket.listen(4096)
}

/// Wait until `addr` accepts TCP connections. ACME HTTP-01 needs this
/// process to answer `/.well-known/acme-challenge/<token>`, so the bootstrap
/// must not start the order before its own listeners are up.
async fn wait_for_listener(addr: SocketAddr, timeout: Duration) {
    let probe = if addr.ip().is_unspecified() {
        match addr {
            SocketAddr::V4(_) => SocketAddr::from(([127, 0, 0, 1], addr.port())),
            SocketAddr::V6(_) => SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], addr.port())),
        }
    } else {
        addr
    };
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(probe).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    warn!(%addr, "listener did not accept before the ACME bootstrap deadline");
}

fn accept_workers(tuning: &exyonq_config_ir::TuningConfig) -> usize {
    crate::server::runtime_parallelism::resolve_accept_workers_with(tuning.accept_workers)
}

// The guard is a distinct ownership parameter: it must be moved into the
// connection task, not reconstructed from shared state.
#[allow(clippy::too_many_arguments)]
async fn serve_hyper<S>(
    mut stream: S,
    shared: reload::SharedServerState,
    proxy_client: ProxyClient,
    x_forwarded_for: HeaderValue,
    ops: Arc<LifecycleState>,
    scheme: &'static str,
    local_port: Option<u16>,
    client_guard: Option<crate::client_throttle::ClientGuard>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Ok(_conn_token) = lifecycle::admit_connection(&ops) else {
        // KF-P16-014: TLS/Hyper admit reject must still classify probes (same as wire).
        let mut buf = [0u8; 1024];
        let head =
            match tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => &buf[..n],
                _ => &buf[..0],
            };
        let resp = drain_probes::drain_boundary_response(head);
        let _ = stream.write_all(resp).await;
        let _ = stream.shutdown().await;
        return;
    };
    // OL-03: one slot per TCP/QUIC connection, held across every request on
    // this transport. A refused connection is closed without a response.
    let _client_guard = match client_guard {
        Some(guard) => Some(guard),
        None => {
            let state = reload::read_state(&shared);
            let ip = handler::client_ip_from_xff(Some(&x_forwarded_for));
            match crate::client_throttle::acquire_connection(
                &state.config.modules.ratelimit.client,
                ip,
            ) {
                Ok(guard) => guard,
                Err(decision) => {
                    tracing::debug!(?decision, %ip, "per-client connection limit refused connection");
                    return;
                }
            }
        }
    };
    // NG-21: the committed header deadline also covers HTTP/2. Hyper's H2
    // builder has no read timeout, so the deadline is armed here and disarmed
    // by the first request the connection delivers.
    let headers_delivered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let io = TokioIo::new(io::HeaderDeadlineIo::new(
        stream,
        io::header_read_timeout(),
        Arc::clone(&headers_delivered),
    ));
    let bad_req_ip = handler::client_ip_from_xff(Some(&x_forwarded_for));
    let bad_req_cfg = reload::read_state(&shared)
        .config
        .modules
        .ratelimit
        .client
        .clone();
    let ops_for_service = Arc::clone(&ops);
    // Cap057 LA-CAP057-002: refresh live SharedServerState per request (mirror H3).
    // Accept-time Arc pin made FPC HITs survive reload disable / live purge on keep-alive.
    let service = hyper::service::service_fn(move |req| {
        let headers_delivered = Arc::clone(&headers_delivered);
        let conn_state = reload::read_state(&shared);
        let proxy_client = proxy_client.clone();
        let x_forwarded_for = x_forwarded_for.clone();
        let ops = Arc::clone(&ops_for_service);
        let client_wants_close = req
            .headers()
            .get(CONNECTION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("close"));
        async move {
            headers_delivered.store(true, std::sync::atomic::Ordering::Release);
            let mut response = serve_connection(
                ConnectionContext {
                    state: conn_state,
                    scheme,
                    local_port,
                    proxy_client,
                    x_forwarded_for,
                    ops,
                },
                req,
            )
            .await?;
            if client_wants_close {
                response
                    .headers_mut()
                    .insert(CONNECTION, HeaderValue::from_static("close"));
            }
            Ok::<_, Infallible>(response)
        }
    });

    let mut builder = ServerBuilder::new(TokioExecutor::new());
    {
        // NG-21: a connection Hyper owns after the wire hands it over must obey
        // the same committed geometry as the pre-parser. The header deadline is
        // also the keep-alive idle deadline for the next request, and the read
        // buffer is bounded by the same header cap instead of Hyper's default.
        let mut http1 = builder.http1();
        http1.keep_alive(true);
        http1.header_read_timeout(io::header_read_timeout());
        http1.max_buf_size(io::max_header_cap());
        http1.timer(hyper_util::rt::TokioTimer::new());
    }
    {
        // NG-21: the same committed geometry bounds HTTP/2 request headers. The
        // `h2` layer enforces this cap while decoding HEADERS, so an oversized
        // block is refused before the service sees a request, exactly as the H1
        // pre-parser refuses one: no response is served.
        let mut http2 = builder.http2();
        http2.max_header_list_size(io::max_header_cap() as u32);
        http2.timer(hyper_util::rt::TokioTimer::new());
        // NG-21 remainder: the IO deadline above only covers the first head.
        // After that, Hyper's h2 builder has no header or idle timeout, so a
        // peer that goes silent on an open connection would hold it forever.
        // The committed keep-alive deadline also arms h2's own PING liveness
        // probe: a peer that stops answering is closed, while a healthy client
        // (any h2 stack answers PING) keeps its connection and streams.
        let liveness = io::header_read_timeout();
        http2.keep_alive_interval(liveness);
        http2.keep_alive_timeout(liveness);
    }
    if let Err(err) = builder.serve_connection_with_upgrades(io, service).await {
        // OL-03 blockBadReq: Hyper rejects malformed request lines before the
        // service is invoked. Count protocol-parse failures (never incomplete
        // client disconnects) toward the per-IP ban.
        let parse_error = err
            .downcast_ref::<hyper::Error>()
            .is_some_and(hyper::Error::is_parse);
        if parse_error {
            crate::client_throttle::note_bad_request(&bad_req_cfg, bad_req_ip);
        }
        warn!(%err, "connection error");
    }
}

fn wire_dispatch_ctx(
    shared: reload::SharedServerState,
    proxy_client: ProxyClient,
    x_forwarded_for: HeaderValue,
    ops: Arc<LifecycleState>,
    scheme: &'static str,
    local_port: Option<u16>,
) -> WireDispatchContext {
    let state = reload::read_state(&shared);
    let pinned_generation = state.generation;
    WireDispatchContext {
        shared,
        state,
        proxy_client,
        x_forwarded_for,
        ops,
        pinned_generation,
        scheme,
        local_port,
        client_guard: None,
    }
}

async fn accept_loop(
    listener: TcpListener,
    state: reload::SharedServerState,
    proxy_client: ProxyClient,
    tls_acceptor: Option<SharedTlsAcceptor>,
    ops: Arc<LifecycleState>,
    proxy_protocol_config: Option<exyonq_config_ir::ProxyProtocolConfig>,
) {
    let mut shutdown_rx = ops.shutdown_rx();
    let local_addr = listener.local_addr().ok();
    loop {
        // KF-P16-014 / P15-WS5-PROBE-002: do not stop accepting on drain_complete.
        // Drain with zero admitted connections must still answer /live and /ready
        // (and 503 other paths) until shutdown is requested. Listener teardown
        // belongs to shutdown only — not to drain admission.
        let accept_result = tokio::select! {
            result = listener.accept() => result,
            changed = shutdown_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                // Shutdown starts the drain; keep the listener open so new
                // connections receive the 503 boundary instead of ECONNREFUSED
                // while in-flight requests finish. run_on_impl aborts the accept
                // tasks only after finalize_graceful_shutdown.
                continue;
            }
        };
        let (stream, mut peer) = match accept_result {
            Ok(conn) => conn,
            Err(err) => {
                warn!(%err, "accept error");
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let conn_state = reload::SharedServerState::clone(&state);
        let proxy_client = proxy_client.clone();

        // NG-06 / F3: detect and parse PROXY protocol header.
        // If enabled and present, replaces peer.ip() with the real client IP.
        let (stream, _proxy_header) =
            match read_proxy_header(stream, &proxy_protocol_config, peer.ip()).await {
                Ok((stream, Some(hdr))) => {
                    // LOCAL / UNSPEC headers carry no addresses: keep real peer.
                    if !hdr.source.ip().is_unspecified() {
                        peer = hdr.source;
                    }
                    (stream, Some(hdr))
                }
                Ok((stream, None)) => (stream, None),
                Err(err) => {
                    // Fail-closed: enabled listener requires a valid PROXY header.
                    warn!(%err, %peer, "dropping connection: invalid PROXY protocol header");
                    continue;
                }
            };

        let x_forwarded_for = HeaderValue::from_str(&peer.ip().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0.0.0.0"));
        let acceptor = tls_acceptor
            .as_ref()
            .and_then(|acceptor| acceptor.snapshot());
        // Fail-closed on a TLS-intended listener with no cert material
        // installed (ACME bootstrap-from-zero pending): drop the connection
        // instead of serving cleartext on a port clients treat as HTTPS.
        // HTTP-01 is served on the cleartext listener only. Live config
        // check — a reload removing server.tls installs None intentionally.
        if acceptor.is_none()
            && tls_acceptor.is_some()
            && local_addr
                .is_some_and(|addr| reload::read_state(&conn_state).config.listen_uses_tls(addr))
        {
            continue;
        }
        let ops_conn = Arc::clone(&ops);

        tokio::spawn(async move {
            if let Some(acceptor) = acceptor {
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        let alpn = tls_stream.get_ref().1.alpn_protocol();
                        let ctx = wire_dispatch_ctx(
                            reload::SharedServerState::clone(&conn_state),
                            proxy_client,
                            x_forwarded_for,
                            ops_conn.clone(),
                            "https",
                            local_addr.map(|a| a.port()),
                        );
                        // Wire-cheap modules (ratelimit/metrics) keep Cap067/wire;
                        // compression (or h2 / drain) still forces Hyper.
                        if !ctx.state.hyper_required_for_modules()
                            && alpn != Some(b"h2")
                            && !ops_conn.is_draining()
                        {
                            dispatch_stream(tls_stream, ctx).await;
                        } else {
                            serve_hyper(
                                tls_stream,
                                reload::SharedServerState::clone(&ctx.shared),
                                ctx.proxy_client,
                                ctx.x_forwarded_for,
                                ops_conn,
                                "https",
                                local_addr.map(|a| a.port()),
                                None,
                            )
                            .await;
                        }
                    }
                    Err(err) => warn!(%err, "tls accept failed"),
                }
                return;
            }
            dispatch_tcp(
                stream,
                wire_dispatch_ctx(
                    conn_state,
                    proxy_client,
                    x_forwarded_for,
                    ops_conn,
                    "http",
                    local_addr.map(|a| a.port()),
                ),
            )
            .await;
        });
    }
}

/// Max time a client has to send the PROXY header on an enabled listener.
/// Real LBs send it immediately; this bound only guards against stalls.
const PROXY_READ_TIMEOUT: Duration = Duration::from_secs(2);

/// NG-06 / F3: read PROXY protocol header from incoming stream.
///
/// Returns `(stream, Option<ProxyHeader>)` on success.
/// If `proxy_protocol` is not enabled in config, returns `(stream, None)`.
/// If enabled, the header is mandatory (fail-closed, like nginx
/// `listen ... proxy_protocol`): missing, malformed or untrusted → Err.
async fn read_proxy_header(
    mut stream: tokio::net::TcpStream,
    config: &Option<exyonq_config_ir::ProxyProtocolConfig>,
    peer_ip: std::net::IpAddr,
) -> anyhow::Result<(tokio::net::TcpStream, Option<proxy_protocol::ProxyHeader>)> {
    let cfg = match config.as_ref() {
        Some(c) if c.enabled => c,
        _ => return Ok((stream, None)),
    };

    // Trusted CIDRs: PROXY headers are only honored from these peers.
    // Since the header is mandatory on an enabled listener, an untrusted
    // peer effectively cannot connect here. Invalid CIDRs fail closed.
    if !cfg.trusted.is_empty() {
        let nets: Vec<ipnetwork::IpNetwork> =
            cfg.trusted.iter().filter_map(|c| c.parse().ok()).collect();
        if !nets.iter().any(|net| net.contains(peer_ip)) {
            anyhow::bail!("PROXY protocol: untrusted peer {peer_ip}");
        }
    }

    let hdr = match tokio::time::timeout(PROXY_READ_TIMEOUT, read_proxy_header_inner(&mut stream))
        .await
    {
        Ok(Ok(hdr)) => hdr,
        Ok(Err(err)) => return Err(err),
        Err(_) => anyhow::bail!("timeout waiting for PROXY header"),
    };
    Ok((stream, Some(hdr)))
}

async fn read_proxy_header_inner(
    stream: &mut tokio::net::TcpStream,
) -> anyhow::Result<proxy_protocol::ProxyHeader> {
    // First 6 bytes disambiguate: "PROXY " (v1) vs v2 signature prefix.
    let mut sig = [0u8; 6];
    stream.read_exact(&mut sig).await?;
    let mut buf = sig.to_vec();

    if sig == *proxy_protocol::V1_PREFIX {
        // v1: read until CRLF (max line 107 bytes, we already have 6).
        loop {
            let mut b = [0u8; 1];
            stream.read_exact(&mut b).await?;
            buf.push(b[0]);
            if buf.ends_with(b"\r\n") {
                break;
            }
            if buf.len() >= 108 {
                anyhow::bail!("PROXY v1 header exceeds max length");
            }
        }
    } else if sig == proxy_protocol::V2_SIGNATURE[..6] {
        // v2: 12-byte signature + ver/cmd + fam/trans + len (16 total),
        // then `len` bytes of addresses/TLVs.
        let mut rest = [0u8; 10];
        stream.read_exact(&mut rest).await?;
        buf.extend_from_slice(&rest);
        let len = u16::from_be_bytes([buf[14], buf[15]]) as usize;
        buf.resize(16 + len, 0);
        stream.read_exact(&mut buf[16..]).await?;
    } else {
        anyhow::bail!("missing PROXY protocol header");
    }

    let (hdr, _consumed) = proxy_protocol::parse(&buf).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(hdr)
}

/// Run on an explicit listen address (used in tests).
///
/// The caller owns the address, so every configured `listen` is pinned to it:
/// otherwise a fixture's placeholder address would get its own socket and the
/// run would claim a port of the host (EADDRINUSE on a busy machine).
pub async fn run_on(listen: SocketAddr, config: AppConfig) -> anyhow::Result<()> {
    run_on_impl(listen, None, pin_listen(config, listen)).await
}

/// `run_on` over a pre-bound listener (tests only): keeping the socket bound
/// removes the bind→drop→rebind window where a parallel `bind(:0)` can steal
/// the port and route the client to a foreign test server.
#[cfg(test)]
pub(crate) async fn run_on_with_listener(
    listen: SocketAddr,
    listener: TcpListener,
    config: AppConfig,
) -> anyhow::Result<()> {
    run_on_impl(listen, Some(listener), pin_listen(config, listen)).await
}

fn pin_listen(mut config: AppConfig, listen: SocketAddr) -> AppConfig {
    let listen = listen.to_string();
    for server in &mut config.servers {
        server.listen.clone_from(&listen);
    }
    config
}

async fn run_on_impl(
    listen: SocketAddr,
    first_listener: Option<TcpListener>,
    config: AppConfig,
) -> anyhow::Result<()> {
    let proxy_client = build_incoming_client();
    let mut config = config;
    config = discovery_overlay::apply_env_discovery_overlay(config);

    let tls_session_cache = TlsSessionCache::default();
    let shared_tls = SharedTlsAcceptor::new();
    reload::reload_tls_acceptor(&config, &shared_tls, &tls_session_cache)?;

    let state = ServerState::new(config.clone(), proxy_client.clone()).await?;
    kernel_control_port::mark_process_started();
    let shared = reload::wrap_state(state);

    // ACME bootstrap must run *after* the listeners accept: HTTP-01 is
    // validated by the CA fetching `/.well-known/acme-challenge/<token>` from
    // this process, and DNS-01/renewal do not block startup either. Spawn it
    // behind a readiness gate instead of awaiting it here.
    let ops = LifecycleState::new();

    // NG-20: bind the L4 listeners before the HTTP accept path starts. A bad
    // stream listener is a startup error, never a silently absent surface.
    crate::stream_proxy::start_streams(&config.streams, &config.upstreams, Arc::clone(&ops))
        .await?;

    // NG-25: every distinct `listen` gets its own socket. Servers sharing an
    // address are SNI/certificate variants of one listener (NG-24); a server
    // with a different address used to be silently ignored. The primary
    // address keeps the tuned backend below; additional addresses use the
    // plain Tokio accept loop, plaintext unless one of their servers has TLS.
    let extra_listens: Vec<SocketAddr> = config
        .listen_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|addr| *addr != listen)
        .collect();
    let mut extra_tasks = Vec::with_capacity(extra_listens.len());
    if !extra_listens.is_empty() {
        for addr in extra_listens {
            let acceptor = config.listen_uses_tls(addr).then(|| shared_tls.clone());
            let listener = bind_tuned(addr).await?;
            info!(
                %addr,
                tls = acceptor.is_some(),
                "exyonq listening (additional listen)"
            );
            extra_tasks.push(tokio::spawn(accept_loop(
                listener,
                reload::SharedServerState::clone(&shared),
                proxy_client.clone(),
                acceptor,
                Arc::clone(&ops),
                config.primary_server().proxy_protocol.clone(),
            )));
        }
    }

    {
        let bootstrap_config = config.clone();
        let bootstrap_tls = shared_tls.clone();
        let bootstrap_cache = tls_session_cache.clone();
        let bootstrap_shared = reload::SharedServerState::clone(&shared);
        tokio::spawn(async move {
            let Ok(addrs) = bootstrap_config.listen_addrs() else {
                return;
            };
            for addr in addrs {
                wait_for_listener(addr, Duration::from_secs(30)).await;
            }
            maybe_bootstrap_acme(
                &bootstrap_config,
                bootstrap_tls,
                bootstrap_cache,
                bootstrap_shared,
            )
            .await;
        });
    }
    // Cap031: tunnel tasks must keep active_connections > 0 after Hyper upgrade handoff.
    {
        let ops_ws = Arc::clone(&ops);
        exyonq_module_api::websocket_lifecycle::install_websocket_tunnel_hold(Arc::new(
            move || Some(Box::new(ops_ws.extend_for_upgraded_tunnel()) as Box<dyn Send>),
        ));
    }

    let kernel_port: Arc<dyn exyonq_module_api::kernel_control::KernelControlPort> =
        Arc::new(CoreKernelControlPort {
            shared: reload::SharedServerState::clone(&shared),
            proxy_client: proxy_client.clone(),
            tls_acceptor: shared_tls.clone(),
            tls_session_cache: tls_session_cache.clone(),
            lifecycle: Arc::clone(&ops),
        });

    let _reload_supervisor = reload_runtime_service().map(|runtime| {
        let config_path = std::env::var("EXYONQ_CONFIG")
            .ok()
            .map(std::path::PathBuf::from);
        runtime.start(config_path, Arc::clone(&kernel_port))
    });

    #[cfg(unix)]
    // Cap063: preserve OS path bytes. Non-empty CONTROL_SOCKET requires CONFIG +
    // registered control plane — fail closed (LA-CAP063-004).
    if let Some(socket_os) = std::env::var_os("EXYONQ_CONTROL_SOCKET") {
        if !socket_os.is_empty() {
            let config_path = std::env::var("EXYONQ_CONFIG").map_err(|_| {
                anyhow::anyhow!(
                    "EXYONQ_CONTROL_SOCKET is set but EXYONQ_CONFIG is missing; \
                     refuse to start without a bindable control socket config path"
                )
            })?;
            let control_plane = control_plane_service().ok_or_else(|| {
                anyhow::anyhow!(
                    "EXYONQ_CONTROL_SOCKET is set but no control plane service is registered"
                )
            })?;
            control_plane
                .spawn_unix_control_socket(
                    std::path::PathBuf::from(socket_os),
                    std::path::PathBuf::from(config_path),
                    kernel_port.clone(),
                )
                .map_err(|err| anyhow::anyhow!("control socket bind failed: {err}"))?;
        }
    }

    let _control_api_handle = {
        let api_socket = std::env::var("EXYONQ_API_SOCKET").ok();
        let api_tcp = std::env::var("EXYONQ_API_TCP").ok();
        if api_socket.is_some() || api_tcp.is_some() {
            let token_file = std::env::var("EXYONQ_SERVER_TOKEN_FILE")
                .ok()
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from);
            match (
                std::env::var("EXYONQ_CONFIG"),
                std::env::var("EXYONQ_SERVER_TOKEN"),
            ) {
                (Ok(config_path), token) if token.is_ok() || token_file.is_some() => {
                    let token = match token {
                        Ok(token) => token,
                        Err(_) => {
                            let path = token_file.as_ref().expect("token file checked above");
                            String::from_utf8(
                                exyonq_control_api::auth::read_token_file(path)?.to_vec(),
                            )
                            .map_err(|_| anyhow::anyhow!("invalid control api token file"))?
                        }
                    };
                    let unix_socket = api_socket.map(std::path::PathBuf::from);
                    let control_socket = std::env::var_os("EXYONQ_CONTROL_SOCKET")
                        .filter(|v| !v.is_empty())
                        .map(std::path::PathBuf::from);
                    if unix_socket.as_ref().is_some()
                        && unix_socket.as_ref() == control_socket.as_ref()
                    {
                        return Err(anyhow::anyhow!(
                            "EXYONQ_API_SOCKET must differ from EXYONQ_CONTROL_SOCKET"
                        ));
                    } else {
                        let tcp_addr = match api_tcp {
                            Some(raw) => match raw.parse::<SocketAddr>() {
                                Ok(addr) => Some(addr),
                                Err(err) => {
                                    return Err(anyhow::anyhow!("invalid EXYONQ_API_TCP: {err}"));
                                }
                            },
                            None => None,
                        };
                        match exyonq_control_api::spawn_control_api(
                            kernel_port.clone(),
                            exyonq_control_api::ControlApiSpawnConfig {
                                config_path: std::path::PathBuf::from(config_path),
                                unix_socket,
                                tcp_addr,
                                token,
                                read_token: std::env::var("EXYONQ_SERVER_READ_TOKEN")
                                    .ok()
                                    .filter(|value| !value.is_empty()),
                                write_token: std::env::var("EXYONQ_SERVER_WRITE_TOKEN")
                                    .ok()
                                    .filter(|value| !value.is_empty()),
                                token_file,
                            },
                        )
                        .await
                        {
                            Ok(handle) => {
                                info!(tasks = handle.task_count(), "http control api started");
                                Some(handle)
                            }
                            Err(err) => {
                                return Err(anyhow::anyhow!("http control api not started: {err}"));
                            }
                        }
                    }
                }
                _ => {
                    return Err(anyhow::anyhow!(
                        "EXYONQ_API_SOCKET/EXYONQ_API_TCP set but EXYONQ_CONFIG or EXYONQ_SERVER_TOKEN missing"
                    ));
                }
            }
        } else {
            None
        }
    };

    #[cfg(unix)]
    {
        let state = reload::read_state(&shared);
        let purge_env = std::env::var("EXYONQ_CACHE_PURGE_SOCKET")
            .ok()
            .filter(|path| !path.is_empty());
        // A configured control socket tells us which per-unit RuntimeDirectory is
        // writable. Derive the purge socket there so systemd deployments get the
        // WordPress sidecars without a second hand-written variable. The explicit
        // EXYONQ_CACHE_PURGE_SOCKET still wins.
        let purge_path = purge_env.clone().or_else(|| {
            if !state.snapshot.full_page_cache.enabled {
                return None;
            }
            let control = std::env::var("EXYONQ_CONTROL_SOCKET").ok()?;
            let parent = std::path::Path::new(&control).parent()?;
            (!parent.as_os_str().is_empty()).then(|| {
                parent
                    .join("cache-purge.sock")
                    .to_string_lossy()
                    .into_owned()
            })
        });
        if !state.snapshot.full_page_cache.enabled {
            if purge_env.is_some() {
                tracing::warn!(
                    "EXYONQ_CACHE_PURGE_SOCKET is set but full_page_cache is disabled; purge socket not started"
                );
            }
        } else if let Some(purge_path) = purge_path {
            if state.config.routes.len() != state.snapshot.full_page_cache.route_site_ids.len() {
                return Err(anyhow::anyhow!(
                    "full_page_cache site ids do not match the route table"
                ));
            }
            let token = match std::env::var("EXYONQ_CACHE_PURGE_TOKEN") {
                Ok(token) if !token.is_empty() => token,
                _ => crate::cache_purge_port::generate_purge_token().map_err(|err| {
                    anyhow::anyhow!(
                        "purge token was not provided and could not be generated: {err}"
                    )
                })?,
            };
            let sites: Vec<(&str, u64)> = state
                .config
                .routes
                .iter()
                .zip(state.snapshot.full_page_cache.route_site_ids.iter())
                .map(|(route, id)| (route.name.as_str(), *id))
                .collect();
            crate::cache_purge_port::publish_purge_sidecars(
                std::path::Path::new(&purge_path),
                &token,
                &sites,
            )
            .map_err(|err| anyhow::anyhow!("purge socket files were not written: {err}"))?;
            if let Some(control_plane) = control_plane_service() {
                let purge_port = crate::lab_coord_hooks::build_lab_or_default_purge_port(
                    reload::SharedServerState::clone(&shared),
                );
                control_plane.spawn_unix_cache_purge_socket(
                    exyonq_module_api::CachePurgeSocketConfig {
                        socket_path: std::path::PathBuf::from(&purge_path),
                        token: std::sync::Arc::<[u8]>::from(token.into_bytes()),
                    },
                    purge_port,
                );
                crate::lab_coord_hooks::start_lab_subscriber_if_any(
                    reload::SharedServerState::clone(&shared),
                );
            } else {
                tracing::warn!(
                    path = %purge_path,
                    "purge sidecars written but no control plane is registered; purge socket not started"
                );
            }
        }
    }

    if config.http3.enabled {
        if let Some(http3_listen) = config.primary_server().http3_listen.as_deref() {
            if let Ok(addr) = http3_listen.parse::<SocketAddr>() {
                let primary = config.primary_server();
                if primary.tls.is_some() {
                    // Every [[server]] sharing the primary listen contributes
                    // its pair; the first is the QUIC default and s2n selects
                    // the rest by SNI. ACME bootstrap-from-zero: H3 has no
                    // hot-reload, so spawn only with the complete set on disk
                    // (restart after the first issuance to enable H3).
                    let mut tls_certs = Vec::new();
                    let mut missing = Vec::new();
                    for server in &config.servers {
                        if server.listen != primary.listen {
                            continue;
                        }
                        let Some(tls) = server.tls.as_ref() else {
                            continue;
                        };
                        if tls.cert.exists() && tls.key.exists() {
                            tls_certs.push(TlsSettings {
                                cert_path: tls.cert.clone(),
                                key_path: tls.key.clone(),
                            });
                        } else {
                            missing.push(tls.cert.display().to_string());
                        }
                    }
                    if tls_certs.is_empty() || !missing.is_empty() {
                        warn!(
                            listen = %primary.listen,
                            missing = ?missing,
                            "http3_listen skipped: every TLS certificate on the shared listen \
                             must be on disk (restart after ACME issuance to enable HTTP/3)"
                        );
                    } else {
                        let settings = exyonq_mod_http3::Http3Settings {
                            listen: addr,
                            tls_certs,
                            // Neutral IR string only — no provider types in core.
                            provider: config.http3.provider.clone(),
                            max_ack_delay_ms: config.http3.max_ack_delay_ms,
                            request_body_drain_cap_bytes: config
                                .http3
                                .request_body_drain_cap_bytes
                                .min(usize::MAX as u64)
                                as usize,
                            qlog_enabled: config.http3.qlog,
                        };
                        http3_runtime_registry::spawn_http3_listener(
                            settings,
                            reload::SharedServerState::clone(&shared),
                            proxy_client.clone(),
                            Arc::clone(&ops),
                        );
                    }
                }
            }
        }
    }

    let workers = if first_listener.is_some() {
        // Test path: the pre-bound listener is the single accept worker.
        1
    } else {
        accept_workers(&config.tuning)
    };
    // Expose the actual primary accept-pool geometry on /metrics. The
    // appender reads /proc at scrape time; this only records the real count.
    crate::worker_memory::set_server_worker_count(workers);
    let tls_on_listen = shared_tls.snapshot().is_some();
    #[cfg(target_os = "linux")]
    let use_io_uring = io_uring_start::io_uring_env_enabled(tls_on_listen);
    #[cfg(not(target_os = "linux"))]
    let use_io_uring = false;
    #[cfg(target_os = "linux")]
    let epoll_keepalive =
        !config.waf.enabled && epoll_start::epoll_static_env_enabled(tls_on_listen);
    #[cfg(target_os = "linux")]
    let use_epoll_listener = epoll_keepalive && epoll_start::epoll_listen_env_enabled();
    #[cfg(target_os = "linux")]
    let use_epoll = use_epoll_listener;
    #[cfg(not(target_os = "linux"))]
    let use_epoll = false;
    #[cfg(target_os = "linux")]
    let use_sync_accept =
        !use_io_uring && !use_epoll && sync_accept_start::sync_accept_env_enabled(tls_on_listen);
    #[cfg(not(target_os = "linux"))]
    let use_sync_accept = false;
    #[cfg(target_os = "linux")]
    // Cap067 P5 KEEP (WouldBlock→Hyper) sends unread accepts to Tokio. Static must still
    // reach Cap067 sendfile via register_sendfile_from_tokio_first — that requires the
    // divert/keepalive pool even when EPOLL_LISTEN owns accept. Without it, P1 falls to
    // Hyper (~140k vs ~280k Cap067). geom4: 4 listen + 4 pool is intentional (not 8+8).
    let epoll_keepalive_active = epoll_keepalive && !use_io_uring;
    #[cfg(not(target_os = "linux"))]
    let epoll_keepalive_active = false;
    #[cfg(target_os = "linux")]
    let use_sdp = !tls_on_listen && sdp_start::env_enabled();
    #[cfg(not(target_os = "linux"))]
    let use_sdp = false;

    info!(
        %listen,
        workers,
        tls = tls_on_listen,
        sync_accept = use_sync_accept,
        epoll_static = use_epoll,
        epoll_keepalive = epoll_keepalive_active,
        io_uring = use_io_uring,
        sdp_p1 = use_sdp,
        "exyonq listening"
    );

    #[cfg(target_os = "linux")]
    epoll_start::install_static_wire_hooks();

    // A blocking static wire thread has no Hyper: let it replay a returned head
    // on the async runtime instead of answering with a fabricated 404.
    #[cfg(target_os = "linux")]
    wire_dispatch::install_blocking_wire_handoff(
        reload::SharedServerState::clone(&shared),
        proxy_client.clone(),
        Arc::clone(&ops),
        tokio::runtime::Handle::current(),
    );

    // PS3A-F3 I7 / PS1C-FACADE / PS3A-PM1: composition builds executor; sync_accept takes facade Arc.
    #[cfg(target_os = "linux")]
    let connection_executor =
        if epoll_keepalive_active || use_io_uring || use_epoll || use_sync_accept {
            Some(Arc::new(connection_executor::CoreConnectionExecutor::new(
                reload::SharedServerState::clone(&shared),
                proxy_client.clone(),
                Arc::clone(&ops),
                tokio::runtime::Handle::current(),
            )))
        } else {
            None
        };
    #[cfg(target_os = "linux")]
    let platform_entry = if use_sync_accept || use_io_uring || use_epoll || epoll_keepalive_active {
        Some(Arc::new(
            crate::kernel::PlatformConnectionEntry::from_executor(Arc::clone(
                connection_executor.as_ref().expect("connection executor"),
            )),
        ))
    } else {
        None
    };

    // Cap067 divert pool: Tokio→OS-thread handoff for sendfile after Hyper wire plan.
    // Also required under EPOLL_LISTEN=1 so WouldBlock→Hyper static can divert (P1).
    #[cfg(target_os = "linux")]
    if epoll_keepalive_active {
        epoll_start::prepare_registered_keepalive_pool(
            workers,
            Arc::clone(platform_entry.as_ref().expect("platform entry for epoll")),
        );
    }

    #[cfg(target_os = "linux")]
    if use_sdp {
        // Exclusive accept: do not start Tokio/epoll/io_uring listeners on this bind.
        let sdp = sdp_start::start_sdp(
            listen,
            reload::SharedServerState::clone(&shared),
            proxy_client.clone(),
            Arc::clone(&ops),
            tokio::runtime::Handle::current(),
        )?;
        LifecycleState::wait_for_shutdown(&ops).await;
        info!("shutdown requested, stopping sdp-p1 listener");
        sdp.signal_stop();
        finalize_graceful_shutdown(&ops).await;
        stop_listener_tasks(&mut extra_tasks).await;
        sdp.join();
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    if use_io_uring {
        let entry = platform_entry.expect("platform entry for io_uring");
        let workers_handle = io_uring_start::start_registered_io_uring(listen, workers, entry)?;
        LifecycleState::wait_for_shutdown(&ops).await;
        info!("shutdown requested, stopping listener");
        workers_handle.signal_stop();
        epoll_start::signal_registered_keepalive_pool_stop();
        finalize_graceful_shutdown(&ops).await;
        stop_listener_tasks(&mut extra_tasks).await;
        workers_handle.stop();
        epoll_start::stop_registered_keepalive_pool();
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    if use_epoll {
        let entry = platform_entry.expect("platform entry for epoll listen");
        let workers_handle = epoll_start::start_registered_epoll_listen(listen, workers, entry)?;
        LifecycleState::wait_for_shutdown(&ops).await;
        info!("shutdown requested, stopping listener");
        workers_handle.signal_stop();
        epoll_start::signal_registered_keepalive_pool_stop();
        finalize_graceful_shutdown(&ops).await;
        stop_listener_tasks(&mut extra_tasks).await;
        workers_handle.stop();
        epoll_start::stop_registered_keepalive_pool();
        return Ok(());
    }

    if use_sync_accept {
        #[cfg(target_os = "linux")]
        {
            let entry = platform_entry.expect("platform entry for sync_accept");
            let sync_workers =
                sync_accept_start::start_registered_sync_accept(listen, workers, entry)?;
            LifecycleState::wait_for_shutdown(&ops).await;
            info!("shutdown requested, stopping listener");
            sync_workers.signal_stop();
            epoll_start::signal_registered_keepalive_pool_stop();
            finalize_graceful_shutdown(&ops).await;
            stop_listener_tasks(&mut extra_tasks).await;
            sync_workers.stop();
            epoll_start::stop_registered_keepalive_pool();
            return Ok(());
        }
    }

    let mut tasks = Vec::with_capacity(workers);
    let mut first_listener = first_listener;
    let proxy_protocol_config = config.primary_server().proxy_protocol.clone();
    for _ in 0..workers {
        let listener = match first_listener.take() {
            Some(l) => l,
            None => bind_tuned(listen).await?,
        };
        let state = reload::SharedServerState::clone(&shared);
        let proxy_client = proxy_client.clone();
        let acceptor = Some(shared_tls.clone());
        let ops_worker = Arc::clone(&ops);
        let pp_config = proxy_protocol_config.clone();
        tasks.push(tokio::spawn(accept_loop(
            listener,
            state,
            proxy_client,
            acceptor,
            ops_worker,
            pp_config,
        )));
    }

    LifecycleState::wait_for_shutdown(&ops).await;
    info!("shutdown requested, draining before listener teardown");
    #[cfg(target_os = "linux")]
    epoll_start::signal_registered_keepalive_pool_stop();
    finalize_graceful_shutdown(&ops).await;
    stop_listener_tasks(&mut tasks).await;
    stop_listener_tasks(&mut extra_tasks).await;
    #[cfg(target_os = "linux")]
    epoll_start::stop_registered_keepalive_pool();

    Ok(())
}

/// Abort listener accept loops only after the drain boundary has been finalized.
async fn stop_listener_tasks(tasks: &mut Vec<tokio::task::JoinHandle<()>>) {
    for task in tasks.iter() {
        task.abort();
    }
    for task in tasks.drain(..) {
        let _ = task.await;
    }
}

/// Cap041: stop Cap024 health probes, then wait for admitted connections (bounded).
async fn finalize_graceful_shutdown(ops: &Arc<LifecycleState>) {
    exyonq_mod_proxy::try_shutdown_health();
    let drained = LifecycleState::wait_for_drain_complete(
        ops,
        LifecycleState::DEFAULT_GRACEFUL_SHUTDOWN_WAIT,
    )
    .await;
    if drained {
        info!(
            active_connections = ops.active_connections(),
            "graceful shutdown: drain complete"
        );
    } else {
        warn!(
            active_connections = ops.active_connections(),
            wait_secs = LifecycleState::DEFAULT_GRACEFUL_SHUTDOWN_WAIT.as_secs(),
            "graceful shutdown: wait timed out; proceeding to process exit"
        );
    }
}

async fn maybe_bootstrap_acme(
    config: &AppConfig,
    tls_acceptor: SharedTlsAcceptor,
    tls_session_cache: TlsSessionCache,
    shared: reload::SharedServerState,
) {
    let Some(service) = acme_integration_service() else {
        return;
    };
    let publication = Arc::new(CoreCertificatePublicationPort {
        tls_acceptor,
        tls_session_cache,
        shared,
    });
    service.bootstrap_if_enabled(config, publication).await;
}

#[cfg(test)]
fn test_config_with_listen(addr: std::net::SocketAddr, tls_block: &str) -> String {
    format!(
        r#"config_version = 1

[[server]]
listen = "{addr}"
routes = ["site"]
{tls_block}
[[route]]
name = "site"
match = {{ path = "/" }}
root = "tests/fixtures/www"
index = "index.html"
"#
    )
}

#[cfg(test)]
mod acme_pending_fail_closed {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;

    const TLS_PENDING: &str = r#"
[server.tls]
cert = "/nonexistent/acme-pending/cert.pem"
key = "/nonexistent/acme-pending/key.pem"

[server.tls.acme]
email = "ops@example.test"
domains = ["example.test"]
"#;

    async fn spawn_accept_loop(listener: TcpListener, config: AppConfig) {
        let state = ServerState::new(config, build_incoming_client())
            .await
            .unwrap();
        tokio::spawn(accept_loop(
            listener,
            reload::wrap_state(state),
            build_incoming_client(),
            // Shared acceptor with nothing installed: cert material pending.
            Some(SharedTlsAcceptor::new()),
            LifecycleState::new(),
            None,
        ));
    }

    #[tokio::test]
    async fn tls_pending_listener_drops_connections() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config: AppConfig = test_config_with_listen(addr, TLS_PENDING).parse().unwrap();
        spawn_accept_loop(listener, config).await;

        let mut conn = TcpStream::connect(addr).await.unwrap();
        let mut buf = [0u8; 8];
        let got = tokio::time::timeout(Duration::from_secs(5), conn.read(&mut buf)).await;
        assert!(
            matches!(got, Ok(Ok(0))),
            "TLS-intended listener without material must drop, got {got:?}"
        );
    }

    #[tokio::test]
    async fn reload_removing_tls_reopens_cleartext() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config: AppConfig = test_config_with_listen(addr, TLS_PENDING).parse().unwrap();
        let state = ServerState::new(config, build_incoming_client())
            .await
            .unwrap();
        let shared = reload::wrap_state(state);
        tokio::spawn(accept_loop(
            listener,
            reload::SharedServerState::clone(&shared),
            build_incoming_client(),
            Some(SharedTlsAcceptor::new()),
            LifecycleState::new(),
            None,
        ));

        // TLS intended, no material: dropped.
        let mut conn = TcpStream::connect(addr).await.unwrap();
        let mut buf = [0u8; 8];
        let got = tokio::time::timeout(Duration::from_secs(5), conn.read(&mut buf)).await;
        assert!(
            matches!(got, Ok(Ok(0))),
            "pending TLS must drop, got {got:?}"
        );
        drop(conn);

        // Reload swaps in a config without server.tls: cleartext intended.
        let no_tls: AppConfig = test_config_with_listen(addr, "").parse().unwrap();
        let state = ServerState::new(no_tls, build_incoming_client())
            .await
            .unwrap();
        *shared.write().expect("server state lock poisoned") = state;

        let mut conn = TcpStream::connect(addr).await.unwrap();
        let got = tokio::time::timeout(Duration::from_millis(500), conn.read(&mut buf)).await;
        assert!(
            got.is_err(),
            "listener must serve cleartext once TLS is removed, got {got:?}"
        );
    }

    #[tokio::test]
    async fn shutdown_keeps_listener_open_for_drain_boundary() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config: AppConfig = test_config_with_listen(addr, "").parse().unwrap();
        let state = ServerState::new(config, build_incoming_client())
            .await
            .unwrap();
        let shared = reload::wrap_state(state);
        let ops = LifecycleState::new();
        let active = ops
            .try_enter()
            .expect("admit test connection before shutdown");
        let task = tokio::spawn(accept_loop(
            listener,
            shared,
            build_incoming_client(),
            None,
            Arc::clone(&ops),
            None,
        ));

        ops.request_shutdown();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut conn = TcpStream::connect(addr)
            .await
            .expect("listener must stay open while drain is pending");
        conn.write_all(b"GET / HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), conn.read_to_end(&mut response))
            .await
            .expect("drain boundary response timeout")
            .unwrap();
        assert!(
            response.starts_with(b"HTTP/1.1 503"),
            "new connection during drain must get 503, got {:?}",
            String::from_utf8_lossy(&response)
        );
        assert!(
            response.windows(8).any(|window| window == b"draining"),
            "drain boundary body missing: {:?}",
            String::from_utf8_lossy(&response)
        );
        assert!(
            response.windows(12).any(|window| window == b"Retry-After:"),
            "drain boundary Retry-After missing: {:?}",
            String::from_utf8_lossy(&response)
        );
        assert!(!ops.drain_complete(), "test token must keep drain pending");
        drop(active);
        assert!(ops.drain_complete());
        task.abort();
    }

    #[tokio::test]
    async fn cleartext_listener_still_serves() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config: AppConfig = test_config_with_listen(addr, "").parse().unwrap();
        spawn_accept_loop(listener, config).await;

        // No request written: only assert the gate does not drop a cleartext
        // connection. (Full request path needs CLI-registered proxy hooks;
        // out of scope for this gate regression.)
        let mut conn = TcpStream::connect(addr).await.unwrap();
        let mut buf = [0u8; 8];
        let got = tokio::time::timeout(Duration::from_millis(500), conn.read(&mut buf)).await;
        assert!(
            got.is_err(),
            "cleartext listener must keep the connection open, got {got:?}"
        );
    }
}

#[cfg(test)]
mod run_on_pins_listen {
    use super::*;
    use crate::kernel::test_hooks::wire_hooks_gate;
    use std::net::TcpListener as StdTcpListener;
    use std::time::Duration;
    use tokio::net::TcpStream;

    /// Regression: `run_on` used to pass the config through untouched, so a
    /// configured `listen` other than the one handed in got its own socket.
    /// When that address is already taken on the host the extra bind failed and
    /// the call never served the address it was given. Here the config points at
    /// a port this test holds open, so the old code failed with EADDRINUSE.
    #[tokio::test]
    async fn serves_the_given_address_when_the_configured_listen_is_busy() {
        // Starting a server applies its config into the process-global module
        // runtimes (proxy cluster generation and bindings). Hold the shared
        // gate so a sibling wire test serving concurrently cannot see its
        // generation flipped mid-request and fail with an unrelated 503.
        let _gate = wire_hooks_gate().await;
        let busy = StdTcpListener::bind("127.0.0.1:0").expect("busy port");
        let busy_addr = busy.local_addr().expect("busy addr");

        let probe = StdTcpListener::bind("127.0.0.1:0").expect("probe port");
        let listen = probe.local_addr().expect("probe addr");
        drop(probe);

        let config: AppConfig = test_config_with_listen(busy_addr, "")
            .parse()
            .expect("config");
        let server = tokio::spawn(async move { run_on(listen, config).await });

        let mut served = false;
        for _ in 0..100 {
            if TcpStream::connect(listen).await.is_ok() {
                served = true;
                break;
            }
            if server.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if !served {
            // Nothing ever accepted on the given address: report why, so a
            // regression names the failed bind instead of just timing out here.
            let outcome = tokio::time::timeout(Duration::from_secs(5), server).await;
            panic!(
                "run_on must serve {listen} while the configured listen {busy_addr} is taken; task outcome: {outcome:?}"
            );
        }
        server.abort();
    }
}
