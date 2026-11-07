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
//! `exyonqctl config migrate` contract (Q-11): a v1 -> v2 rewrite changes only
//! the `config_version` line, keeps a rollback copy, and reports drift through
//! `--check`. Every case runs the real binary.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const V1: &str = r#"# operator note stays put
config_version = 1

[[server]]
listen = "127.0.0.1:18080"
routes = ["r"]

[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#;

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("exyonqctl-migrate-{tag}-{nanos}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ctl(args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_exyonqctl");
    Command::new(exe)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to exec {exe}: {e}"))
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn check_reports_needed_without_writing() {
    let dir = temp_dir("check");
    let cfg = dir.join("config.toml");
    fs::write(&cfg, V1).unwrap();

    let out = ctl(&[
        "config",
        "migrate",
        "--config",
        cfg.to_str().unwrap(),
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("MIGRATION=NEEDED"));
    assert_eq!(fs::read_to_string(&cfg).unwrap(), V1);
    assert!(!dir.join("config.toml.v1.bak").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn migrate_in_place_changes_only_the_version_line_and_keeps_a_backup() {
    let dir = temp_dir("inplace");
    let cfg = dir.join("config.toml");
    fs::write(&cfg, V1).unwrap();

    let out = ctl(&["config", "migrate", "--config", cfg.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let migrated = fs::read_to_string(&cfg).unwrap();
    assert!(migrated.starts_with("# operator note stays put\nconfig_version = 2\n"));

    let before: Vec<&str> = V1.lines().collect();
    let after: Vec<&str> = migrated.lines().collect();
    assert_eq!(before.len(), after.len());
    let changed: Vec<usize> = before
        .iter()
        .zip(after.iter())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(changed, vec![1], "only the config_version line may change");

    let backup = dir.join("config.toml.v1.bak");
    assert_eq!(fs::read_to_string(&backup).unwrap(), V1);

    // The migrated file still validates through the offline linter.
    let lint = ctl(&["config", "lint", cfg.to_str().unwrap()]);
    assert!(lint.status.success(), "stderr: {}", stderr(&lint));

    // A second run is a no-op; the existing backup is left untouched.
    let again = ctl(&["config", "migrate", "--config", cfg.to_str().unwrap()]);
    assert!(again.status.success(), "stderr: {}", stderr(&again));
    assert!(stdout(&again).contains("nothing to do"));
    assert_eq!(fs::read_to_string(&backup).unwrap(), V1);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn output_leaves_the_source_untouched() {
    let dir = temp_dir("output");
    let cfg = dir.join("config.toml");
    let out_path = dir.join("config.v2.toml");
    fs::write(&cfg, V1).unwrap();

    let out = ctl(&[
        "config",
        "migrate",
        "--config",
        cfg.to_str().unwrap(),
        "--output",
        out_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(fs::read_to_string(&cfg).unwrap(), V1);
    assert!(fs::read_to_string(&out_path)
        .unwrap()
        .contains("config_version = 2"));
    assert!(!dir.join("config.toml.v1.bak").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn invalid_config_is_rejected_without_touching_the_file() {
    let dir = temp_dir("invalid");
    let cfg = dir.join("config.toml");
    let broken = "config_version = 1\nunknown_key = 1\n";
    fs::write(&cfg, broken).unwrap();

    let out = ctl(&["config", "migrate", "--config", cfg.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert_eq!(fs::read_to_string(&cfg).unwrap(), broken);
    assert!(!dir.join("config.toml.v1.bak").exists());
    let _ = fs::remove_dir_all(dir);
}
