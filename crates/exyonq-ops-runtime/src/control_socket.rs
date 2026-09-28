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
//!
//! # Filesystem threat boundary
//!
//! Cleanup is descriptor-relative to a canonical, opened parent and requires
//! either an euid-owned non-group/world-writable directory or an euid/root-owned
//! sticky directory (including `/tmp`). Every existing final entry, lease,
//! stage, and deleted socket must be owned by the current euid. Atomic
//! no-replace/exchange rename quarantines candidates before deletion, so a
//! cross-uid process cannot make cleanup unlink a replacement inode.
//!
//! A mode-0600 `flock` lease serializes cooperating same-uid ExyonQ processes
//! for the socket lifetime. An arbitrary hostile process already running under
//! the same uid can bypass advisory locks and owner-only directory permissions;
//! defending that principal requires OS sandboxing or a distinct service uid.
//! Filesystems must provide atomic same-filesystem exchange and no-replace
//! rename (`renameat2` on Linux, `renameatx_np` on macOS). When unavailable,
//! cleanup fails closed and leaves the socket/artifact for diagnosis.

use exyonq_module_api::cache_purge::{CachePurgePort, CachePurgeSocketConfig};
use exyonq_module_api::kernel_control::{
    ControlPlaneService, KernelControlPort, OpsCommand, OpsCommandOutcome,
};
use serde::Serialize;
use std::ffi::CString;
use std::fs::{DirBuilder, File, Metadata};
use std::io::ErrorKind;
use std::os::fd::{AsFd, AsRawFd};
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

const CONTROL_STAGE_DIR_MODE: u32 = 0o700;
const CONTROL_STAGE_PREFIX: &str = ".exq-";
const CONTROL_STAGE_SOCKET_NAME: &str = "s";
const CONTROL_LEASE_MODE: u32 = 0o600;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
}

fn file_identity(metadata: &Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;

    FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    }
}

fn entry_identity(metadata: exyonq_linux_ffi::EntryMetadata) -> FileIdentity {
    FileIdentity {
        dev: metadata.dev,
        ino: metadata.ino,
    }
}

fn set_and_verify_mode(path: &Path, expected: u32, object: &str) -> anyhow::Result<Metadata> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(expected)).map_err(|err| {
        anyhow::anyhow!("chmod {object} {} to {expected:#o}: {err}", path.display())
    })?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| anyhow::anyhow!("lstat {object} {} after chmod: {err}", path.display()))?;
    let actual = metadata.permissions().mode() & 0o777;
    if actual != expected {
        anyhow::bail!(
            "{object} {} mode verification failed: expected {expected:#o}, got {actual:#o}",
            path.display()
        );
    }
    Ok(metadata)
}

struct ControlSocketTarget {
    parent_path: PathBuf,
    parent: File,
    parent_identity: FileIdentity,
    final_path: PathBuf,
    final_name: CString,
    effective_uid: u32,
    _lease: File,
}

impl ControlSocketTarget {
    fn open(socket_path: &Path) -> anyhow::Result<Self> {
        use std::os::unix::ffi::OsStrExt;

        let parent = socket_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|err| {
            anyhow::anyhow!("create control socket parent {}: {err}", parent.display())
        })?;
        let parent_path = std::fs::canonicalize(parent).map_err(|err| {
            anyhow::anyhow!(
                "canonicalize control socket parent {}: {err}",
                parent.display()
            )
        })?;
        let parent_file = File::open(&parent_path).map_err(|err| {
            anyhow::anyhow!(
                "open control socket parent {}: {err}",
                parent_path.display()
            )
        })?;
        let parent_metadata = parent_file.metadata().map_err(|err| {
            anyhow::anyhow!(
                "inspect opened control socket parent {}: {err}",
                parent_path.display()
            )
        })?;
        if !parent_metadata.is_dir() {
            anyhow::bail!(
                "control socket parent is not a directory: {}",
                parent_path.display()
            );
        }
        let effective_uid = exyonq_linux_ffi::effective_uid();
        let (parent_uid, parent_mode) = {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            (
                parent_metadata.uid(),
                parent_metadata.permissions().mode() & 0o7777,
            )
        };
        let group_or_world_writable = parent_mode & 0o022 != 0;
        let sticky = parent_mode & 0o1000 != 0;
        let private_owned = parent_uid == effective_uid && !group_or_world_writable;
        let trusted_sticky = sticky && (parent_uid == effective_uid || parent_uid == 0);
        if !private_owned && !trusted_sticky {
            anyhow::bail!(
                "unsafe control socket parent {}: uid={parent_uid}, euid={effective_uid}, mode={parent_mode:#o}; require an euid-owned non-group/world-writable directory or an euid/root-owned sticky directory",
                parent_path.display()
            );
        }
        let final_component = socket_path.file_name().ok_or_else(|| {
            anyhow::anyhow!(
                "control socket path has no final component: {}",
                socket_path.display()
            )
        })?;
        let final_name = CString::new(final_component.as_bytes()).map_err(|_| {
            anyhow::anyhow!(
                "control socket filename contains NUL: {}",
                socket_path.display()
            )
        })?;
        let mut lease_bytes = Vec::with_capacity(final_component.as_bytes().len() + 24);
        lease_bytes.extend_from_slice(b".exyonq-");
        lease_bytes.extend_from_slice(final_component.as_bytes());
        lease_bytes.extend_from_slice(b".lock");
        let lease_name = CString::new(lease_bytes).map_err(|_| {
            anyhow::anyhow!(
                "control socket lease filename contains NUL: {}",
                socket_path.display()
            )
        })?;
        let lease = exyonq_linux_ffi::open_or_create_lease_at(
            parent_file.as_fd(),
            &lease_name,
            CONTROL_LEASE_MODE,
        )
        .map_err(|err| {
            anyhow::anyhow!(
                "open control socket ownership lease in {}: {err}",
                parent_path.display()
            )
        })?;
        exyonq_linux_ffi::set_fd_mode(lease.as_fd(), CONTROL_LEASE_MODE).map_err(|err| {
            anyhow::anyhow!(
                "set control socket ownership lease mode in {}: {err}",
                parent_path.display()
            )
        })?;
        let lease_metadata = lease.metadata().map_err(|err| {
            anyhow::anyhow!(
                "inspect control socket ownership lease in {}: {err}",
                parent_path.display()
            )
        })?;
        let (lease_uid, lease_mode) = {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            (
                lease_metadata.uid(),
                lease_metadata.permissions().mode() & 0o777,
            )
        };
        if !lease_metadata.is_file()
            || lease_uid != effective_uid
            || lease_mode != CONTROL_LEASE_MODE
        {
            anyhow::bail!(
                "unsafe control socket ownership lease in {}: uid={lease_uid}, euid={effective_uid}, mode={lease_mode:#o}",
                parent_path.display()
            );
        }
        if !exyonq_linux_ffi::try_flock_exclusive(lease.as_raw_fd()).map_err(|err| {
            anyhow::anyhow!(
                "lock control socket ownership lease in {}: {err}",
                parent_path.display()
            )
        })? {
            anyhow::bail!(
                "control socket ownership lease is held by another process: {}",
                socket_path.display()
            );
        }
        let target = Self {
            parent_path: parent_path.clone(),
            parent: parent_file,
            parent_identity: file_identity(&parent_metadata),
            final_path: parent_path.join(final_component),
            final_name,
            effective_uid,
            _lease: lease,
        };
        target.verify_parent_identity()?;
        Ok(target)
    }

    fn verify_parent_identity(&self) -> anyhow::Result<()> {
        let metadata = std::fs::symlink_metadata(&self.parent_path).map_err(|err| {
            anyhow::anyhow!(
                "lstat control socket parent {}: {err}",
                self.parent_path.display()
            )
        })?;
        if !metadata.is_dir() || file_identity(&metadata) != self.parent_identity {
            anyhow::bail!(
                "control socket parent changed while binding: {}",
                self.parent_path.display()
            );
        }
        Ok(())
    }

    fn entry_metadata(&self) -> std::io::Result<exyonq_linux_ffi::EntryMetadata> {
        exyonq_linux_ffi::metadata_at_nofollow(self.parent.as_fd(), &self.final_name)
    }

    fn verify_entry_owner(&self, metadata: exyonq_linux_ffi::EntryMetadata) -> anyhow::Result<()> {
        if metadata.uid != self.effective_uid {
            anyhow::bail!(
                "control socket entry {} is owned by uid {}, expected euid {}",
                self.final_path.display(),
                metadata.uid,
                self.effective_uid
            );
        }
        Ok(())
    }
}

