//! Embed honest release metadata for `exyonqctl --version`.
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
        std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.4.9".to_string())
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
