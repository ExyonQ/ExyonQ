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
//! EXQ-092: `exyonqctl status` / `reload` operator-facing errors and exit
//! codes. Every case runs the real binary; nothing here is mocked.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const VALID: &str = r#"config_version = 1

[[server]]
listen = "127.0.0.1:18460"
routes = ["r"]

[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#;

const INVALID_TOML: &str = r#"config_version = 1

[[server]]
listen = 18460
"#;

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("exy-092-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn ctl(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_exyonqctl"));
    cmd.args(args)
        .env_remove("EXYONQ_CONFIG")
        .env_remove("EXYONQ_CONTROL_SOCKET");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("run exyonqctl")
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

#[test]
fn status_without_socket_says_what_to_pass_and_exits_4() {
    let dir = temp_dir("status-absent");
    let socket = dir.join("absent.sock");
    let out = ctl(
        &["status"],
        &[("EXYONQ_CONTROL_SOCKET", socket.to_str().unwrap())],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code(&out), 4, "stderr: {stderr}");
    assert!(stderr.contains("control socket not found"), "{stderr}");
    assert!(stderr.contains("--socket"), "{stderr}");
    assert!(stderr.contains("EXYONQ_CONTROL_SOCKET"), "{stderr}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn status_with_empty_socket_env_exits_4() {
    let out = ctl(&["status"], &[("EXYONQ_CONTROL_SOCKET", "")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code(&out), 4, "stderr: {stderr}");
    assert!(stderr.contains("set but empty"), "{stderr}");
}

#[test]
fn reload_with_missing_config_exits_2() {
    let dir = temp_dir("reload-missing");
    let cfg = dir.join("nope.toml");
    let out = ctl(&["reload", "--config", cfg.to_str().unwrap()], &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code(&out), 2, "stderr: {stderr}");
    assert!(stderr.contains("config not found"), "{stderr}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn reload_with_invalid_config_reports_the_diagnostic_and_exits_3() {
    let dir = temp_dir("reload-invalid");
    let cfg = dir.join("broken.toml");
    fs::write(&cfg, INVALID_TOML).unwrap();
    let socket = dir.join("absent.sock");
    let out = ctl(
        &["reload", "--config", cfg.to_str().unwrap()],
        &[("EXYONQ_CONTROL_SOCKET", socket.to_str().unwrap())],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code(&out), 3, "stderr: {stderr}");
    // The TOML diagnostic must reach the operator, not only the journal.
    assert!(stderr.contains("EXY-CONFIG"), "{stderr}");
    assert!(stderr.contains("config invalid"), "{stderr}");
    assert!(stderr.contains("was not asked to reload"), "{stderr}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn reload_with_valid_config_and_absent_socket_exits_4() {
    let dir = temp_dir("reload-socket");
    let cfg = dir.join("ok.toml");
    fs::write(&cfg, VALID).unwrap();
    let socket = dir.join("absent.sock");
    let out = ctl(
        &["reload", "--config", cfg.to_str().unwrap()],
        &[("EXYONQ_CONTROL_SOCKET", socket.to_str().unwrap())],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code(&out), 4, "stderr: {stderr}");
    assert!(stderr.contains("control socket not found"), "{stderr}");
    let _ = fs::remove_dir_all(dir);
}