struct SocketStage {
    dir_path: PathBuf,
    socket_path: PathBuf,
    relative_socket: CString,
    dir_identity: FileIdentity,
    socket_identity: Option<FileIdentity>,
}

impl SocketStage {
    fn create(target: &ControlSocketTarget) -> anyhow::Result<Self> {
        use std::fmt::Write as _;
        use std::os::unix::fs::DirBuilderExt;

        for _ in 0..64 {
            let mut random = [0_u8; 12];
            getrandom::fill(&mut random)
                .map_err(|err| anyhow::anyhow!("generate control socket staging name: {err}"))?;
            let mut name = String::with_capacity(CONTROL_STAGE_PREFIX.len() + 24);
            name.push_str(CONTROL_STAGE_PREFIX);
            for byte in random {
                write!(&mut name, "{byte:02x}")
                    .expect("writing hexadecimal bytes to String cannot fail");
            }

            let dir_path = target.parent_path.join(&name);
            let mut builder = DirBuilder::new();
            builder.mode(CONTROL_STAGE_DIR_MODE);
            match builder.create(&dir_path) {
                Ok(()) => {
                    let initial_metadata = std::fs::symlink_metadata(&dir_path).map_err(|err| {
                        let _ = std::fs::remove_dir(&dir_path);
                        anyhow::anyhow!(
                            "lstat new control socket staging directory {}: {err}",
                            dir_path.display()
                        )
                    })?;
                    if !initial_metadata.is_dir() {
                        anyhow::bail!(
                            "control socket staging path is not a directory: {}",
                            dir_path.display()
                        );
                    }
                    let socket_path = dir_path.join(CONTROL_STAGE_SOCKET_NAME);
                    let relative_socket =
                        CString::new(format!("{name}/{CONTROL_STAGE_SOCKET_NAME}").into_bytes())
                            .expect("generated staging path contains no NUL");
                    let mut stage = Self {
                        dir_path,
                        socket_path,
                        relative_socket,
                        dir_identity: file_identity(&initial_metadata),
                        socket_identity: None,
                    };
                    let metadata = set_and_verify_mode(
                        &stage.dir_path,
                        CONTROL_STAGE_DIR_MODE,
                        "staging directory",
                    )?;
                    stage.dir_identity = file_identity(&metadata);
                    target.verify_parent_identity()?;
                    return Ok(stage);
                }
                Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    return Err(anyhow::anyhow!(
                        "create private control socket staging directory {}: {err}",
                        dir_path.display()
                    ));
                }
            }
        }
        anyhow::bail!("could not allocate a unique control socket staging directory")
    }

    fn record_socket(&mut self, metadata: &Metadata) {
        self.socket_identity = Some(file_identity(metadata));
    }

    fn remove_source_and_dir(&mut self) -> anyhow::Result<()> {
        let Some(expected_socket) = self.socket_identity else {
            anyhow::bail!("control socket staging inode was not recorded");
        };
        let metadata = std::fs::symlink_metadata(&self.socket_path).map_err(|err| {
            anyhow::anyhow!(
                "lstat staged control socket {} before cleanup: {err}",
                self.socket_path.display()
            )
        })?;
        if !is_socket(&metadata) || file_identity(&metadata) != expected_socket {
            anyhow::bail!(
                "staged control socket changed before cleanup: {}",
                self.socket_path.display()
            );
        }
        std::fs::remove_file(&self.socket_path).map_err(|err| {
            anyhow::anyhow!(
                "remove staged control socket {}: {err}",
                self.socket_path.display()
            )
        })?;
        self.socket_identity = None;
        self.remove_dir()
    }

    fn remove_dir(&self) -> anyhow::Result<()> {
        let metadata = std::fs::symlink_metadata(&self.dir_path).map_err(|err| {
            anyhow::anyhow!(
                "lstat control socket staging directory {}: {err}",
                self.dir_path.display()
            )
        })?;
        if !metadata.is_dir() || file_identity(&metadata) != self.dir_identity {
            anyhow::bail!(
                "control socket staging directory changed before cleanup: {}",
                self.dir_path.display()
            );
        }
        std::fs::remove_dir(&self.dir_path).map_err(|err| {
            anyhow::anyhow!(
                "remove control socket staging directory {}: {err}",
                self.dir_path.display()
            )
        })
    }
}

