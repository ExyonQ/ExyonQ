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

use crate::unix_connect::connect_unix_stream;
use crate::wire::WireError;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
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
pub enum FcgiStream {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl FcgiStream {
    pub fn connect(endpoint: &PoolEndpoint, connect_timeout: Duration) -> Result<Self, WireError> {
        match endpoint {
            PoolEndpoint::Unix(path) => {
                let stream = connect_unix_stream(Path::new(path), connect_timeout)?;
                Ok(Self::Unix(stream))
            }
            PoolEndpoint::Tcp(addr) => {
                let stream = TcpStream::connect_timeout(addr, connect_timeout).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock
                    {
                        WireError::Timeout
                    } else {
                        WireError::ConnectionFailed
                    }
                })?;
                Ok(Self::Tcp(stream))
            }
        }
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), WireError> {
        match self {
            Self::Unix(s) => s.set_read_timeout(timeout).map_err(|_| WireError::IoFailed),
            Self::Tcp(s) => s.set_read_timeout(timeout).map_err(|_| WireError::IoFailed),
        }
    }

    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<(), WireError> {
        match self {
            Self::Unix(s) => s.set_write_timeout(timeout).map_err(|_| WireError::IoFailed),
            Self::Tcp(s) => s.set_write_timeout(timeout).map_err(|_| WireError::IoFailed),
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
