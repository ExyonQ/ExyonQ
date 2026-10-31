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
mod common;

use common::scripted_peer::{cleanup_unix, join_peer, spawn_unix_peer, PeerConfig};
use exyonq_mod_fastcgi::{MinForwardRequest, PhpFpmClient, WireTransport, FCGI_REQUEST_COMPLETE};

#[test]
fn unix_wire_get_roundtrip() {
    let body = b"Content-Type: text/plain\r\n\r\nwire-ok";
    let (path, handle) = spawn_unix_peer(PeerConfig {
        stdout_body: body.to_vec(),
        ..PeerConfig::default()
    });

    let transport = WireTransport::unix(path.clone(), 1);
    let client = PhpFpmClient::with_transport("php", transport);
    let req = MinForwardRequest::get("/index.php", "/index.php", "/var/www/index.php");

    let response = client.forward_min_request(&req).expect("unix roundtrip");
    assert_eq!(response.stdout, body);
    assert_eq!(response.protocol_status, FCGI_REQUEST_COMPLETE);

    join_peer(handle);
    cleanup_unix(&path);
}

#[test]
fn unix_wire_post_with_stdin() {
    let (path, handle) = spawn_unix_peer(PeerConfig::default());
    let transport = WireTransport::unix(path.clone(), 1);
    let client = PhpFpmClient::with_transport("php", transport);
    let req = MinForwardRequest::post(
        "/index.php",
        "/index.php",
        "/var/www/index.php",
        "application/x-www-form-urlencoded",
        b"a=1&b=2",
    );

    let response = client.forward_min_request(&req).expect("post roundtrip");
    assert!(response.stdout.starts_with(b"Content-Type:"));

    join_peer(handle);
    cleanup_unix(&path);
}
