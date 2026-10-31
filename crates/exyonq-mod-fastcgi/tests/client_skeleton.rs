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
use exyonq_mod_fastcgi::{
    ClientError, PhpFpmClient, TransportError, FCGI_VERSION_1, RECORD_HEADER_LEN,
};

fn minimal_frame() -> [u8; RECORD_HEADER_LEN] {
    [FCGI_VERSION_1, 1, 0, 1, 0, 0, 0, 0]
}

#[test]
fn client_exposes_pool_label_only() {
    let client = PhpFpmClient::new("php");
    assert_eq!(client.pool_label().name, "php");
}

#[test]
fn forward_request_is_not_implemented() {
    let client = PhpFpmClient::new("php");
    let frame = minimal_frame();
    assert_eq!(
        client.forward_request(&[&frame]),
        Err(ClientError::InertUnavailable)
    );
}

#[test]
fn submit_frame_uses_inert_transport() {
    let client = PhpFpmClient::new("php");
    assert_eq!(
        client.submit_frame(&minimal_frame()),
        Err(ClientError::Transport(TransportError::InertUnavailable))
    );
}

#[test]
fn client_skeleton_has_no_socket_fields() {
    for path in ["src/client.rs", "src/transport.rs", "src/scripted.rs"] {
        let content = std::fs::read_to_string(format!("{}/{}", env!("CARGO_MANIFEST_DIR"), path))
            .expect("read src");
        for forbidden in ["TcpStream", "UnixStream", "connect(", "tokio::net"] {
            assert!(
                !content.contains(forbidden),
                "{path} must not reference `{forbidden}`"
            );
        }
    }
}

#[test]
fn default_client_is_inert() {
    let client = PhpFpmClient::default();
    assert_eq!(client.pool_label().name, "default");
    assert!(client.forward_request(&[]).is_err());
}
