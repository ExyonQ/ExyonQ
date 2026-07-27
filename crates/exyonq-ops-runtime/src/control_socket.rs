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
//! Unix control socket — command parsing and JSON response formatting.

use exyonq_module_api::cache_purge::{CachePurgePort, CachePurgeSocketConfig};
use exyonq_module_api::kernel_control::{
    ControlPlaneService, KernelControlPort, OpsCommand, OpsCommandOutcome,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

/// Parse one control line into a known command (case-insensitive).
pub fn parse_command_line(line: &str) -> Result<OpsCommand, String> {
    match line.trim().to_ascii_lowercase().as_str() {
        "reload" => Ok(OpsCommand::Reload),
        "status" => Ok(OpsCommand::Status),
        "drain" => Ok(OpsCommand::Drain),
        "shutdown" => Ok(OpsCommand::Shutdown),
        "" => Err("empty command".into()),
        other => Err(format!("unknown command: {other}")),
    }
}

#[derive(Serialize)]
struct ControlResponseJson<'a> {
    ok: bool,
    command: &'static str,
    generation: u64,
    fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
    uptime_s: u64,
    active_connections: u64,
    draining: bool,
    reload_in_progress: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<&'a str>,
}

fn command_label(command: OpsCommand) -> &'static str {
    match command {
        OpsCommand::Reload => "reload",
        OpsCommand::Status => "status",
        OpsCommand::Drain => "drain",
        OpsCommand::Shutdown => "shutdown",
    }
}

fn to_json(outcome: &OpsCommandOutcome) -> ControlResponseJson<'_> {
    ControlResponseJson {
        ok: outcome.ok,
        command: command_label(outcome.command),
        generation: outcome.snapshot.generation,
        fingerprint: outcome.snapshot.fingerprint.clone(),
        error: outcome.error.clone(),
        code: outcome.code.as_deref(),
        uptime_s: outcome.snapshot.uptime_s,
        active_connections: outcome.snapshot.active_connections,
        draining: outcome.snapshot.draining,
        reload_in_progress: outcome.snapshot.reload_in_progress,
        version: outcome.snapshot.version.as_deref(),
    }
}

async fn dispatch_command(
    command: OpsCommand,
    config_path: &Path,
    port: &Arc<dyn KernelControlPort>,
) -> OpsCommandOutcome {
    match command {
        OpsCommand::Reload => port.request_reload(config_path).await,
        OpsCommand::Status => port.read_status(true),
        OpsCommand::Drain => port.request_drain(),
        OpsCommand::Shutdown => port.request_shutdown(),
    }
}

#[derive(Serialize)]
struct UnknownControlResponse<'a> {
    ok: bool,
    command: &'static str,
    generation: u64,
    fingerprint: String,
    error: String,
    uptime_s: u64,
    active_connections: u64,
    draining: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<&'a str>,
}

fn unknown_json(outcome: &OpsCommandOutcome) -> UnknownControlResponse<'_> {
    UnknownControlResponse {
        ok: false,
        command: "unknown",
        generation: outcome.snapshot.generation,
        fingerprint: outcome.snapshot.fingerprint.clone(),
        error: outcome.error.clone().unwrap_or_default(),
        uptime_s: outcome.snapshot.uptime_s,
        active_connections: outcome.snapshot.active_connections,
        draining: outcome.snapshot.draining,
        version: outcome.snapshot.version.as_deref(),
    }
}

async fn handle_client(
    mut stream: UnixStream,
    config_path: PathBuf,
    port: Arc<dyn KernelControlPort>,
) -> anyhow::Result<()> {
    let mut lines = BufReader::new(&mut stream);
    let mut command = String::new();
    lines.read_line(&mut command).await?;

    let parsed = parse_command_line(&command);
    let is_unknown = parsed.is_err();
    let outcome = match parsed {
        Ok(cmd) => dispatch_command(cmd, &config_path, &port).await,
        Err(err) => {
            let mut base = port.read_status(false);
            base.ok = false;
            base.error = Some(err);
            base
        }
    };

    let payload = if is_unknown {
        serde_json::to_string(&unknown_json(&outcome))? + "\n"
    } else {
        serde_json::to_string(&to_json(&outcome))? + "\n"
    };
    stream.write_all(payload.as_bytes()).await?;
    Ok(())
}

