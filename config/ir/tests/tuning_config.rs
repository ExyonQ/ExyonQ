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
//! `[tuning]` IR contract (NG-21): parse, default, bounds, unknown keys and
//! fingerprint participation.

use exyonq_config_ir::{fingerprint, AppConfig};

const SERVER: &str = "[[server]]\nlisten = \"127.0.0.1:18080\"\n";

fn parse(body: &str) -> Result<AppConfig, String> {
    AppConfig::parse_str(body).map_err(|err| err.to_string())
}

#[test]
fn accept_workers_parses_and_absent_means_auto() {
    let explicit = parse(&format!(
        "config_version = 2\n[tuning]\naccept_workers = 4\n{SERVER}"
    ))
    .expect("tuning config parses");
    assert_eq!(explicit.tuning.accept_workers, Some(4));

    let absent = parse(&format!("config_version = 2\n{SERVER}")).expect("config parses");
    assert_eq!(
        absent.tuning.accept_workers, None,
        "absent [tuning] must mean auto, not a fixed worker count"
    );
}

#[test]
fn accept_workers_rejects_out_of_range() {
    for bad in ["0", "17"] {
        let err = parse(&format!(
            "config_version = 2\n[tuning]\naccept_workers = {bad}\n{SERVER}"
        ))
        .expect_err("out-of-range worker count must fail");
        assert!(
            err.contains("tuning.accept_workers"),
            "error must name the field, got: {err}"
        );
    }
}

#[test]
fn tuning_rejects_unknown_keys() {
    let err = parse(&format!(
        "config_version = 2\n[tuning]\nkeepalive_s = 1\n{SERVER}"
    ))
    .expect_err("unknown tuning key must fail");
    assert!(
        err.contains("keepalive_s") || err.contains("unknown field"),
        "unexpected error: {err}"
    );
}

#[test]
fn tuning_participates_in_the_fingerprint() {
    let one = parse(&format!(
        "config_version = 2\n[tuning]\naccept_workers = 1\n{SERVER}"
    ))
    .unwrap();
    let two = parse(&format!(
        "config_version = 2\n[tuning]\naccept_workers = 2\n{SERVER}"
    ))
    .unwrap();
    assert_ne!(
        fingerprint(&one),
        fingerprint(&two),
        "changing worker geometry must change the fingerprint (reload must see it)"
    );
}

#[test]
fn v1_configs_keep_working_without_tuning() {
    let v1 = parse(&format!("config_version = 1\n{SERVER}")).expect("v1 config still parses");
    assert_eq!(v1.config_version, 1);
    assert_eq!(v1.tuning.accept_workers, None);
}

#[test]
fn keepalive_and_header_cap_parse_with_bounds() {
    let cfg = parse(&format!(
        "config_version = 2\n[tuning]\nkeepalive_timeout_ms = 400\nmax_header_bytes = 16384\n{SERVER}"
    ))
    .expect("tuning geometry parses");
    assert_eq!(cfg.tuning.keepalive_timeout_ms, Some(400));
    assert_eq!(cfg.tuning.max_header_bytes, Some(16_384));

    for bad in ["keepalive_timeout_ms = 0", "keepalive_timeout_ms = 600001"] {
        let err = parse(&format!("config_version = 2\n[tuning]\n{bad}\n{SERVER}"))
            .expect_err("out-of-range keep-alive must fail");
        assert!(err.contains("keepalive_timeout_ms"), "got: {err}");
    }
    for bad in [
        "max_header_bytes = 4096",
        "max_header_bytes = 65535",
        "max_header_bytes = 131072",
    ] {
        let err = parse(&format!("config_version = 2\n[tuning]\n{bad}\n{SERVER}"))
            .expect_err("out-of-range header cap must fail");
        assert!(err.contains("max_header_bytes"), "got: {err}");
    }
}

#[test]
fn header_geometry_participates_in_the_fingerprint() {
    let slow = parse(&format!(
        "config_version = 2\n[tuning]\nkeepalive_timeout_ms = 1000\n{SERVER}"
    ))
    .unwrap();
    let fast = parse(&format!(
        "config_version = 2\n[tuning]\nkeepalive_timeout_ms = 2000\n{SERVER}"
    ))
    .unwrap();
    assert_ne!(fingerprint(&slow), fingerprint(&fast));

    let small = parse(&format!(
        "config_version = 2\n[tuning]\nmax_header_bytes = 8192\n{SERVER}"
    ))
    .unwrap();
    let large = parse(&format!(
        "config_version = 2\n[tuning]\nmax_header_bytes = 16384\n{SERVER}"
    ))
    .unwrap();
    assert_ne!(fingerprint(&small), fingerprint(&large));
}

#[test]
fn absent_geometry_keeps_the_historical_defaults() {
    let cfg = parse(&format!("config_version = 2\n{SERVER}")).expect("config parses");
    assert_eq!(cfg.tuning.keepalive_timeout_ms, None);
    assert_eq!(cfg.tuning.max_header_bytes, None);
}