impl Drop for SocketStage {
    fn drop(&mut self) {
        let Ok(dir_metadata) = std::fs::symlink_metadata(&self.dir_path) else {
            return;
        };
        if !dir_metadata.is_dir() || file_identity(&dir_metadata) != self.dir_identity {
            return;
        }
        match (
            self.socket_identity,
            std::fs::symlink_metadata(&self.socket_path),
        ) {
            (Some(expected_socket), Ok(socket_metadata))
                if is_socket(&socket_metadata)
                    && file_identity(&socket_metadata) == expected_socket =>
            {
                let _ = std::fs::remove_file(&self.socket_path);
            }
            _ => {}
        }
        let _ = std::fs::remove_dir(&self.dir_path);
    }
}

fn is_socket(metadata: &Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;

    metadata.file_type().is_socket()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlaceholderLocation {
    StageQuarantine,
    StageParking,
    Final,
    Gone,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CleanupPhase {
    BeforeExchange,
    AfterExchange,
    AfterCandidateVerified,
    AfterPlaceholderParked,
    AfterPlaceholderVerified,
    AfterCandidateDeleted,
}

struct CleanupStage {
    parent: File,
    final_name: CString,
    final_path: PathBuf,
    dir_name: CString,
    dir: File,
    dir_path: PathBuf,
    dir_identity: FileIdentity,
    _placeholder: File,
    placeholder_identity: FileIdentity,
    placeholder_location: PlaceholderLocation,
}

impl CleanupStage {
    fn create(target: &ControlSocketTarget) -> anyhow::Result<Self> {
        use std::fmt::Write as _;

        let cleanup_parent = target.parent.try_clone().map_err(|err| {
            anyhow::anyhow!(
                "clone control socket parent descriptor {}: {err}",
                target.parent_path.display()
            )
        })?;
        for _ in 0..64 {
            let mut random = [0_u8; 12];
            getrandom::fill(&mut random)
                .map_err(|err| anyhow::anyhow!("generate cleanup staging name: {err}"))?;
            let mut name = String::with_capacity(CONTROL_STAGE_PREFIX.len() + 24);
            name.push_str(CONTROL_STAGE_PREFIX);
            for byte in random {
                write!(&mut name, "{byte:02x}")
                    .expect("writing hexadecimal bytes to String cannot fail");
            }
            let dir_name =
                CString::new(name.as_bytes()).expect("generated staging name contains no NUL");
            match exyonq_linux_ffi::create_directory_at(
                target.parent.as_fd(),
                &dir_name,
                CONTROL_STAGE_DIR_MODE,
            ) {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    return Err(anyhow::anyhow!(
                        "create descriptor-relative cleanup stage in {}: {err}",
                        target.parent_path.display()
                    ));
                }
            }
            let dir = match exyonq_linux_ffi::open_directory_at(target.parent.as_fd(), &dir_name) {
                Ok(dir) => dir,
                Err(err) => {
                    let _ = exyonq_linux_ffi::unlink_directory_at(target.parent.as_fd(), &dir_name);
                    return Err(anyhow::anyhow!(
                        "open descriptor-relative cleanup stage in {}: {err}",
                        target.parent_path.display()
                    ));
                }
            };
            if let Err(err) = exyonq_linux_ffi::set_fd_mode(dir.as_fd(), CONTROL_STAGE_DIR_MODE) {
                let _ = exyonq_linux_ffi::unlink_directory_at(target.parent.as_fd(), &dir_name);
                return Err(anyhow::anyhow!(
                    "set cleanup stage mode in {}: {err}",
                    target.parent_path.display()
                ));
            }
            let dir_metadata = match dir.metadata() {
                Ok(metadata) => metadata,
                Err(err) => {
                    let _ = exyonq_linux_ffi::unlink_directory_at(target.parent.as_fd(), &dir_name);
                    return Err(anyhow::anyhow!(
                        "inspect cleanup stage in {}: {err}",
                        target.parent_path.display()
                    ));
                }
            };
            let (dir_uid, dir_mode) = {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                (
                    dir_metadata.uid(),
                    dir_metadata.permissions().mode() & 0o777,
                )
            };
            if !dir_metadata.is_dir()
                || dir_uid != target.effective_uid
                || dir_mode != CONTROL_STAGE_DIR_MODE
            {
                let _ = exyonq_linux_ffi::unlink_directory_at(target.parent.as_fd(), &dir_name);
                return Err(anyhow::anyhow!(
                    "unsafe cleanup stage in {}: uid={dir_uid}, euid={}, mode={dir_mode:#o}",
                    target.parent_path.display(),
                    target.effective_uid
                ));
            }
            let placeholder = match exyonq_linux_ffi::create_file_exclusive_at(
                dir.as_fd(),
                c"q",
                CONTROL_LEASE_MODE,
            ) {
                Ok(placeholder) => placeholder,
                Err(err) => {
                    let _ = exyonq_linux_ffi::unlink_directory_at(target.parent.as_fd(), &dir_name);
                    return Err(anyhow::anyhow!(
                        "create cleanup placeholder in {}: {err}",
                        target.parent_path.display()
                    ));
                }
            };
            let placeholder_metadata = match placeholder.metadata() {
                Ok(metadata) => metadata,
                Err(err) => {
                    let _ = exyonq_linux_ffi::unlink_file_at(dir.as_fd(), c"q");
                    let _ = exyonq_linux_ffi::unlink_directory_at(target.parent.as_fd(), &dir_name);
                    return Err(anyhow::anyhow!(
                        "inspect cleanup placeholder in {}: {err}",
                        target.parent_path.display()
                    ));
                }
            };
            let placeholder_identity = file_identity(&placeholder_metadata);
            if let Err(err) = exyonq_linux_ffi::set_fd_mode(placeholder.as_fd(), CONTROL_LEASE_MODE)
            {
                let _ = exyonq_linux_ffi::unlink_file_at(dir.as_fd(), c"q");
                let _ = exyonq_linux_ffi::unlink_directory_at(target.parent.as_fd(), &dir_name);
                return Err(anyhow::anyhow!(
                    "set cleanup placeholder mode in {}: {err}",
                    target.parent_path.display()
                ));
            }
            let verified_placeholder = match placeholder.metadata() {
                Ok(metadata) => metadata,
                Err(err) => {
                    let _ = exyonq_linux_ffi::unlink_file_at(dir.as_fd(), c"q");
                    let _ = exyonq_linux_ffi::unlink_directory_at(target.parent.as_fd(), &dir_name);
                    return Err(anyhow::anyhow!(
                        "verify cleanup placeholder in {}: {err}",
                        target.parent_path.display()
                    ));
                }
            };
            let (placeholder_uid, placeholder_mode) = {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                (
                    verified_placeholder.uid(),
                    verified_placeholder.permissions().mode() & 0o777,
                )
            };
            if !verified_placeholder.is_file()
                || file_identity(&verified_placeholder) != placeholder_identity
                || placeholder_uid != target.effective_uid
                || placeholder_mode != CONTROL_LEASE_MODE
            {
                let _ = exyonq_linux_ffi::unlink_file_at(dir.as_fd(), c"q");
                let _ = exyonq_linux_ffi::unlink_directory_at(target.parent.as_fd(), &dir_name);
                return Err(anyhow::anyhow!(
                    "unsafe cleanup placeholder in {}: uid={placeholder_uid}, euid={}, mode={placeholder_mode:#o}",
                    target.parent_path.display(),
                    target.effective_uid
                ));
            }

            return Ok(Self {
                parent: cleanup_parent,
                final_name: target.final_name.clone(),
                final_path: target.final_path.clone(),
                dir_name,
                dir,
                dir_path: target.parent_path.join(name),
                dir_identity: file_identity(&dir_metadata),
                _placeholder: placeholder,
                placeholder_identity,
                placeholder_location: PlaceholderLocation::StageQuarantine,
            });
        }
        anyhow::bail!("could not allocate a unique cleanup staging directory")
    }

    fn restore_candidate_by_exchange(&mut self) -> anyhow::Result<()> {
        exyonq_linux_ffi::rename_exchange_at(
            self.dir.as_fd(),
            c"q",
            self.parent.as_fd(),
            &self.final_name,
        )
        .map_err(|err| {
            anyhow::anyhow!(
                "restore quarantined control socket entry to {}: {err}",
                self.final_path.display()
            )
        })?;
        self.placeholder_location = PlaceholderLocation::StageQuarantine;
        Ok(())
    }

    fn restore_stage_entry_to_final(&mut self, name: &std::ffi::CStr) -> anyhow::Result<()> {
        match exyonq_linux_ffi::rename_noreplace_at(
            self.dir.as_fd(),
            name,
            self.parent.as_fd(),
            &self.final_name,
        ) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                exyonq_linux_ffi::rename_exchange_at(
                    self.dir.as_fd(),
                    name,
                    self.parent.as_fd(),
                    &self.final_name,
                )
                .map_err(|exchange_err| {
                    anyhow::anyhow!(
                        "restore preserved entry to {} after collision: {exchange_err}",
                        self.final_path.display()
                    )
                })
            }
            Err(err) => Err(anyhow::anyhow!(
                "restore preserved entry to {}: {err}",
                self.final_path.display()
            )),
        }
    }

    fn remove_expected_socket(
        &mut self,
        target: &ControlSocketTarget,
        expected: FileIdentity,
    ) -> anyhow::Result<bool> {
        self.remove_expected_socket_with_hook(target, expected, |_| {})
    }

    fn remove_expected_socket_with_hook(
        &mut self,
        target: &ControlSocketTarget,
        expected: FileIdentity,
        mut hook: impl FnMut(CleanupPhase),
    ) -> anyhow::Result<bool> {
        hook(CleanupPhase::BeforeExchange);
        match exyonq_linux_ffi::rename_exchange_at(
            self.parent.as_fd(),
            &self.final_name,
            self.dir.as_fd(),
            c"q",
        ) {
            Ok(()) => self.placeholder_location = PlaceholderLocation::Final,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "atomically quarantine control socket {}: {err}",
                    self.final_path.display()
                ));
            }
        }
        hook(CleanupPhase::AfterExchange);

        let quarantined = match exyonq_linux_ffi::metadata_at_nofollow(self.dir.as_fd(), c"q") {
            Ok(metadata) => metadata,
            Err(err) => {
                let restore = self.restore_candidate_by_exchange();
                return Err(anyhow::anyhow!(
                    "inspect quarantined control socket {}: {err}; restore={restore:?}",
                    self.final_path.display()
                ));
            }
        };
        if !quarantined.is_socket()
            || entry_identity(quarantined) != expected
            || quarantined.uid != target.effective_uid
        {
            self.restore_candidate_by_exchange()?;
            return Ok(false);
        }
        hook(CleanupPhase::AfterCandidateVerified);

        if let Err(err) = exyonq_linux_ffi::rename_noreplace_at(
            self.parent.as_fd(),
            &self.final_name,
            self.dir.as_fd(),
            c"p",
        ) {
            let restore = self.restore_candidate_by_exchange();
            return Err(anyhow::anyhow!(
                "park cleanup placeholder for {}: {err}; restore={restore:?}",
                self.final_path.display()
            ));
        }
        self.placeholder_location = PlaceholderLocation::StageParking;
        hook(CleanupPhase::AfterPlaceholderParked);

        let parked = match exyonq_linux_ffi::metadata_at_nofollow(self.dir.as_fd(), c"p") {
            Ok(parked) => parked,
            Err(err) => {
                self.placeholder_location = PlaceholderLocation::Unknown;
                let restore = self.restore_stage_entry_to_final(c"p");
                return Err(anyhow::anyhow!(
                    "inspect parked cleanup placeholder for {}: {err}; restore={restore:?}",
                    self.final_path.display()
                ));
            }
        };
        if !parked.is_regular() || entry_identity(parked) != self.placeholder_identity {
            self.placeholder_location = PlaceholderLocation::Unknown;
            self.restore_stage_entry_to_final(c"p")?;
            exyonq_linux_ffi::unlink_file_at(self.dir.as_fd(), c"q").map_err(|err| {
                anyhow::anyhow!(
                    "delete verified socket after replacement won cleanup race for {}: {err}",
                    self.final_path.display()
                )
            })?;
            return Ok(true);
        }
        hook(CleanupPhase::AfterPlaceholderVerified);

        if let Err(err) = exyonq_linux_ffi::unlink_file_at(self.dir.as_fd(), c"q") {
            let restore = exyonq_linux_ffi::rename_noreplace_at(
                self.dir.as_fd(),
                c"q",
                self.parent.as_fd(),
                &self.final_name,
            );
            return Err(anyhow::anyhow!(
                "delete verified quarantined control socket {}: {err}; restore={restore:?}",
                self.final_path.display()
            ));
        }
        hook(CleanupPhase::AfterCandidateDeleted);
        exyonq_linux_ffi::unlink_file_at(self.dir.as_fd(), c"p").map_err(|err| {
            anyhow::anyhow!(
                "delete verified cleanup placeholder for {}: {err}",
                self.final_path.display()
            )
        })?;
        self.placeholder_location = PlaceholderLocation::Gone;
        Ok(true)
    }

    fn remove_attempt_owned_stage(&self) -> anyhow::Result<()> {
        let current = exyonq_linux_ffi::metadata_at_nofollow(self.parent.as_fd(), &self.dir_name)
            .map_err(|err| {
            anyhow::anyhow!("inspect cleanup stage {}: {err}", self.dir_path.display())
        })?;
        if !current.is_directory() || entry_identity(current) != self.dir_identity {
            anyhow::bail!(
                "cleanup stage identity changed before removal: {}",
                self.dir_path.display()
            );
        }
        exyonq_linux_ffi::unlink_directory_at(self.parent.as_fd(), &self.dir_name).map_err(|err| {
            anyhow::anyhow!("remove cleanup stage {}: {err}", self.dir_path.display())
        })
    }
}

