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
pub mod state;
pub mod sync_accept_start;
mod wire_dispatch;

#[cfg(all(test, target_os = "linux"))]
mod ps2_iu2_tests;
#[cfg(all(test, target_os = "linux"))]
mod ps3a_epoll_fsm_twin_tests;

pub use epoll_start::{epoll_listen_env_enabled, epoll_static_env_enabled};
#[cfg(target_os = "linux")]
pub use epoll_start::{
    register_epoll_keepalive_enqueue, register_epoll_keepalive_prepare,
    register_epoll_keepalive_stop, register_epoll_listen_start,
};
pub use io_uring_start::io_uring_env_enabled;
#[cfg(target_os = "linux")]
pub use io_uring_start::register_io_uring_start;
#[cfg(target_os = "linux")]
pub use os_worker_guard::OsWorkerGuard;
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
    if let Ok(raw) = std::env::var("EXYONQ_ACCEPT_WORKERS") {
        if raw.eq_ignore_ascii_case("auto") {
            return std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .clamp(1, 16);
        }
        if let Ok(n) = raw.parse::<usize>() {
            return n.clamp(1, 16);
        }
    }
    1
}

async fn serve_hyper<S>(
    mut stream: S,
    state: Arc<ServerState>,
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
    let service = hyper::service::service_fn(move |req| {
        let conn_state = Arc::clone(&state);
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
    state: Arc<ServerState>,
    proxy_client: ProxyClient,
    x_forwarded_for: HeaderValue,
    ops: Arc<LifecycleState>,
) -> WireDispatchContext {
    let pinned_generation = state.generation;
    WireDispatchContext {
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
        let acceptor = tls_acceptor.snapshot();
        let ops_conn = Arc::clone(&ops);

        tokio::spawn(async move {
            if let Some(acceptor) = acceptor {
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        let state = reload::read_state(&conn_state);
                        let alpn = tls_stream.get_ref().1.alpn_protocol();
                        let ctx = wire_dispatch_ctx(
                            state,
                            proxy_client,
                            x_forwarded_for,
                            ops_conn.clone(),
                        );
                        if !ctx.state.modules_enabled()
                            && alpn != Some(b"h2")
                            && !ops_conn.is_draining()
                        {
                            dispatch_stream(tls_stream, ctx).await;
                        } else {
                            serve_hyper(
                                tls_stream,
                                ctx.state,
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
            let state = reload::read_state(&conn_state);
            dispatch_tcp(
                stream,
                wire_dispatch_ctx(state, proxy_client, x_forwarded_for, ops_conn),
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
    if let Ok(socket_path) = std::env::var("EXYONQ_CONTROL_SOCKET") {
        if let Ok(config_path) = std::env::var("EXYONQ_CONFIG") {
            if let Some(control_plane) = control_plane_service() {
                control_plane.spawn_unix_control_socket(
                    std::path::PathBuf::from(socket_path),
                    std::path::PathBuf::from(config_path),
                    kernel_port.clone(),
                );
            }
        }
    }

    #[cfg(unix)]
    if let Ok(purge_path) = std::env::var("EXYONQ_CACHE_PURGE_SOCKET") {
        match std::env::var("EXYONQ_CACHE_PURGE_TOKEN") {
            Ok(token) if !token.is_empty() => {
                if let Some(control_plane) = control_plane_service() {
                    let purge_port = crate::lab_coord_hooks::build_lab_or_default_purge_port(
                        reload::SharedServerState::clone(&shared),
                    );
                    control_plane.spawn_unix_cache_purge_socket(
                        exyonq_module_api::CachePurgeSocketConfig {
                            socket_path: std::path::PathBuf::from(purge_path),
                            token: std::sync::Arc::<[u8]>::from(token.into_bytes()),
                        },
                        purge_port,
                    );
                    crate::lab_coord_hooks::start_lab_subscriber_if_any(
                        reload::SharedServerState::clone(&shared),
                    );
                }
            }
            _ => {
                tracing::warn!(
                    "EXYONQ_CACHE_PURGE_SOCKET set but EXYONQ_CACHE_PURGE_TOKEN missing/empty; purge socket not started"
                );
            }
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
    let tls_on_listen = shared_tls.snapshot().is_some();
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
    let epoll_keepalive_active = epoll_keepalive && !use_epoll && !use_io_uring;
    #[cfg(not(target_os = "linux"))]
    let epoll_keepalive_active = false;
    info!(
        %listen,
        workers,
        tls = tls_on_listen,
        sync_accept = use_sync_accept,
        epoll_static = use_epoll,
        epoll_keepalive = epoll_keepalive_active,
        io_uring = use_io_uring,
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

    #[cfg(target_os = "linux")]
    if epoll_keepalive_active || use_epoll {
        epoll_start::prepare_registered_keepalive_pool(
            workers,
            Arc::clone(platform_entry.as_ref().expect("platform entry for epoll")),
        );
    }

    #[cfg(target_os = "linux")]
    if use_io_uring {
        let entry = platform_entry.expect("platform entry for io_uring");
        let workers_handle = io_uring_start::start_registered_io_uring(listen, workers, entry)?;
        LifecycleState::wait_for_shutdown(&ops).await;
        info!("shutdown requested, stopping listener");
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
    for task in tasks {
        task.abort();
    }
    #[cfg(target_os = "linux")]
    epoll_start::stop_registered_keepalive_pool();

    Ok(())
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
