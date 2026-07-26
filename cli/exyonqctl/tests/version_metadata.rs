//! Version metadata honesty tests for exyonqctl (P1.6-WS3).

use std::process::Command;

fn run_version() -> String {
    let exe = env!("CARGO_BIN_EXE_exyonqctl");
    let out = Command::new(exe)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("failed to exec {exe}: {e}"));
    assert!(
        out.status.success(),
        "exyonqctl --version failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    if s.trim().is_empty() {
        s = String::from_utf8_lossy(&out.stderr).into_owned();
    }
    s
}

#[test]
fn exyonqctl_version_reports_workspace_pkg_version() {
    let v = run_version();
    let pkg = env!("CARGO_PKG_VERSION");
    assert!(
        v.contains(pkg),
        "exyonqctl --version must contain CARGO_PKG_VERSION={pkg}; got:\n{v}"
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
        v.contains("artifact_version="),
        "expected artifact_version= field:\n{v}"
    );
}

#[test]
fn exyonqctl_version_matches_cargo_pkg_not_invented_rc() {
    let v = run_version();
    let pkg = env!("CARGO_PKG_VERSION");
    if !pkg.contains("-rc.") {
        let first = v.lines().next().unwrap_or("");
        assert!(
            !first.contains("-rc."),
            "must not invent -rc while package is {pkg}: {first}"
        );
    }
}
