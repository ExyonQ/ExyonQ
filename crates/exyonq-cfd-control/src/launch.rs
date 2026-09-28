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

//! Spawn / wait / stop the `exyonq-dataplane` child process.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use exyonq_cfd_gen::{GenDir, Generation, RouteTable, SCHEMA_VERSION};
use thiserror::Error;

use crate::budget::resolve_competitive_h1_shards;
use crate::project::load_routes_from_env;

/// Feature gate: `EXYONQ_COMPETITIVE_H1_DATAPLANE=1` enables foundation path.
pub const ENV_ENABLED: &str = "EXYONQ_COMPETITIVE_H1_DATAPLANE";
pub const ENV_LISTEN: &str = "EXYONQ_CFD_LISTEN";
pub const ENV_GEN_DIR: &str = "EXYONQ_CFD_GEN_DIR";
pub const ENV_SHARDS: &str = "EXYONQ_CFD_SHARDS";
pub const ENV_BIN: &str = "EXYONQ_DATAPLANE_BIN";
pub const ENV_SCHEMA: &str = "EXYONQ_CFD_SCHEMA_VERSION";

pub const READY_TIMEOUT_DEFAULT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum CfdLaunchError {
    #[error("competitive H1 dataplane not enabled ({ENV_ENABLED}!=1)")]
    NotEnabled,
    #[error("missing {ENV_LISTEN}")]
    MissingListen,
    #[error("missing {ENV_GEN_DIR}")]
    MissingGenDir,
    #[error("dataplane binary not found")]
    BinaryNotFound,
    #[error("schema mismatch: control={control} dataplane_env={other:?}")]
    SchemaMismatch { control: u32, other: Option<String> },
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("generation: {0}")]
    Gen(#[from] exyonq_cfd_gen::GenError),
    #[error("dataplane did not become READY within {0:?}")]
    ReadyTimeout(Duration),
    #[error("dataplane exited before READY (status={0:?})")]
    ExitedBeforeReady(Option<i32>),
    #[error("invalid status line: {0}")]
    InvalidStatus(String),
    #[error("CFD listen collides with product listen (dual-accept forbidden): {0}")]
    ListenCollision(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataplaneStatus {
    Starting,
    Ready { generation_id: u64, listen: String },
    Stopping,
    Stopped,
    Failed { message: String },
}

impl DataplaneStatus {
    pub fn parse_line(line: &str) -> Result<Self, CfdLaunchError> {
        let line = line.trim();
        if line == "STARTING" {
            return Ok(Self::Starting);
        }
        if line == "STOPPING" {
            return Ok(Self::Stopping);
        }
        if line == "STOPPED" {
            return Ok(Self::Stopped);
        }
        if let Some(rest) = line.strip_prefix("READY ") {
            let mut gen = None;
            let mut listen = None;
            for part in rest.split_whitespace() {
                if let Some(v) = part.strip_prefix("gen=") {
                    gen = v.parse().ok();
                } else if let Some(v) = part.strip_prefix("listen=") {
                    listen = Some(v.to_string());
                }
            }
            match (gen, listen) {
                (Some(generation_id), Some(listen)) => Ok(Self::Ready {
                    generation_id,
                    listen,
                }),
                _ => Err(CfdLaunchError::InvalidStatus(line.to_string())),
            }
        } else if let Some(msg) = line.strip_prefix("FAILED ") {
            Ok(Self::Failed {
                message: msg.to_string(),
            })
        } else {
            Err(CfdLaunchError::InvalidStatus(line.to_string()))
        }
    }
}

#[derive(Debug, Clone)]
pub struct CfdLaunchConfig {
    pub listen: String,
    pub gen_dir: PathBuf,
    pub shards: usize,
    pub dataplane_bin: PathBuf,
    pub ready_timeout: Duration,
}

impl CfdLaunchConfig {
    pub fn from_env() -> Result<Self, CfdLaunchError> {
        if !dataplane_enabled() {
            return Err(CfdLaunchError::NotEnabled);
        }
        let listen = std::env::var(ENV_LISTEN).map_err(|_| CfdLaunchError::MissingListen)?;
        if listen.trim().is_empty() {
            return Err(CfdLaunchError::MissingListen);
        }
        let gen_dir = std::env::var(ENV_GEN_DIR).map_err(|_| CfdLaunchError::MissingGenDir)?;
        if gen_dir.trim().is_empty() {
            return Err(CfdLaunchError::MissingGenDir);
        }
        let shards = std::env::var(ENV_SHARDS).ok().and_then(|s| s.parse().ok());
        let shards = resolve_competitive_h1_shards(shards);
        let dataplane_bin = resolve_dataplane_bin()?;
        Ok(Self {
            listen,
            gen_dir: PathBuf::from(gen_dir),
            shards,
            dataplane_bin,
            ready_timeout: READY_TIMEOUT_DEFAULT,
        })
    }
}

pub fn dataplane_enabled() -> bool {
    matches!(
        std::env::var(ENV_ENABLED).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

pub fn resolve_dataplane_bin() -> Result<PathBuf, CfdLaunchError> {
    if let Ok(p) = std::env::var(ENV_BIN) {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(pb);
        }
        return Err(CfdLaunchError::BinaryNotFound);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("exyonq-dataplane");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("exyonq-dataplane");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(CfdLaunchError::BinaryNotFound)
}

/// Reject when competitive CFD listen shares a port with any product listen string.
pub fn assert_no_listen_collision(
    cfd_listen: &str,
    product_listens: &[String],
) -> Result<(), CfdLaunchError> {
    let cfd_port = parse_port(cfd_listen).ok_or_else(|| {
        CfdLaunchError::ListenCollision(format!("unparseable CFD listen {cfd_listen}"))
    })?;
    for p in product_listens {
        let Some(pp) = parse_port(p) else {
            return Err(CfdLaunchError::ListenCollision(format!(
                "unparseable product listen {p} (fail-closed when CFD enabled)"
            )));
        };
        if pp == cfd_port {
            return Err(CfdLaunchError::ListenCollision(format!(
                "port {cfd_port} also in product listen {p}"
            )));
        }
    }
    Ok(())
}

fn parse_port(listen: &str) -> Option<u16> {
    listen
        .parse::<SocketAddr>()
        .ok()
        .map(|a| a.port())
        .or_else(|| {
            listen
                .rsplit_once(':')
                .and_then(|(_, port)| port.parse().ok())
        })
}

/// Running dataplane child + gen directory.
pub struct CfdChild {
    pub child: Child,
    pub gen_dir: GenDir,
    pub listen: String,
    pub shards: usize,
    shutdown_requested: bool,
}

impl CfdChild {
    pub fn start(cfg: &CfdLaunchConfig) -> Result<Self, CfdLaunchError> {
        if let Ok(v) = std::env::var(ENV_SCHEMA) {
            if v.parse::<u32>().ok() != Some(SCHEMA_VERSION) {
                return Err(CfdLaunchError::SchemaMismatch {
                    control: SCHEMA_VERSION,
                    other: Some(v),
                });
            }
        }
        if !cfg.dataplane_bin.is_file() {
            return Err(CfdLaunchError::BinaryNotFound);
        }
        let gen_dir = GenDir::new(&cfg.gen_dir);
        gen_dir.ensure()?;
        let _ = fs::remove_file(gen_dir.status_path());
        let _ = fs::remove_file(gen_dir.root().join("shutdown"));
        let table = match load_routes_from_env() {
            Ok(Some(t)) => t,
            Ok(None) => RouteTable::default(),
            Err(e) => {
                return Err(CfdLaunchError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    e.to_string(),
                )));
            }
        };
        let g1 = Generation::from_route_table(1, &table)?;
        gen_dir.publish(&g1)?;

        let mut cmd = Command::new(&cfg.dataplane_bin);
        cmd.arg("serve")
            .arg("--listen")
            .arg(&cfg.listen)
            .arg("--gen-dir")
            .arg(gen_dir.root())
            .arg("--shards")
            .arg(cfg.shards.to_string())
            .arg("--schema-version")
            .arg(SCHEMA_VERSION.to_string())
            .env(ENV_GEN_DIR, gen_dir.root())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let child = cmd.spawn()?;
        let mut this = Self {
            child,
            gen_dir,
            listen: cfg.listen.clone(),
            shards: cfg.shards,
            shutdown_requested: false,
        };
        this.wait_ready(cfg.ready_timeout)?;
        Ok(this)
    }

    pub fn wait_ready(&mut self, timeout: Duration) -> Result<DataplaneStatus, CfdLaunchError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Err(CfdLaunchError::ExitedBeforeReady(status.code()));
            }
            if let Ok(raw) = fs::read_to_string(self.gen_dir.status_path()) {
                if let Ok(st) = DataplaneStatus::parse_line(raw.lines().next().unwrap_or("")) {
                    match &st {
                        DataplaneStatus::Ready { .. } => return Ok(st),
                        DataplaneStatus::Failed { message } => {
                            return Err(CfdLaunchError::InvalidStatus(message.clone()));
                        }
                        _ => {}
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(CfdLaunchError::ReadyTimeout(timeout));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn publish_generation(&self, gen: &Generation) -> Result<(), CfdLaunchError> {
        self.gen_dir.publish(gen)?;
        Ok(())
    }

    /// Publish a new generation with compiled route projection (Phase 2).
    pub fn publish_routes(
        &self,
        generation_id: u64,
        table: &RouteTable,
    ) -> Result<(), CfdLaunchError> {
        let gen = Generation::from_route_table(generation_id, table)?;
        self.publish_generation(&gen)
    }

    /// Publish routes + optional WAF projection (Phase 3).
    pub fn publish_composite(
        &self,
        generation_id: u64,
        composite: &exyonq_cfd_gen::CompositeProjection,
    ) -> Result<(), CfdLaunchError> {
        let gen = Generation::from_composite(generation_id, composite)?;
        self.publish_generation(&gen)
    }

    pub fn try_wait_exited(&mut self) -> Result<Option<i32>, CfdLaunchError> {
        Ok(self.child.try_wait()?.map(|s| s.code().unwrap_or(-1)))
    }

    pub fn shutdown(mut self) -> Result<(), CfdLaunchError> {
        self.shutdown_requested = true;
        let shutdown_path = self.gen_dir.root().join("shutdown");
        let _ = fs::write(&shutdown_path, b"1\n");
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(_status) = self.child.try_wait()? {
                let _ = fs::remove_file(&shutdown_path);
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&shutdown_path);
        Ok(())
    }
}

impl Drop for CfdChild {
    fn drop(&mut self) {
        if self.shutdown_requested {
            return;
        }
        let shutdown_path = self.gen_dir.root().join("shutdown");
        let _ = fs::write(&shutdown_path, b"1\n");
        for _ in 0..50 {
            if let Ok(Some(_)) = self.child.try_wait() {
                let _ = fs::remove_file(&shutdown_path);
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&shutdown_path);
    }
}

pub fn wait_status(gen_dir: &Path) -> Result<DataplaneStatus, CfdLaunchError> {
    let raw = fs::read_to_string(GenDir::new(gen_dir).status_path())?;
    DataplaneStatus::parse_line(raw.lines().next().unwrap_or(""))
}

#[cfg(test)]
mod collision_tests {
    use super::*;

    #[test]
    fn collision_same_port() {
        let product = vec!["127.0.0.1:8080".to_string()];
        assert!(assert_no_listen_collision("127.0.0.1:8080", &product).is_err());
    }

    #[test]
    fn no_collision_different_port() {
        let product = vec!["127.0.0.1:8080".to_string()];
        assert!(assert_no_listen_collision("127.0.0.1:18080", &product).is_ok());
    }
}
