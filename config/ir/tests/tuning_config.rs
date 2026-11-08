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
