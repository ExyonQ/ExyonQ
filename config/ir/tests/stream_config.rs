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
//! NG-20 `[[stream]]` IR contract: parse/defaults, fail-closed validation and
//! fingerprint participation.

use exyonq_config_ir::{
    fingerprint, AppConfig, StreamProtocol, DEFAULT_STREAM_IDLE_TIMEOUT_MS,
    DEFAULT_STREAM_MAX_SESSIONS,
};

fn with_stream(stream: &str) -> String {
    format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:18080"
routes = []

[[upstream]]
name = "backend"
timeout_ms = 2000

[[upstream.endpoints]]
id = "backend-1"
address = "127.0.0.1"
port = 19000

{stream}
"#
    )
}

fn parse(stream: &str) -> Result<AppConfig, String> {
    AppConfig::parse_str(&with_stream(stream)).map_err(|err| err.to_string())
}

#[test]
fn stream_defaults_are_tcp_and_bounded() {
    let config = parse(
        r#"
[[stream]]
name = "mysql"
listen = "127.0.0.1:13306"
upstream = "backend"
"#,
    )
    .expect("stream parses");

    assert_eq!(config.streams.len(), 1);
    let stream = &config.streams[0];
    assert_eq!(stream.protocol, StreamProtocol::Tcp);
    assert_eq!(stream.idle_timeout_ms, DEFAULT_STREAM_IDLE_TIMEOUT_MS);
    assert_eq!(stream.max_sessions, DEFAULT_STREAM_MAX_SESSIONS);
}

#[test]
fn stream_explicit_values_parse() {
    let config = parse(
        r#"
[[stream]]
name = "redis"
listen = "127.0.0.1:16379"
protocol = "udp"
upstream = "backend"
idle_timeout_ms = 1500
max_sessions = 32
"#,
    )
    .expect("stream parses");

    let stream = &config.streams[0];
    assert_eq!(stream.protocol, StreamProtocol::Udp);
    assert_eq!(stream.idle_timeout_ms, 1500);
    assert_eq!(stream.max_sessions, 32);
}

#[test]
fn stream_rejects_unknown_upstream() {
    let error = parse(
        r#"
[[stream]]
name = "mysql"
listen = "127.0.0.1:13306"
upstream = "missing"
"#,
    )
    .expect_err("unknown upstream must fail");
    assert!(error.contains("unknown upstream"), "{error}");
}

#[test]
fn stream_rejects_http_listen_collision() {
    let error = parse(
        r#"
[[stream]]
name = "mysql"
listen = "127.0.0.1:18080"
upstream = "backend"
"#,
    )
    .expect_err("a stream must not shadow the HTTP listener");
    assert!(error.contains("already bound"), "{error}");
}

#[test]
fn udp_stream_may_share_a_port_with_http_tcp() {
    let config = parse(
        r#"
[[stream]]
name = "quic-udp"
listen = "127.0.0.1:18080"
protocol = "udp"
upstream = "backend"
"#,
    )
    .expect("UDP and TCP are separate port spaces");
    assert_eq!(config.streams[0].protocol, StreamProtocol::Udp);
}

#[test]
fn stream_rejects_duplicate_stream_listen_and_name() {
    let duplicate_listen = parse(
        r#"
[[stream]]
name = "one"
listen = "127.0.0.1:13306"
upstream = "backend"

[[stream]]
name = "two"
listen = "127.0.0.1:13306"
upstream = "backend"
"#,
    )
    .expect_err("duplicate L4 bind must fail");
    assert!(
        duplicate_listen.contains("already bound"),
        "{duplicate_listen}"
    );

    let duplicate_name = parse(
        r#"
[[stream]]
name = "same"
listen = "127.0.0.1:13306"
upstream = "backend"

[[stream]]
name = "same"
listen = "127.0.0.1:13307"
upstream = "backend"
"#,
    )
    .expect_err("duplicate stream name must fail");
    assert!(duplicate_name.contains("duplicate"), "{duplicate_name}");
}

#[test]
fn stream_rejects_out_of_range_timeouts_and_session_caps() {
    for field in ["idle_timeout_ms = 0", "max_sessions = 0"] {
        let error = parse(&format!(
            r#"
[[stream]]
name = "bad"
listen = "127.0.0.1:13306"
upstream = "backend"
{field}
"#
        ))
        .expect_err("out-of-range stream field must fail");
        assert!(error.contains("stream `bad`"), "{error}");
    }
}

#[test]
fn stream_rejects_unknown_protocol() {
    let error = parse(
        r#"
[[stream]]
name = "bad"
listen = "127.0.0.1:13306"
protocol = "sctp"
upstream = "backend"
"#,
    )
    .expect_err("unknown protocol must fail");
    assert!(
        error.contains("sctp") || error.contains("unknown variant"),
        "{error}"
    );
}

#[test]
fn stream_fingerprint_tracks_l4_contract() {
    let one = parse(
        r#"
[[stream]]
name = "mysql"
listen = "127.0.0.1:13306"
upstream = "backend"
idle_timeout_ms = 1000
"#,
    )
    .expect("first stream");
    let two = parse(
        r#"
[[stream]]
name = "mysql"
listen = "127.0.0.1:13306"
upstream = "backend"
idle_timeout_ms = 2000
"#,
    )
    .expect("second stream");

    assert_ne!(
        fingerprint(&one),
        fingerprint(&two),
        "changing an L4 timeout must change the reload fingerprint"
    );
}
