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
//! Unified FastCGI stream for UDS and TCP pooling (P1.2).

use crate::unix_connect::connect_unix_stream_detailed;
use crate::wire::WireError;
use std::fmt;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// Pre-resolved pool endpoint (no per-request DNS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolEndpoint {
    Unix(PathBuf),
    /// Literal `SocketAddr` resolved at config compile / pool construction.
    Tcp(SocketAddr),
}

impl PoolEndpoint {
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Self::Unix(path.into())
    }

    pub fn tcp_addr(addr: SocketAddr) -> Self {
        Self::Tcp(addr)
    }
}

/// Bidirectional FastCGI peer stream (one request at a time).
#[derive(Debug)]
pub enum FcgiStream {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl FcgiStream {
    /// Connect preserving the OS error for diagnostics (`errno`, phase).
    pub fn connect_detailed(
        endpoint: &PoolEndpoint,
        connect_timeout: Duration,
    ) -> Result<Self, ConnectFailure> {
        match endpoint {
            PoolEndpoint::Unix(path) => connect_unix_stream_detailed(path, connect_timeout)
                .map(Self::Unix)
                .map_err(|error| ConnectFailure {
                    endpoint: format!("unix:{}", path.display()),
                    error,
                }),
            PoolEndpoint::Tcp(addr) => TcpStream::connect_timeout(addr, connect_timeout)
                .map(Self::Tcp)
                .map_err(|error| ConnectFailure {
                    endpoint: format!("tcp:{addr}"),
                    error,
                }),
        }
    }

    pub fn connect(endpoint: &PoolEndpoint, connect_timeout: Duration) -> Result<Self, WireError> {
        Self::connect_detailed(endpoint, connect_timeout).map_err(ConnectFailure::into_wire_error)
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), WireError> {
        match self {
            Self::Unix(s) => s.set_read_timeout(timeout).map_err(|_| WireError::IoFailed),
            Self::Tcp(s) => s.set_read_timeout(timeout).map_err(|_| WireError::IoFailed),
        }
    }

    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<(), WireError> {
        match self {
            Self::Unix(s) => s
                .set_write_timeout(timeout)
                .map_err(|_| WireError::IoFailed),
            Self::Tcp(s) => s
                .set_write_timeout(timeout)
                .map_err(|_| WireError::IoFailed),
        }
    }

    pub fn apply_rw_timeouts(&self, timeout: Duration) -> Result<(), WireError> {
        self.set_read_timeout(Some(timeout))?;
        self.set_write_timeout(Some(timeout))?;
        Ok(())
    }

    /// Non-destructive liveness probe for idle pooled sockets (EOF ⇒ stale).
    ///
    /// Must run **before** any FastCGI request bytes so safe-retry stays at
    /// [`CommitStage::NotStarted`](crate::commit::CommitStage::NotStarted).
    pub fn probe_idle_alive(&mut self) -> Result<(), WireError> {
        let prev = match self {
            Self::Unix(s) => s.read_timeout().ok().flatten(),
            Self::Tcp(s) => s.read_timeout().ok().flatten(),
        };
        let _ = self.set_read_timeout(Some(Duration::from_millis(1)));
        let mut buf = [0u8; 1];
        let result = match self.read(&mut buf) {
            Ok(0) => Err(WireError::ConnectionClosed),
            Ok(_) => Err(WireError::ConnectionClosed), // unexpected peer data
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(())
            }
            Err(_) => Err(WireError::IoFailed),
        };
        let _ = self.set_read_timeout(prev);
        result
    }
}

/// Connect failure with the real OS error kept for operator logs.
#[derive(Debug)]
pub struct ConnectFailure {
    pub endpoint: String,
    pub error: std::io::Error,
}

impl ConnectFailure {
    /// Symbolic `errno` for the log (`EACCES`, `ENOENT`, `ECONNREFUSED`, …).
    pub fn errno_label(&self) -> &'static str {
        match self.error.raw_os_error() {
            Some(code) if code == libc::EACCES => "EACCES",
            Some(code) if code == libc::EPERM => "EPERM",
            Some(code) if code == libc::ENOENT => "ENOENT",
            Some(code) if code == libc::ECONNREFUSED => "ECONNREFUSED",
            Some(code) if code == libc::ETIMEDOUT => "ETIMEDOUT",
            Some(code) if code == libc::ENOTSOCK => "ENOTSOCK",
            Some(code) if code == libc::EADDRNOTAVAIL => "EADDRNOTAVAIL",
            Some(code) if code == libc::ECONNRESET => "ECONNRESET",
            _ => match self.error.kind() {
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => "ETIMEDOUT",
                std::io::ErrorKind::NotFound => "ENOENT",
                std::io::ErrorKind::PermissionDenied => "EACCES",
                std::io::ErrorKind::ConnectionRefused => "ECONNREFUSED",
                _ => "EIO",
            },
        }
    }

    /// Operator hint for the common PHP-FPM failures in issue #109.
    pub fn hint(&self) -> &'static str {
        match self.errno_label() {
            "EACCES" | "EPERM" => {
                "socket permissions deny the ExyonQ user; set php-fpm listen.owner/listen.group so this user can connect, then restart PHP-FPM (a USR2 reload keeps the old socket inode)"
            }
            "ENOENT" => {
                "socket path does not exist; check [[fcgi_pool]].address and that PHP-FPM is running with that listen path"
            }
            "ECONNREFUSED" => {
                "nothing is accepting on the socket; start PHP-FPM or fix the listener backlog/transport"
            }
            "ETIMEDOUT" => {
                "connect timed out; check PHP-FPM backlog, pm.max_children and system socket limits"
            }
            _ => "check the FastCGI endpoint path, transport (unix/tcp) and that PHP-FPM is reachable",
        }
    }

    pub fn into_wire_error(self) -> WireError {
        match self.error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => WireError::Timeout,
            _ => WireError::ConnectionFailed,
        }
    }
}

impl fmt::Display for ConnectFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (errno={}, os_error={:?}): {}",
            self.endpoint,
            self.errno_label(),
            self.error.raw_os_error(),
            self.error
        )
    }
}

impl std::error::Error for ConnectFailure {}

impl Read for FcgiStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.read(buf),
            Self::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for FcgiStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.write(buf),
            Self::Tcp(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.flush(),
            Self::Tcp(s) => s.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_failure_keeps_missing_socket_errno() {
        let path = std::env::temp_dir().join(format!(
            "exyonq-missing-fcgi-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let failure =
            FcgiStream::connect_detailed(&PoolEndpoint::Unix(path), Duration::from_millis(200))
                .expect_err("missing socket must fail");
        assert_eq!(failure.errno_label(), "ENOENT");
        assert!(
            failure.hint().contains("does not exist"),
            "{}",
            failure.hint()
        );
        assert!(failure.to_string().contains("errno=ENOENT"));
    }
}