impl Drop for CleanupStage {
    fn drop(&mut self) {
        let placeholder_name = match self.placeholder_location {
            PlaceholderLocation::StageQuarantine => Some(c"q"),
            PlaceholderLocation::StageParking => Some(c"p"),
            PlaceholderLocation::Final
            | PlaceholderLocation::Gone
            | PlaceholderLocation::Unknown => None,
        };
        if let Some(name) = placeholder_name {
            if let Ok(metadata) = exyonq_linux_ffi::metadata_at_nofollow(self.dir.as_fd(), name) {
                if metadata.is_regular() && entry_identity(metadata) == self.placeholder_identity {
                    let _ = exyonq_linux_ffi::unlink_file_at(self.dir.as_fd(), name);
                }
            }
        }
        let _ = self.remove_attempt_owned_stage();
    }
}

fn unlink_if_same_socket(
    target: &ControlSocketTarget,
    expected: FileIdentity,
) -> anyhow::Result<bool> {
    let mut stage = CleanupStage::create(target)?;
    stage.remove_expected_socket(target, expected)
}

struct PublishedSocket<'a> {
    target: &'a ControlSocketTarget,
    identity: FileIdentity,
    armed: bool,
}

impl<'a> PublishedSocket<'a> {
    fn new(target: &'a ControlSocketTarget, identity: FileIdentity) -> Self {
        Self {
            target,
            identity,
            armed: true,
        }
    }

