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
use exyonq_core::AppConfig;
use std::path::PathBuf;

#[test]
fn rejects_invalid_config_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/invalid-version.toml");
    let err = AppConfig::from_file(&path).unwrap_err();
    assert!(matches!(
        err,
        exyonq_core::ConfigError::UnsupportedVersion { found: 99, .. }
    ));
}

#[test]
fn loads_minimal_config_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml");
    let config = AppConfig::from_file(&path).expect("minimal config");
    assert_eq!(config.routes.len(), 1);
}
