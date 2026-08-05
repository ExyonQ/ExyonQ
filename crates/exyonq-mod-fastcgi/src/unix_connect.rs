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
//! Non-blocking Unix domain connect with explicit timeout (PR5-B1-real-final).

use crate::wire::WireError;
use socket2::{Domain, SockAddr, Socket, Type};
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// Connect to a Unix stream socket with an explicit connect timeout.
///
/// Uses non-blocking `connect` + `connect_timeout` poll + `SO_ERROR` check, then restores
/// blocking mode for the existing spawn_blocking transport path.
pub fn connect_unix_stream(path: &Path, timeout: Duration) -> Result<UnixStream, WireError> {
    let addr = SockAddr::unix(path).map_err(|_| WireError::ConnectionFailed)?;
    let socket =
        Socket::new(Domain::UNIX, Type::STREAM, None).map_err(|_| WireError::ConnectionFailed)?;

    match socket.connect_timeout(&addr, timeout) {
        Ok(()) => {}
        Err(err) => return Err(map_connect_error(&err)),
    }

    socket
        .set_nonblocking(false)
        .map_err(|_| WireError::ConnectionFailed)?;

    let owned: OwnedFd = socket.into();
    Ok(UnixStream::from(owned))
}

fn map_connect_error(err: &std::io::Error) -> WireError {
    match err.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => WireError::Timeout,
        ErrorKind::NotFound
        | ErrorKind::ConnectionRefused
        | ErrorKind::PermissionDenied
        | ErrorKind::AddrNotAvailable => WireError::ConnectionFailed,
        _ => match err.raw_os_error() {
            Some(libc::ENOENT)
            | Some(libc::ECONNREFUSED)
            | Some(libc::EACCES)
            | Some(libc::EPERM)
            | Some(libc::ENOTSOCK)
            | Some(libc::ECONNRESET) => WireError::ConnectionFailed,
            Some(libc::ETIMEDOUT) => WireError::Timeout,
            _ => WireError::ConnectionFailed,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;

    static BIND_SEQ: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn missing_socket_maps_to_connection_failed() {
        let path = std::env::temp_dir().join(format!(
            "exyonq-no-sock-{}-{}.sock",
            std::process::id(),
            BIND_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let err = connect_unix_stream(&path, Duration::from_millis(200)).unwrap_err();
        assert_eq!(err, WireError::ConnectionFailed);
    }

    #[test]
    fn valid_connect_succeeds() {
        let dir = std::env::temp_dir().join(format!("exyonq-conn-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!(
            "ok-{}.sock",
            BIND_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
        let handle = thread::spawn(move || {
            let _ = listener.accept();
        });
        connect_unix_stream(&path, Duration::from_secs(2)).expect("connect");
        handle.join().expect("join");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn timed_out_error_kind_maps_to_timeout() {
        assert_eq!(
            map_connect_error(&std::io::Error::new(ErrorKind::TimedOut, "poll timeout")),
            WireError::Timeout
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn connect_timeout_when_listen_backlog_saturated() {
        let dir = std::env::temp_dir().join(format!("exyonq-sat-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!(
            "sat-{}.sock",
            BIND_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);

        let listener = Socket::new(Domain::UNIX, Type::STREAM, None).expect("socket");
        let addr = SockAddr::unix(&path).expect("addr");
        listener.bind(&addr).expect("bind");
        listener.listen(1).expect("listen");

        let first = connect_unix_stream(&path, Duration::from_millis(500)).expect("first");
        let second = connect_unix_stream(&path, Duration::from_millis(500)).expect("second");
        std::mem::forget((first, second));

        let err = connect_unix_stream(&path, Duration::from_millis(250)).unwrap_err();
        assert!(
            matches!(err, WireError::Timeout | WireError::ConnectionFailed),
            "saturated backlog must fail connect (timeout or refused), got {err:?}"
        );

        let _ = std::fs::remove_file(&path);
    }
}