    fn disarm(mut self) -> FileIdentity {
        self.armed = false;
        self.identity
    }
}

impl Drop for PublishedSocket<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = unlink_if_same_socket(self.target, self.identity);
        }
    }
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

/// Cap063 startup cleanup safety.
///
/// The canonical parent directory is held open and protected by a per-socket
/// ownership lease. Existing entries must be euid-owned sockets. A failed live
/// connect is followed by atomic exchange into a private directory; deletion
/// happens there only after the moved inode matches the pre-connect identity.
/// The final pathname is never check-then-unlinked.
fn prepare_control_socket_target(target: &ControlSocketTarget) -> anyhow::Result<()> {
    let metadata = match target.entry_metadata() {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    target.verify_entry_owner(metadata)?;

    if metadata.is_directory() {
        anyhow::bail!(
            "control socket path is a directory (refusing to mutate): {}",
            target.final_path.display()
        );
    }
    if metadata.is_regular() {
        anyhow::bail!(
            "control socket path is a regular file (refusing to unlink): {}",
            target.final_path.display()
        );
    }
    if metadata.is_symlink() {
        anyhow::bail!(
            "control socket path is a symlink (refusing to follow or unlink): {}",
            target.final_path.display()
        );
    }
    if !metadata.is_socket() {
        anyhow::bail!(
            "control socket path exists and is not a Unix socket: {}",
            target.final_path.display()
        );
    }
    if path_has_live_listener(&target.final_path) {
        anyhow::bail!(
            "control socket path is in use by a live listener: {}",
            target.final_path.display()
        );
    }

    let expected = entry_identity(metadata);
    if !unlink_if_same_socket(target, expected)? {
        anyhow::bail!(
            "control socket changed during stale cleanup; preserved current entry: {}",
            target.final_path.display()
        );
    }
    Ok(())
}

/// Validate and remove an euid-owned stale socket using atomic quarantine.
///
/// The parent must be either an euid-owned non-group/world-writable directory,
/// or an euid/root-owned sticky directory such as `/tmp`. On filesystems or
/// platforms without atomic exchange/no-replace rename support, cleanup fails
/// closed and leaves the socket in place.
#[cfg(test)]
fn prepare_control_socket_path(socket_path: &Path) -> anyhow::Result<()> {
    let target = ControlSocketTarget::open(socket_path)?;
    prepare_control_socket_target(&target)
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
            let payload = serde_json::to_string(&unknown_json(&base))? + "\n";
            // LET-197 / SUB-LET-CTRL-WRITE-FALSE-OK: handle_client's Result includes
            // control response delivery. Discarding write_response Err then returning
            // Ok(()) reported false success while the client never received the reply.
            write_response(&mut stream, &payload).await?;
            return Ok(());
        }
    };

    let parsed = parse_command_line(&command_line);
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

