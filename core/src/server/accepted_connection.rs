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
//! PS3A-F3 I1 — owned accepted-connection bundle (mechanism → core policy).
//!
//! **INTERNAL WORKSPACE CONTRACT — NOT STABLE PUBLIC API.**
//! No policy state. Deliverable for [`crate::kernel::PlatformConnectionEntry`].
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use bytes::Bytes;
use std::net::{SocketAddr, TcpStream};

/// Which Linux accept/serve path produced the connection (metadata only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    SyncAccept,
    Epoll,
    IoUring,
}

/// Owned delivery unit from worker mechanism to core policy.
///
/// Socket ownership transfers with this value. Not [`Clone`].
pub struct AcceptedConnection {
    pub stream: TcpStream,
    pub peer: SocketAddr,
    pub prefetched: Option<(Bytes, Bytes)>,
    pub transport: TransportKind,
}

impl AcceptedConnection {
    pub fn new(
        stream: TcpStream,
        peer: SocketAddr,
        prefetched: Option<(Bytes, Bytes)>,
        transport: TransportKind,
    ) -> Self {
        Self {
            stream,
            peer,
            prefetched,
            transport,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn i1_accepted_connection_is_not_clone() {
        fn assert_not_clone<T>() {}
        assert_not_clone::<AcceptedConnection>();
    }

    #[test]
    fn i1_accepted_connection_owns_stream_and_peer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (server, peer) = listener.accept().unwrap();
        drop(server);
        let conn = AcceptedConnection::new(client, peer, None, TransportKind::SyncAccept);
        assert_eq!(conn.peer, peer);
        assert!(conn.prefetched.is_none());
        assert_eq!(conn.transport, TransportKind::SyncAccept);
        drop(conn);
    }

    #[test]
    fn i1_no_policy_fields_in_type() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (_s, peer) = listener.accept().unwrap();
        let _ = AcceptedConnection {
            stream: client,
            peer,
            prefetched: Some((Bytes::new(), Bytes::new())),
            transport: TransportKind::Epoll,
        };
    }
}
