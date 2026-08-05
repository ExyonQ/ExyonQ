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
//! Hyper handoff bundle — PS1B ownership validated, PS1C contract surface.

use crate::kernel::generation::GenerationView;
use crate::lifecycle::ConnectionLifecycleToken;
use bytes::Bytes;
use std::net::{SocketAddr, TcpStream};

/// Owned handoff payload: one consumer, no Clone.
///
/// Platform constructs; kernel consumer (`spawn_hyper_handoff`) owns until Hyper dispatch ends.
pub struct HyperHandoff {
    pub stream: TcpStream,
    pub peer: SocketAddr,
    pub prefetched: Option<(Bytes, Bytes)>,
    pub generation: GenerationView,
    pub token: ConnectionLifecycleToken,
}

impl HyperHandoff {
    pub fn new(
        stream: TcpStream,
        peer: SocketAddr,
        prefetched: Option<(Bytes, Bytes)>,
        generation: impl Into<GenerationView>,
        token: ConnectionLifecycleToken,
    ) -> Self {
        Self {
            stream,
            peer,
            prefetched,
            generation: generation.into(),
            token,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::LifecycleState;

    #[test]
    fn ps1c_hyper_handoff_not_clone() {
        fn assert_not_clone<T>() {}
        assert_not_clone::<HyperHandoff>();
    }

    #[test]
    fn ps1c_handoff_preserves_generation_and_peer() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        let generation = GenerationView::pinned(11, false, Some(2));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, peer) = listener.accept().unwrap();
        drop(server);
        let handoff = HyperHandoff::new(stream, peer, None, generation, token);
        assert_eq!(handoff.generation.generation, 11);
        assert_eq!(handoff.peer, peer);
        drop(handoff.token);
        assert_eq!(ops.active_connections(), 0);
    }
}