/// Synchronously bind in a private staging directory and atomically publish.
///
/// Cap063: the stage is mode 0700 from creation, the inaccessible socket is
/// chmod/verified at 0600, then descriptor-relative `linkat` publishes the same
/// inode without replacing a race winner. No process-global umask is mutated.
pub fn bind_control_listener(socket_path: &Path) -> anyhow::Result<BoundControlSocket> {
    let target = ControlSocketTarget::open(socket_path)?;
    prepare_control_socket_target(&target)?;
    target.verify_parent_identity()?;

    let mut stage = SocketStage::create(&target)?;
    let listener = UnixListener::bind(&stage.socket_path).map_err(|err| {
        anyhow::anyhow!(
            "failed to bind staged control socket {}: {err}",
            stage.socket_path.display()
        )
    })?;
    let bound_metadata = std::fs::symlink_metadata(&stage.socket_path).map_err(|err| {
        anyhow::anyhow!(
            "lstat newly bound control socket {}: {err}",
            stage.socket_path.display()
        )
    })?;
    if !is_socket(&bound_metadata) {
        anyhow::bail!(
            "newly bound control socket is not a socket: {}",
            stage.socket_path.display()
        );
    }
    stage.record_socket(&bound_metadata);
    let socket_metadata = set_and_verify_mode(
        &stage.socket_path,
        CONTROL_SOCKET_MODE,
        "staged control socket",
    )?;
    let published_identity = file_identity(&socket_metadata);
    stage.record_socket(&socket_metadata);

    target.verify_parent_identity()?;
    exyonq_linux_ffi::hard_link_at(
        target.parent.as_fd(),
        &stage.relative_socket,
        target.parent.as_fd(),
        &target.final_name,
    )
    .map_err(|err| {
        anyhow::anyhow!(
            "atomically publish control socket {} without replacement: {err}",
            target.final_path.display()
        )
    })?;

    let published = PublishedSocket::new(&target, published_identity);
    let final_metadata = target.entry_metadata().map_err(|err| {
        anyhow::anyhow!(
            "inspect published control socket {}: {err}",
            target.final_path.display()
        )
    })?;
    target.verify_entry_owner(final_metadata)?;
    if !final_metadata.is_socket()
        || entry_identity(final_metadata) != published_identity
        || final_metadata.permissions() & 0o777 != CONTROL_SOCKET_MODE
    {
        anyhow::bail!(
            "published control socket verification failed for {}",
            target.final_path.display()
        );
    }
    target.verify_parent_identity()?;
    stage.remove_source_and_dir()?;
    let published_inode = published.disarm();

    info!(
        path = %target.final_path.display(),
        mode = format!("0o{CONTROL_SOCKET_MODE:o}"),
        "control socket atomically published"
    );
    Ok(BoundControlSocket {
        listener: Some(listener),
        target,
        published_inode,
    })
}

/// Owns the listener and its published inode.
///
/// Drop closes the listener first, then atomically quarantines the pathname.
///
/// Only the recorded socket inode is deleted. A replacement is exchanged back
/// into the final pathname, and unsupported atomic primitives leave the entry
/// in place with a diagnostic instead of falling back to pathname unlink.
pub struct BoundControlSocket {
    listener: Option<UnixListener>,
    target: ControlSocketTarget,
    published_inode: FileIdentity,
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
        if let Err(err) = unlink_if_same_socket(&self.target, self.published_inode) {
            warn!(
                path = %self.target.final_path.display(),
                %err,
                "left control socket in place after safe cleanup failed"
            );
        }
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
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
    use std::process::Command;
    use std::sync::{mpsc, Barrier};
    use std::thread;

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
    fn prepare_refuses_symlink_without_mutating_it() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let path = dir.path().join("control.sock");
        std::fs::write(&target, b"preserve").unwrap();
        symlink(&target, &path).unwrap();

        let err = prepare_control_socket_path(&path).unwrap_err().to_string();
        assert!(err.contains("symlink"), "{err}");
        assert!(std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"preserve");
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
        for _ in 0..100 {
            if !path_has_live_listener(&path) {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!path_has_live_listener(&path));
        prepare_control_socket_path(&path).unwrap();
        assert!(!path.exists());
    }

