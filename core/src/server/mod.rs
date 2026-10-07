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
    default_runtime_parallelism, resolve_accept_workers, resolve_epoll_pool_threads,
    resolve_tokio_worker_threads, MAX_WORKERS, MIN_WORKERS,
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

fn accept_workers() -> usize {
    crate::server::runtime_parallelism::resolve_accept_workers()
}

async fn serve_hyper<S>(
    mut stream: S,
    shared: reload::SharedServerState,
    proxy_client: ProxyClient,
    x_forwarded_for: HeaderValue,
    ops: Arc<LifecycleState>,
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
    let io = TokioIo::new(stream);
    let ops_for_service = Arc::clone(&ops);
    // Cap057 LA-CAP057-002: refresh live SharedServerState per request (mirror H3).
    // Accept-time Arc pin made FPC HITs survive reload disable / live purge on keep-alive.
    let service = hyper::service::service_fn(move |req| {
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
            let mut response = serve_connection(
                ConnectionContext {
                    state: conn_state,
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
    builder.http1().keep_alive(true);
    if let Err(err) = builder.serve_connection_with_upgrades(io, service).await {
        warn!(%err, "connection error");
    }
}

fn wire_dispatch_ctx(
    shared: reload::SharedServerState,
    proxy_client: ProxyClient,
    x_forwarded_for: HeaderValue,
    ops: Arc<LifecycleState>,
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
    }
}

async fn accept_loop(
    listener: TcpListener,
    state: reload::SharedServerState,
    proxy_client: ProxyClient,
    tls_acceptor: SharedTlsAcceptor,
    ops: Arc<LifecycleState>,
) {
    let mut shutdown_rx = ops.shutdown_rx();
    loop {
        // KF-P16-014 / P15-WS5-PROBE-002: do not stop accepting on drain_complete.
        // Drain with zero admitted connections must still answer /live and /ready
        // (and 503 other paths) until shutdown is requested. Listener teardown
        // belongs to shutdown only — not to drain admission.
        let accept_result = tokio::select! {
            result = listener.accept() => result,
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && ops.shutdown_requested() {
                    break;
                }
                continue;
            }
        };
        let (stream, peer) = match accept_result {
            Ok(conn) => conn,
            Err(err) => {
                warn!(%err, "accept error");
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let conn_state = reload::SharedServerState::clone(&state);
        let proxy_client = proxy_client.clone();
        let x_forwarded_for = HeaderValue::from_str(&peer.ip().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0.0.0.0"));
        let acceptor = tls_acceptor.snapshot().acceptor();
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
                wire_dispatch_ctx(conn_state, proxy_client, x_forwarded_for, ops_conn),
            )
            .await;
        });
    }
}

/// Run on an explicit listen address (used in tests).
pub async fn run_on(listen: SocketAddr, config: AppConfig) -> anyhow::Result<()> {
    let proxy_client = build_incoming_client();
    let mut config = config;
    config = discovery_overlay::apply_env_discovery_overlay(config);

    let tls_session_cache = TlsSessionCache::default();
    let shared_tls = SharedTlsAcceptor::new();
    reload::reload_tls_acceptor(&config, &shared_tls, &tls_session_cache)?;

    maybe_bootstrap_acme(&config, shared_tls.clone(), tls_session_cache.clone()).await;

    let state = ServerState::new(config.clone(), proxy_client.clone()).await?;
    kernel_control_port::mark_process_started();
    let shared = reload::wrap_state(state);
    let ops = LifecycleState::new();
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
            match (
                std::env::var("EXYONQ_CONFIG"),
                std::env::var("EXYONQ_SERVER_TOKEN"),
            ) {
                (Ok(config_path), Ok(token)) => {
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
        if !state.snapshot.full_page_cache.enabled {
            if purge_env.is_some() {
                tracing::warn!(
                    "EXYONQ_CACHE_PURGE_SOCKET is set but full_page_cache is disabled; purge socket not started"
                );
            }
        } else if let Some(purge_path) = purge_env {
            if state.config.routes.len() != state.snapshot.full_page_cache.route_site_ids.len() {
                return Err(anyhow::anyhow!(
                    "full_page_cache site ids do not match the route table"
                ));
            }
            let token = match std::env::var("EXYONQ_CACHE_PURGE_TOKEN") {
                Ok(token) if !token.is_empty() => token,
                _ => crate::cache_purge_port::generate_purge_token().map_err(|err| {
                    anyhow::anyhow!("purge token was not provided and could not be generated: {err}")
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
            let control_plane = control_plane_service().ok_or_else(|| {
                anyhow::anyhow!(
                    "full_page_cache is enabled and EXYONQ_CACHE_PURGE_SOCKET is set, but no control plane is registered"
                )
            })?;
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
        }
    }

    if config.http3.enabled {
        if let Some(http3_listen) = config.primary_server().http3_listen.as_deref() {
            if let Ok(addr) = http3_listen.parse::<SocketAddr>() {
                if let Some(tls) = config.primary_server().tls.clone() {
                    let settings = exyonq_mod_http3::Http3Settings {
                        listen: addr,
                        tls: TlsSettings {
                            cert_path: tls.cert,
                            key_path: tls.key,
                        },
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

    let workers = accept_workers();
    let tls_on_listen = shared_tls.snapshot().is_loaded();
    #[cfg(target_os = "linux")]
    let use_io_uring = io_uring_start::io_uring_env_enabled(tls_on_listen);
    #[cfg(not(target_os = "linux"))]
    let use_io_uring = false;
    #[cfg(target_os = "linux")]
    let epoll_keepalive = epoll_start::epoll_static_env_enabled(tls_on_listen);
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
            sync_workers.stop();
            epoll_start::stop_registered_keepalive_pool();
            return Ok(());
        }
    }

    let mut tasks = Vec::with_capacity(workers);
    for _ in 0..workers {
        let listener = bind_tuned(listen).await?;
        let state = reload::SharedServerState::clone(&shared);
        let proxy_client = proxy_client.clone();
        let acceptor = shared_tls.clone();
        let ops_worker = Arc::clone(&ops);
        tasks.push(tokio::spawn(accept_loop(
            listener,
            state,
            proxy_client,
            acceptor,
            ops_worker,
        )));
    }

    LifecycleState::wait_for_shutdown(&ops).await;
    info!("shutdown requested, stopping listener");
    for task in &tasks {
        task.abort();
    }
    #[cfg(target_os = "linux")]
    epoll_start::signal_registered_keepalive_pool_stop();
    finalize_graceful_shutdown(&ops).await;
    for task in tasks {
        let _ = task.await;
    }
    #[cfg(target_os = "linux")]
    epoll_start::stop_registered_keepalive_pool();

    Ok(())
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
) {
    let Some(service) = acme_integration_service() else {
        return;
    };
    let publication = Arc::new(CoreCertificatePublicationPort {
        tls_acceptor,
        tls_session_cache,
    });
    service.bootstrap_if_enabled(config, publication).await;
}
