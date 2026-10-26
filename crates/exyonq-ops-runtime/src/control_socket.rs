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
//!
//! Cap063 owns server-side AF_UNIX transport: bind safety, permissions,
//! bounded line framing, and per-client isolation. Command semantics remain
//! with Cap013/040/041/048/etc.

use exyonq_module_api::cache_purge::{CachePurgePort, CachePurgeSocketConfig};
use exyonq_module_api::kernel_control::{
    ControlPlaneService, KernelControlPort, MemberRequest, OpsCommand, OpsCommandOutcome,
};
use serde::Serialize;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;
use tracing::{info, warn};

/// Cap052 client timeout is 30s; server applies the same bound per connection I/O.
pub const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum accepted control command line including trailing newline.
pub const CONTROL_MAX_COMMAND_BYTES: usize = 4096;

/// Filesystem mode after bind (owner read/write only).
pub const CONTROL_SOCKET_MODE: u32 = 0o600;

/// Parse one control line into a known command (case-insensitive).
pub fn parse_command_line(line: &str) -> Result<OpsCommand, String> {
    let line = line.trim();
    let lower = line.to_ascii_lowercase();
    match lower.as_str() {
        "reload" => Ok(OpsCommand::Reload),
        "status" => Ok(OpsCommand::Status),
        "drain" => Ok(OpsCommand::Drain),
        "shutdown" => Ok(OpsCommand::Shutdown),
        "" => Err("empty command".into()),
        _ if lower.starts_with("member ") => {
            let req = parse_member_args(&line["member ".len()..])?;
            Ok(if req.endpoint_id.is_some() {
                OpsCommand::MemberSet
            } else {
                OpsCommand::MemberList
            })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

/// AP-01: parse `member list [upstream=<name>]` /
/// `member set <id> [upstream=<name>] [enabled=<bool>|status=<E|D>] [weight=<u32>|loadfactor=<u32>]`.
pub fn parse_member_args(args: &str) -> Result<MemberRequest, String> {
    let mut req = MemberRequest::default();
    let mut toks = args.split_whitespace();
    let verb = toks.next().unwrap_or("");
    match verb {
        "list" => {
            for tok in toks {
                req.upstream = Some(
                    tok.strip_prefix("upstream=")
                        .ok_or_else(|| format!("unknown member arg: {tok}"))?
                        .to_string(),
                );
            }
            return Ok(req);
        }
        "set" => {
            req.endpoint_id = Some(
                toks.next()
                    .ok_or("member set requires <endpoint_id>")?
                    .to_string(),
            );
        }
        other => return Err(format!("unknown member verb: {other}")),
    }
    for tok in toks {
        let (key, val) = tok
            .split_once('=')
            .ok_or_else(|| format!("expected key=value: {tok}"))?;
        match key.to_ascii_lowercase().as_str() {
            "upstream" => req.upstream = Some(val.to_string()),
            "enabled" => {
                req.enabled = Some(match val.to_ascii_lowercase().as_str() {
                    "true" | "1" | "on" | "enabled" => true,
                    "false" | "0" | "off" | "disabled" => false,
                    _ => return Err(format!("invalid enabled value: {val}")),
                })
            }
            "status" => {
                req.enabled = Some(match val.to_ascii_uppercase().as_str() {
                    "E" | "ENABLED" => true,
                    "D" | "DISABLED" => false,
                    _ => return Err(format!("unsupported status: {val}")),
                })
            }
            "weight" | "loadfactor" => {
                req.weight = Some(
                    val.parse::<u32>()
                        .map_err(|_| format!("invalid weight: {val}"))?,
                )
            }
            "route" => {
                return Err(
                    "route is config-time endpoint identity; change it in config + reload".into(),
                )
            }
            other => return Err(format!("unknown member key: {other}")),
        }
    }
    if req.enabled.is_none() && req.weight.is_none() {
        return Err(
            "member set requires at least one of enabled=/status=/weight=/loadfactor=".into(),
        );
    }
    Ok(req)
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
    /// AP-01: member rows for member_list/member_set outcomes.
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
    /// Q-09: epoch notAfter of the primary TLS leaf cert, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    tls_cert_not_after_epoch: Option<u64>,
}

fn command_label(command: OpsCommand) -> &'static str {
    match command {
        OpsCommand::Reload => "reload",
        OpsCommand::Status => "status",
        OpsCommand::Drain => "drain",
        OpsCommand::Shutdown => "shutdown",
        OpsCommand::MemberList => "member_list",
        OpsCommand::MemberSet => "member_set",
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
        payload: outcome
            .payload
            .as_deref()
            .and_then(|p| serde_json::from_str(p).ok()),
        tls_cert_not_after_epoch: outcome.snapshot.tls_cert_not_after_epoch,
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
        OpsCommand::MemberList | OpsCommand::MemberSet => unreachable!(),
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

fn unknown_json<'a>(
    outcome: &'a OpsCommandOutcome,
    command: &'static str,
) -> UnknownControlResponse<'a> {
    UnknownControlResponse {
        ok: false,
        command,
        generation: outcome.snapshot.generation,
        fingerprint: outcome.snapshot.fingerprint.clone(),
        error: outcome.error.clone().unwrap_or_default(),
        uptime_s: outcome.snapshot.uptime_s,
        active_connections: outcome.snapshot.active_connections,
        draining: outcome.snapshot.draining,
        version: outcome.snapshot.version.as_deref(),
    }
}

fn chmod_owner_only(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(CONTROL_SOCKET_MODE))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Cap063 LA-001: create the socket inode under umask 0o177 so mode is 0o600
/// at bind time — no cross-UID connect window before a later chmod.
#[cfg(unix)]
fn bind_unix_listener_owner_mode(socket_path: &Path) -> std::io::Result<UnixListener> {
    use std::sync::Mutex;
    static UMASK_LOCK: Mutex<()> = Mutex::new(());

    // mode_t width differs: Linux u32, Darwin u16.
    #[cfg(target_os = "macos")]
    type ModeT = u16;
    #[cfg(not(target_os = "macos"))]
    type ModeT = u32;

    unsafe extern "C" {
        fn umask(mask: ModeT) -> ModeT;
    }

    let _guard = UMASK_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Socket create mask 0777 & ~0177 = 0600.
    let previous = unsafe { umask(0o177) };
    let bound = UnixListener::bind(socket_path);
    unsafe {
        umask(previous);
    }
    bound
}

#[cfg(not(unix))]
fn bind_unix_listener_owner_mode(socket_path: &Path) -> std::io::Result<UnixListener> {
    UnixListener::bind(socket_path)
}

/// True when a connect to `path` succeeds (live listener owns the name).
fn path_has_live_listener(path: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(path).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Cap063 bind safety:
/// - missing path → OK (bind creates)
/// - directory / regular file / non-socket → refuse (never unlink foreign objects)
/// - live Unix socket (connect succeeds) → refuse (never steal active instance)
/// - stale Unix socket / dangling symlink entry → unlink then bind
///
/// Uses `symlink_metadata` so a symlink is classified without following.
pub fn prepare_control_socket_path(socket_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = socket_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let meta = match std::fs::symlink_metadata(socket_path) {
        Ok(m) => m,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };

    let ft = meta.file_type();
    if ft.is_dir() {
        anyhow::bail!(
            "control socket path is a directory (refusing to mutate): {}",
            socket_path.display()
        );
    }
    if ft.is_file() {
        anyhow::bail!(
            "control socket path is a regular file (refusing to unlink): {}",
            socket_path.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if ft.is_socket() || ft.is_symlink() {
            if path_has_live_listener(socket_path) {
                anyhow::bail!(
                    "control socket path is in use by a live listener: {}",
                    socket_path.display()
                );
            }
            // Stale socket or dangling/unusable symlink entry — remove the directory
            // entry only (`remove_file` does not follow symlinks).
            std::fs::remove_file(socket_path).map_err(|err| {
                anyhow::anyhow!(
                    "failed to remove stale control socket {}: {err}",
                    socket_path.display()
                )
            })?;
            return Ok(());
        }
        anyhow::bail!(
            "control socket path exists and is not a Unix socket: {}",
            socket_path.display()
        );
    }

    #[cfg(not(unix))]
    {
        anyhow::bail!(
            "control socket path exists and cannot be reused safely: {}",
            socket_path.display()
        );
    }
}

async fn read_command_line(stream: &mut UnixStream) -> anyhow::Result<Result<String, String>> {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::with_capacity(64);
    // Cap take at MAX+1 so an oversize line is detectable without unbounded growth.
    let mut limited = (&mut reader).take((CONTROL_MAX_COMMAND_BYTES as u64) + 1);
    let read = timeout(CONTROL_IO_TIMEOUT, limited.read_until(b'\n', &mut buf))
        .await
        .map_err(|_| anyhow::anyhow!("control command read timeout"))?
        .map_err(|err| anyhow::anyhow!("control command read error: {err}"))?;

    if read == 0 {
        return Ok(Err("empty connection".into()));
    }
    if buf.len() > CONTROL_MAX_COMMAND_BYTES || !buf.ends_with(b"\n") {
        return Ok(Err(format!(
            "command too long (max {CONTROL_MAX_COMMAND_BYTES} bytes) or missing newline"
        )));
    }

    let line = String::from_utf8_lossy(&buf).into_owned();
    Ok(Ok(line))
}

async fn write_response(stream: &mut UnixStream, payload: &str) -> anyhow::Result<()> {
    timeout(CONTROL_IO_TIMEOUT, stream.write_all(payload.as_bytes()))
        .await
        .map_err(|_| anyhow::anyhow!("control response write timeout"))?
        .map_err(|err| anyhow::anyhow!("control response write error: {err}"))?;
    Ok(())
}

async fn handle_client(
    mut stream: UnixStream,
    config_path: PathBuf,
    port: Arc<dyn KernelControlPort>,
) -> anyhow::Result<()> {
    let line_result = read_command_line(&mut stream).await?;
    let command_line = match line_result {
        Ok(line) => line,
        Err(err) => {
            let mut base = port.read_status(false);
            base.ok = false;
            base.error = Some(err);
            let payload = serde_json::to_string(&unknown_json(&base, "unknown"))? + "\n";
            // LET-197 / SUB-LET-CTRL-WRITE-FALSE-OK: handle_client's Result includes
            // control response delivery. Discarding write_response Err then returning
            // Ok(()) reported false success while the client never received the reply.
            write_response(&mut stream, &payload).await?;
            return Ok(());
        }
    };

    let parsed = parse_command_line(&command_line);
    let is_unknown = parsed.is_err();
    // Keep the member verb in the error label instead of a bare "unknown".
    let mut tokens = command_line.split_whitespace();
    let error_label: &'static str = match (tokens.next(), tokens.next()) {
        (Some("member"), Some("list")) => "member_list",
        (Some("member"), Some("set")) => "member_set",
        _ => "unknown",
    };
    let outcome = match parsed {
        Ok(cmd @ (OpsCommand::MemberList | OpsCommand::MemberSet)) => {
            match parse_member_args(command_line.trim()["member ".len()..].trim()) {
                Ok(req) if cmd == OpsCommand::MemberList => {
                    port.member_list(req.upstream.as_deref())
                }
                Ok(req) => port.member_set(&req),
                Err(err) => {
                    let mut base = port.read_status(false);
                    base.ok = false;
                    base.error = Some(err);
                    base
                }
            }
        }
        Ok(cmd) => dispatch_command(cmd, &config_path, &port).await,
        Err(err) => {
            let mut base = port.read_status(false);
            base.ok = false;
            base.error = Some(err);
            base
        }
    };

    let payload = if is_unknown {
        serde_json::to_string(&unknown_json(&outcome, error_label))? + "\n"
    } else {
        serde_json::to_string(&to_json(&outcome))? + "\n"
    };
    write_response(&mut stream, &payload).await?;
    Ok(())
}

#[cfg(test)]
pub async fn run_control_socket(
    socket_path: &Path,
    config_path: &Path,
    port: Arc<dyn KernelControlPort>,
) -> anyhow::Result<()> {
    let bound = bind_control_listener(socket_path)?;
    run_control_accept_loop(bound, config_path, port).await
}

/// Synchronous bind with owner-only mode at creation time.
///
/// Cap063: umask-scoped bind closes the bind→chmod race; chmod confirms mode.
/// chmod failure unlinks the path and fails closed (LA-CAP063-003).
pub fn bind_control_listener(socket_path: &Path) -> anyhow::Result<BoundControlSocket> {
    prepare_control_socket_path(socket_path)?;
    let listener = bind_unix_listener_owner_mode(socket_path).map_err(|err| {
        anyhow::anyhow!(
            "failed to bind control socket {}: {err}",
            socket_path.display()
        )
    })?;
    if let Err(err) = chmod_owner_only(socket_path) {
        drop(listener);
        let _ = std::fs::remove_file(socket_path);
        return Err(anyhow::anyhow!(
            "chmod owner-only on control socket {}: {err}",
            socket_path.display()
        ));
    }
    info!(
        path = %socket_path.display(),
        mode = format!("0o{CONTROL_SOCKET_MODE:o}"),
        "control socket listening"
    );
    Ok(BoundControlSocket {
        listener: Some(listener),
        path: socket_path.to_path_buf(),
    })
}

/// Owns the listening FD and unlinks the path only after the listener is closed
/// (LA-CAP063-002: never unlink while still accepting).
pub struct BoundControlSocket {
    listener: Option<UnixListener>,
    path: PathBuf,
}

impl BoundControlSocket {
    fn listener(&self) -> &UnixListener {
        self.listener
            .as_ref()
            .expect("BoundControlSocket listener present")
    }
}

impl Drop for BoundControlSocket {
    fn drop(&mut self) {
        drop(self.listener.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn run_control_accept_loop(
    bound: BoundControlSocket,
    config_path: &Path,
    port: Arc<dyn KernelControlPort>,
) -> anyhow::Result<()> {
    loop {
        let accepted = bound.listener().accept().await;
        let (stream, _) = match accepted {
            Ok(pair) => pair,
            Err(err) => {
                warn!(%err, "control socket accept error");
                tokio::task::yield_now().await;
                continue;
            }
        };
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
    ) -> Result<(), String> {
        let bound = bind_control_listener(&socket_path).map_err(|err| err.to_string())?;
        tokio::spawn(async move {
            if let Err(err) = run_control_accept_loop(bound, &config_path, port).await {
                warn!(path = %socket_path.display(), %err, "control socket stopped");
            }
        });
        Ok(())
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
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener as StdUnixListener;

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
                    tls_cert_not_after_epoch: None,
                },
                error: None,
                code: None,
                payload: None,
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
                    tls_cert_not_after_epoch: None,
                },
                error: None,
                code: None,
                payload: None,
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

    #[test]
    fn parses_member_commands() {
        assert_eq!(
            parse_command_line("member list").unwrap(),
            OpsCommand::MemberList
        );
        assert_eq!(
            parse_command_line("member list upstream=backend").unwrap(),
            OpsCommand::MemberList
        );
        assert_eq!(
            parse_command_line("member set ep-b weight=0").unwrap(),
            OpsCommand::MemberSet
        );
        assert_eq!(
            parse_command_line("member set ep-a upstream=up enabled=false weight=3").unwrap(),
            OpsCommand::MemberSet
        );
        assert_eq!(
            parse_command_line("member set ep-a status=D loadfactor=5").unwrap(),
            OpsCommand::MemberSet
        );
        assert!(parse_command_line("member").is_err());
        assert!(parse_command_line("member set ep-a").is_err()); // no field pairs
        assert!(parse_command_line("member frobnicate ep-a weight=1").is_err());
        assert!(parse_command_line("member set ep-a weight=abc").is_err());
        assert!(parse_command_line("member set ep-a route=x").is_err()); // unsupported key

        let req = parse_member_args("set ep-a upstream=up enabled=off loadfactor=7").unwrap();
        assert_eq!(req.endpoint_id.as_deref(), Some("ep-a"));
        assert_eq!(req.upstream.as_deref(), Some("up"));
        assert_eq!(req.enabled, Some(false));
        assert_eq!(req.weight, Some(7));

        let req = parse_member_args("list").unwrap();
        assert!(req.endpoint_id.is_none());
        assert!(req.upstream.is_none());
    }

    #[tokio::test]
    async fn unknown_command_maps_to_error_outcome() {
        let port = Arc::new(StubPort);
        let mut base = port.read_status(false);
        base.ok = false;
        base.error = Some("unknown command: foo".into());
        assert!(!base.ok);
        let json = unknown_json(&base, "unknown");
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
                tls_cert_not_after_epoch: None,
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

    #[test]
    fn prepare_refuses_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        std::fs::write(&path, b"not-a-socket").unwrap();
        let err = prepare_control_socket_path(&path).unwrap_err().to_string();
        assert!(err.contains("regular file"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"not-a-socket");
    }

    #[test]
    fn prepare_refuses_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        std::fs::create_dir(&path).unwrap();
        let err = prepare_control_socket_path(&path).unwrap_err().to_string();
        assert!(err.contains("directory"), "{err}");
        assert!(path.is_dir());
    }

    #[test]
    fn prepare_refuses_live_listener() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let _listener = StdUnixListener::bind(&path).unwrap();
        let err = prepare_control_socket_path(&path).unwrap_err().to_string();
        assert!(err.contains("live listener"), "{err}");
        assert!(path.exists());
    }

    #[test]
    fn prepare_removes_stale_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let listener = StdUnixListener::bind(&path).unwrap();
        drop(listener);
        assert!(path.exists());
        prepare_control_socket_path(&path).unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn bind_sets_owner_only_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let port: Arc<dyn KernelControlPort> = Arc::new(StubPort);
        let path_clone = path.clone();
        let handle = tokio::spawn(async move {
            let _ = run_control_socket(&path_clone, Path::new("/tmp/cfg.toml"), port).await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, CONTROL_SOCKET_MODE, "mode={mode:#o}");
        handle.abort();
        let _ = handle.await;
    }

    fn oversize_command_line() -> Vec<u8> {
        let mut req = vec![b'x'; CONTROL_MAX_COMMAND_BYTES + 8];
        req.extend_from_slice(b"\n");
        req
    }

    /// Oversize line → error JSON reply is delivered; handle_client completes Ok.
    #[tokio::test]
    async fn oversize_command_delivers_error_json_ok() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let bound = bind_control_listener(&path).unwrap();
        let port: Arc<dyn KernelControlPort> = Arc::new(StubPort);
        let accept = tokio::spawn(async move {
            let (stream, _) = bound.listener().accept().await.expect("accept");
            handle_client(stream, PathBuf::from("/tmp/cfg.toml"), port).await
        });
        let mut client = None;
        for _ in 0..100 {
            match UnixStream::connect(&path).await {
                Ok(s) => {
                    client = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        let mut client = client.expect("connect control socket");
        client.write_all(&oversize_command_line()).await.unwrap();
        let mut buf = vec![0u8; 2048];
        let n = tokio::time::timeout(Duration::from_secs(2), client.read(&mut buf))
            .await
            .expect("read timeout")
            .expect("read");
        assert!(n > 0, "expected error JSON bytes");
        let text = String::from_utf8_lossy(&buf[..n]);
        assert!(
            text.contains("\"ok\":false"),
            "expected ok:false, got {text}"
        );
        let res = accept.await.expect("join");
        assert!(
            res.is_ok(),
            "delivered error reply must complete Ok: {res:?}"
        );
    }

    /// LET-197: peer gone after oversize framing Err, before/during error-reply write →
    /// handle_client must return Err (not Ok(())). Real AF_UNIX socketpair (not a mock writer).
    #[tokio::test]
    async fn error_reply_write_failure_returns_err_not_ok() {
        use tokio::io::AsyncWriteExt;
        let port: Arc<dyn KernelControlPort> = Arc::new(StubPort);
        let mut observed_err = false;
        for _ in 0..40 {
            let (mut client, server) = UnixStream::pair().expect("unix socketpair");
            let port = Arc::clone(&port);
            let handle = tokio::spawn(async move {
                handle_client(server, PathBuf::from("/tmp/cfg.toml"), port).await
            });
            // Reach the sealed arm via oversize → Ok(Err(...)), then drop so write_response fails.
            let _ = client.write_all(&oversize_command_line()).await;
            drop(client);
            let res = tokio::time::timeout(Duration::from_secs(2), handle)
                .await
                .expect("join timeout")
                .expect("join");
            match res {
                Err(err) => {
                    let msg = format!("{err:#}");
                    assert!(
                        msg.contains("control response write") || msg.contains("write"),
                        "expected write-class Err, got: {msg}"
                    );
                    observed_err = true;
                    break;
                }
                Ok(()) => continue,
            }
        }
        assert!(
            observed_err,
            "LET-197: write_response failure must return Err, not Ok(())"
        );
    }
}