pub async fn run_control_socket(
    socket_path: &Path,
    config_path: &Path,
    port: Arc<dyn KernelControlPort>,
) -> anyhow::Result<()> {
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
    if let Some(parent) = socket_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let listener = UnixListener::bind(socket_path)?;
    info!(path = %socket_path.display(), "control socket listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let config_path = config_path.to_path_buf();
        let port = Arc::clone(&port);
        tokio::spawn(async move {
            if let Err(err) = handle_client(stream, config_path, port).await {
                warn!(%err, "control client error");
            }
        });
    }
}

pub struct UnixControlPlane;

impl ControlPlaneService for UnixControlPlane {
    fn spawn_unix_control_socket(
        &self,
        socket_path: PathBuf,
        config_path: PathBuf,
        port: Arc<dyn KernelControlPort>,
    ) {
        tokio::spawn(async move {
            if let Err(err) = run_control_socket(&socket_path, &config_path, port).await {
                warn!(path = %socket_path.display(), %err, "control socket stopped");
            }
        });
    }

    fn spawn_unix_cache_purge_socket(
        &self,
        config: CachePurgeSocketConfig,
        port: Arc<dyn CachePurgePort>,
    ) {
        crate::purge_socket::spawn_cache_purge_socket(config, port);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use exyonq_module_api::kernel_control::KernelStatusSnapshot;

    struct StubPort;

    #[async_trait]
    impl KernelControlPort for StubPort {
        async fn request_reload(&self, _config_path: &Path) -> OpsCommandOutcome {
            OpsCommandOutcome {
                ok: true,
                command: OpsCommand::Reload,
                snapshot: KernelStatusSnapshot {
                    generation: 1,
                    fingerprint: "fp".into(),
                    uptime_s: 0,
                    active_connections: 0,
                    draining: false,
                    version: None,
                    reload_in_progress: false,
                },
                error: None,
                code: None,
            }
        }

        fn request_drain(&self) -> OpsCommandOutcome {
            self.read_status(false)
        }

        fn request_shutdown(&self) -> OpsCommandOutcome {
            self.read_status(false)
        }

        fn read_status(&self, include_version: bool) -> OpsCommandOutcome {
            OpsCommandOutcome {
                ok: true,
                command: OpsCommand::Status,
                snapshot: KernelStatusSnapshot {
                    generation: 1,
                    fingerprint: "fp".into(),
                    uptime_s: 1,
                    active_connections: 2,
                    draining: false,
                    version: if include_version {
                        Some("test".into())
                    } else {
                        None
                    },
                    reload_in_progress: false,
                },
                error: None,
                code: None,
            }
        }
    }

    #[test]
    fn parses_known_commands() {
        assert_eq!(parse_command_line("reload\n").unwrap(), OpsCommand::Reload);
        assert_eq!(parse_command_line("STATUS").unwrap(), OpsCommand::Status);
        assert_eq!(parse_command_line(" drain ").unwrap(), OpsCommand::Drain);
        assert!(parse_command_line("nope").is_err());
    }

    #[tokio::test]
    async fn unknown_command_maps_to_error_outcome() {
        let port = Arc::new(StubPort);
        let mut base = port.read_status(false);
        base.ok = false;
        base.error = Some("unknown command: foo".into());
        assert!(!base.ok);
        let json = unknown_json(&base);
        assert_eq!(json.command, "unknown");
    }

    #[tokio::test]
    async fn control_json_includes_stable_code_when_present() {
        let outcome = OpsCommandOutcome::from_parts(
            false,
            OpsCommand::Reload,
            KernelStatusSnapshot {
                generation: 3,
                fingerprint: "fp".into(),
                uptime_s: 1,
                active_connections: 0,
                draining: false,
                version: None,
                reload_in_progress: false,
            },
            Some("EXY-RELOAD-0002: validation rejected".into()),
            None,
        );
        assert!(outcome.code_result_consistent());
        let json = to_json(&outcome);
        assert_eq!(json.code, Some("EXY-RELOAD-0002"));
        assert!(!json.ok);
        let body = serde_json::to_string(&json).expect("serialize");
        assert!(body.contains("\"code\":\"EXY-RELOAD-0002\""));
        assert!(!body.contains("password"));
        assert!(!body.contains("secret"));
    }
}