    fn staging_entries(parent: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(CONTROL_STAGE_PREFIX))
            })
            .collect()
    }

    #[tokio::test]
    async fn published_listener_accepts_via_final_link_with_exact_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let bound = bind_control_listener(&path).unwrap();

        let metadata = std::fs::symlink_metadata(&path).unwrap();
        assert!(is_socket(&metadata));
        assert_eq!(file_identity(&metadata), bound.published_inode);
        assert_eq!(metadata.permissions().mode() & 0o777, CONTROL_SOCKET_MODE);
        assert!(staging_entries(dir.path()).is_empty());

        let client = UnixStream::connect(&path)
            .await
            .expect("connect final link");
        let (server, _) = tokio::time::timeout(Duration::from_secs(2), bound.listener().accept())
            .await
            .expect("accept timeout")
            .expect("accept final link");
        drop((client, server));
    }

    #[test]
    fn stage_raii_removes_attempt_owned_entries_before_and_after_bind() {
        let dir = tempfile::tempdir().unwrap();
        let target = ControlSocketTarget::open(&dir.path().join("control.sock")).unwrap();

        let stage = SocketStage::create(&target).unwrap();
        let stage_path = stage.dir_path.clone();
        assert_eq!(
            std::fs::symlink_metadata(&stage_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            CONTROL_STAGE_DIR_MODE
        );
        drop(stage);
        assert!(!stage_path.exists());

        let mut stage = SocketStage::create(&target).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let listener = UnixListener::bind(&stage.socket_path).unwrap();
        let metadata = std::fs::symlink_metadata(&stage.socket_path).unwrap();
        stage.record_socket(&metadata);
        let stage_path = stage.dir_path.clone();
        drop(listener);
        drop(stage);
        assert!(!stage_path.exists());
        assert!(staging_entries(dir.path()).is_empty());
    }

    #[test]
    fn stage_raii_cleans_bind_and_chmod_failure_states() {
        let dir = tempfile::tempdir().unwrap();
        let target = ControlSocketTarget::open(&dir.path().join("control.sock")).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        let _guard = runtime.enter();

        let mut bind_failure_stage = SocketStage::create(&target).unwrap();
        let bind_failure_path = bind_failure_stage.dir_path.clone();
        bind_failure_stage.socket_path = bind_failure_stage.dir_path.clone();
        assert!(UnixListener::bind(&bind_failure_stage.socket_path).is_err());
        drop(bind_failure_stage);
        assert!(!bind_failure_path.exists());

        let mut chmod_failure_stage = SocketStage::create(&target).unwrap();
        let listener = UnixListener::bind(&chmod_failure_stage.socket_path).unwrap();
        let metadata = std::fs::symlink_metadata(&chmod_failure_stage.socket_path).unwrap();
        chmod_failure_stage.record_socket(&metadata);
        std::fs::remove_file(&chmod_failure_stage.socket_path).unwrap();
        assert!(set_and_verify_mode(
            &chmod_failure_stage.socket_path,
            CONTROL_SOCKET_MODE,
            "staged control socket",
        )
        .is_err());
        let chmod_failure_path = chmod_failure_stage.dir_path.clone();
        drop(listener);
        drop(chmod_failure_stage);
        assert!(!chmod_failure_path.exists());
        assert!(staging_entries(dir.path()).is_empty());
    }

    #[test]
    fn atomic_publication_collision_preserves_single_winner_and_cleans_stages() {
        const CONTENDERS: usize = 8;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let start = Arc::new(Barrier::new(CONTENDERS));
        let release = Arc::new(Barrier::new(CONTENDERS + 1));
        let (tx, rx) = mpsc::channel();
        let mut threads = Vec::with_capacity(CONTENDERS);

        for _ in 0..CONTENDERS {
            let path = path.clone();
            let start = Arc::clone(&start);
            let release = Arc::clone(&release);
            let tx = tx.clone();
            threads.push(thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .build()
                    .unwrap();
                let _guard = runtime.enter();
                start.wait();
                let bound = bind_control_listener(&path);
                tx.send(
                    bound
                        .as_ref()
                        .map(|bound| bound.published_inode)
                        .map_err(|err| err.to_string()),
                )
                .unwrap();
                release.wait();
                drop(bound);
            }));
        }
        drop(tx);

        let outcomes: Vec<_> = (0..CONTENDERS)
            .map(|_| rx.recv().expect("contender result"))
            .collect();
        let winners: Vec<_> = outcomes
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .collect();
        assert_eq!(winners.len(), 1, "outcomes={outcomes:?}");
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        assert_eq!(file_identity(&metadata), **winners.first().unwrap());
        let _client = StdUnixStream::connect(&path).expect("winner remains reachable");
        assert!(staging_entries(dir.path()).is_empty());

        release.wait();
        for handle in threads {
            handle.join().unwrap();
        }
        assert!(!path.exists());
        assert!(staging_entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn drop_removes_own_unchanged_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let bound = bind_control_listener(&path).unwrap();
        drop(bound);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn drop_preserves_regular_symlink_and_socket_replacements() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();

        let regular_path = dir.path().join("regular.sock");
        let regular_bound = bind_control_listener(&regular_path).unwrap();
        std::fs::remove_file(&regular_path).unwrap();
        std::fs::write(&regular_path, b"replacement").unwrap();
        drop(regular_bound);
        assert_eq!(std::fs::read(&regular_path).unwrap(), b"replacement");

        let symlink_path = dir.path().join("symlink.sock");
        let symlink_bound = bind_control_listener(&symlink_path).unwrap();
        std::fs::remove_file(&symlink_path).unwrap();
        symlink("missing-target", &symlink_path).unwrap();
        drop(symlink_bound);
        assert!(std::fs::symlink_metadata(&symlink_path)
            .unwrap()
            .file_type()
            .is_symlink());

        let socket_path = dir.path().join("socket.sock");
        let socket_bound = bind_control_listener(&socket_path).unwrap();
        std::fs::remove_file(&socket_path).unwrap();
        let replacement = StdUnixListener::bind(&socket_path).unwrap();
        let replacement_identity = file_identity(&std::fs::symlink_metadata(&socket_path).unwrap());
        drop(socket_bound);
        assert_eq!(
            file_identity(&std::fs::symlink_metadata(&socket_path).unwrap()),
            replacement_identity
        );
        drop(replacement);
    }

    #[tokio::test]
    async fn ownership_lease_serializes_same_uid_instances() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let bound = bind_control_listener(&path).unwrap();
        let err = match bind_control_listener(&path) {
            Ok(_) => panic!("second owner unexpectedly acquired the socket lease"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("ownership lease is held"), "{err}");
        drop(bound);
    }

    #[test]
    fn unsafe_writable_parent_is_rejected_but_sticky_parent_is_accepted() {
        let unsafe_parent = tempfile::tempdir().unwrap();
        std::fs::set_permissions(unsafe_parent.path(), std::fs::Permissions::from_mode(0o777))
            .unwrap();
        let unsafe_path = unsafe_parent.path().join("control.sock");
        let err = prepare_control_socket_path(&unsafe_path)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsafe control socket parent"), "{err}");

        let sticky_parent = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            sticky_parent.path(),
            std::fs::Permissions::from_mode(0o1777),
        )
        .unwrap();
        let sticky_path = sticky_parent.path().join("control.sock");
        prepare_control_socket_path(&sticky_path).unwrap();
    }

    #[test]
    fn sticky_tmp_parent_is_accepted() {
        let unique = format!(
            "exyonq-control-parent-{}-{}.sock",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        );
        let path = Path::new("/tmp").join(&unique);
        let canonical_parent = std::fs::canonicalize("/tmp").unwrap();
        let lease = canonical_parent.join(format!(".exyonq-{unique}.lock"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&lease);
        prepare_control_socket_path(&path).unwrap();
        assert!(!path.exists());
        std::fs::remove_file(lease).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn foreign_parent_and_final_inode_ownership_are_rejected() {
        if exyonq_linux_ffi::effective_uid() != 0 {
            return;
        }

        let foreign_parent = tempfile::tempdir().unwrap();
        std::os::unix::fs::chown(foreign_parent.path(), Some(12345), None).unwrap();
        let parent_err = prepare_control_socket_path(&foreign_parent.path().join("control.sock"))
            .unwrap_err()
            .to_string();
        assert!(
            parent_err.contains("unsafe control socket parent"),
            "{parent_err}"
        );

        let sticky_parent = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            sticky_parent.path(),
            std::fs::Permissions::from_mode(0o1777),
        )
        .unwrap();
        let foreign_socket = sticky_parent.path().join("foreign.sock");
        let listener = StdUnixListener::bind(&foreign_socket).unwrap();
        drop(listener);
        std::os::unix::fs::chown(&foreign_socket, Some(12345), None).unwrap();
        let inode_err = prepare_control_socket_path(&foreign_socket)
            .unwrap_err()
            .to_string();
        assert!(inode_err.contains("owned by uid 12345"), "{inode_err}");
        assert!(is_socket(
            &std::fs::symlink_metadata(&foreign_socket).unwrap()
        ));
    }

    #[tokio::test]
    async fn drop_replacement_race_preserves_winner_for_1000_iterations() {
        use std::fs::OpenOptions;

        let dir = tempfile::tempdir().unwrap();
        for iteration in 0..1000_u32 {
            let path = dir.path().join(format!("control-{iteration}.sock"));
            let bound = bind_control_listener(&path).unwrap();
            let start = Arc::new(Barrier::new(2));
            let race_start = Arc::clone(&start);
            let race_path = path.clone();
            let marker = iteration.to_ne_bytes();
            let racer = thread::spawn(move || {
                race_start.wait();
                loop {
                    match std::fs::remove_file(&race_path) {
                        Ok(()) => {}
                        Err(err) if err.kind() == ErrorKind::NotFound => {}
                        Err(err) => panic!("remove race target: {err}"),
                    }
                    match OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&race_path)
                    {
                        Ok(mut file) => {
                            use std::io::Write as _;
                            file.write_all(&marker).unwrap();
                            break;
                        }
                        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                            thread::yield_now();
                        }
                        Err(err) => panic!("create replacement: {err}"),
                    }
                }
            });
            start.wait();
            drop(bound);
            racer.join().unwrap();
            assert_eq!(
                std::fs::read(&path).unwrap(),
                marker,
                "replacement lost at iteration {iteration}"
            );
            std::fs::remove_file(&path).unwrap();
            assert!(
                staging_entries(dir.path()).is_empty(),
                "owned staging artifact leaked at iteration {iteration}"
            );
        }
    }

    #[test]
    fn cleanup_replacement_at_every_phase_is_preserved() {
        let phases = [
            CleanupPhase::BeforeExchange,
            CleanupPhase::AfterExchange,
            CleanupPhase::AfterCandidateVerified,
            CleanupPhase::AfterPlaceholderParked,
            CleanupPhase::AfterPlaceholderVerified,
            CleanupPhase::AfterCandidateDeleted,
        ];

        let dir = tempfile::tempdir().unwrap();
        for (index, injected_phase) in phases.into_iter().enumerate() {
            let path = dir.path().join(format!("phase-{index}.sock"));
            let listener = StdUnixListener::bind(&path).unwrap();
            let expected = file_identity(&std::fs::symlink_metadata(&path).unwrap());
            drop(listener);

            let target = ControlSocketTarget::open(&path).unwrap();
            let mut stage = CleanupStage::create(&target).unwrap();
            let marker = format!("replacement-{injected_phase:?}").into_bytes();
            let result = stage
                .remove_expected_socket_with_hook(&target, expected, |phase| {
                    if phase == injected_phase {
                        match std::fs::remove_file(&path) {
                            Ok(()) => {}
                            Err(err) if err.kind() == ErrorKind::NotFound => {}
                            Err(err) => panic!("remove at {phase:?}: {err}"),
                        }
                        std::fs::write(&path, &marker).unwrap();
                    }
                })
                .unwrap();
            assert_eq!(result, injected_phase != CleanupPhase::BeforeExchange);
            let preserved = std::fs::read(&path).unwrap_or_else(|err| {
                panic!("replacement missing after {injected_phase:?}: {err}")
            });
            assert_eq!(preserved, marker, "replacement lost at {injected_phase:?}");
            drop(stage);
            std::fs::remove_file(&path).unwrap();
        }
        assert!(staging_entries(dir.path()).is_empty());
    }

    #[test]
    fn process_umask_is_unchanged_during_parallel_binds() {
        let executable = std::env::current_exe().unwrap();
        let output = Command::new("sh")
            .args([
                "-c",
                "umask 027; exec \"$1\" umask_parallel_child --ignored --nocapture",
                "sh",
            ])
            .arg(executable)
            .env("EXYONQ_UMASK_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "launched in an isolated child with a known umask"]
    fn umask_parallel_child() {
        if std::env::var_os("EXYONQ_UMASK_CHILD").is_none() {
            return;
        }

        const CREATORS: usize = 4;
        let dir = tempfile::tempdir().unwrap();
        let start = Arc::new(Barrier::new(CREATORS + 1));
        let mut threads = Vec::with_capacity(CREATORS + 1);

        for creator in 0..CREATORS {
            let root = dir.path().to_path_buf();
            let start = Arc::clone(&start);
            threads.push(thread::spawn(move || {
                start.wait();
                for iteration in 0..500 {
                    let file_path = root.join(format!("f-{creator}-{iteration}"));
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o666)
                        .open(&file_path)
                        .unwrap();
                    let file_mode = file.metadata().unwrap().permissions().mode() & 0o777;
                    assert_eq!(
                        file_mode,
                        0o640,
                        "umask leaked into {}",
                        file_path.display()
                    );
                    drop(file);
                    std::fs::remove_file(&file_path).unwrap();

                    let dir_path = root.join(format!("d-{creator}-{iteration}"));
                    let mut builder = DirBuilder::new();
                    builder.mode(0o777);
                    builder.create(&dir_path).unwrap();
                    let dir_mode =
                        std::fs::metadata(&dir_path).unwrap().permissions().mode() & 0o777;
                    assert_eq!(dir_mode, 0o750, "umask leaked into {}", dir_path.display());
                    std::fs::remove_dir(&dir_path).unwrap();
                }
            }));
        }

        let root = dir.path().to_path_buf();
        let start = Arc::clone(&start);
        threads.push(thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .build()
                .unwrap();
            let _guard = runtime.enter();
            start.wait();
            for iteration in 0..500 {
                let path = root.join(format!("control-{iteration}.sock"));
                let bound = bind_control_listener(&path).unwrap();
                drop(bound);
            }
        }));

        for handle in threads {
            handle.join().unwrap();
        }

        let sentinel = dir.path().join("after");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o666)
            .open(&sentinel)
            .unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o640);
        assert!(staging_entries(dir.path()).is_empty());
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
