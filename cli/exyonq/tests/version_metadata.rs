//! Version metadata honesty tests (P1.6-WS3).
//! Does not require RC artifacts.

use std::process::Command;

fn run_version() -> String {
    let exe = env!("CARGO_BIN_EXE_exyonq");
    let out = Command::new(exe)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("failed to exec {exe}: {e}"));
    assert!(
        out.status.success(),
        "exyonq --version failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // clap may print version on stdout or stderr depending on version; accept both.
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    if s.trim().is_empty() {
        s = String::from_utf8_lossy(&out.stderr).into_owned();
    }
    s
}

#[test]
fn exyonq_version_reports_workspace_pkg_version() {
    let v = run_version();
    let pkg = env!("CARGO_PKG_VERSION");
    assert!(
        v.contains(pkg),
        "exyonq --version must contain CARGO_PKG_VERSION={pkg}; got:\n{v}"
    );
    assert!(
        v.contains("product_version="),
        "expected product_version= field:\n{v}"
    );
    assert!(
        v.contains("source_revision="),
        "expected source_revision= field:\n{v}"
    );
    assert!(v.contains("target="), "expected target= field:\n{v}");
    assert!(v.contains("profile="), "expected profile= field:\n{v}");
    assert!(
        v.contains("allocator="),
        "expected allocator= field:\n{v}"
    );
    assert!(
        v.contains("artifact_version="),
        "expected artifact_version= field:\n{v}"
    );
}

#[test]
fn exyonq_version_does_not_claim_rc_suffix_while_dev_is_plain() {
    let v = run_version();
    let pkg = env!("CARGO_PKG_VERSION");
    // WS3 keeps productive version without -rc until declare/admit.
    if !pkg.contains("-rc.") {
        assert!(
            !v.lines().next().unwrap_or("").contains("-rc."),
            "short version line must not invent -rc while package is {pkg}:\n{v}"
        );
    }
}
