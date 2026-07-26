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
//! OS signal listeners — map Ctrl-C / SIGTERM to `KernelControlPort::request_shutdown`.
//!
//! P15-WS5-SIG-001: systemd `KillMode=control-group` default stop sends SIGTERM.
//! P15-WS5-SIG-002: keep listening until cancel so repeated signals stay idempotent
//! (`request_shutdown`) instead of falling through to the default terminate handler.
//! Unsupported: SIGHUP (no config-on-signal; use `exyonqctl reload`), SIGQUIT.

use exyonq_module_api::kernel_control::KernelControlPort;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub fn spawn_shutdown_listener(
    port: Arc<dyn KernelControlPort>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(err) => {
                    tracing::warn!(%err, "SIGTERM listener failed to install; ctrl_c only");
                    spawn_ctrl_c_only(port, cancel).await;
                    return;
                }
            };
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    res = tokio::signal::ctrl_c() => {
                        match res {
                            Ok(()) => {
                                info!(signal = "SIGINT", "shutdown signal received, requesting kernel shutdown");
                                let _ = port.request_shutdown();
                            }
                            Err(err) => {
                                tracing::warn!(%err, "ctrl_c listener failed");
                                break;
                            }
                        }
                    }
                    _ = sigterm.recv() => {
                        info!(signal = "SIGTERM", "shutdown signal received, requesting kernel shutdown");
                        let _ = port.request_shutdown();
                    }
                }
            }
        }
        #[cfg(not(unix))]
        {
            spawn_ctrl_c_only(port, cancel).await;
        }
    })
}

async fn spawn_ctrl_c_only(port: Arc<dyn KernelControlPort>, cancel: CancellationToken) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            res = tokio::signal::ctrl_c() => {
                match res {
                    Ok(()) => {
                        info!(signal = "SIGINT", "shutdown signal received, requesting kernel shutdown");
                        let _ = port.request_shutdown();
                    }
                    Err(err) => {
                        tracing::warn!(%err, "ctrl_c listener failed");
                        break;
                    }
                }
            }
        }
    }
}
