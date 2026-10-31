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
//! Embed honest release metadata for `exyonq --version`.
//! No builder paths or secrets — only product/target/profile/revision/artifact class.

#[path = "build_support.rs"]
mod build_support;

use std::process::Command;

fn main() {
    let revision = match build_support::resolve_source_revision(git_rev_parse_head()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    println!("cargo:rustc-env=EXYONQ_SOURCE_REVISION={revision}");

    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=EXYONQ_TARGET={target}");

    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=EXYONQ_PROFILE={profile}");

    let artifact = std::env::var("EXYONQ_ARTIFACT_VERSION").unwrap_or_else(|_| {
        std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string())
    });
    println!("cargo:rustc-env=EXYONQ_ARTIFACT_VERSION={artifact}");

    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
    println!("cargo:rerun-if-env-changed=EXYONQ_SOURCE_REVISION");
    println!("cargo:rerun-if-env-changed=EXYONQ_ARTIFACT_VERSION");
    println!("cargo:rerun-if-env-changed=EXYONQ_OFFICIAL_RELEASE");
    println!("cargo:rerun-if-env-changed=EXYONQ_REQUIRE_SOURCE_REVISION");
}

fn git_rev_parse_head() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
