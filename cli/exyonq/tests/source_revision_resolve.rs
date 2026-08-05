//! Source-revision contract tests (P14V042 Phase 1).
//! Exercises the same resolver used by `build.rs` without mutating process env.

#[path = "../build_support.rs"]
mod build_support;

use build_support::{is_canonical_git_sha40, resolve_source_revision_with};

fn sha40(byte: u8) -> String {
    format!("{:02x}", byte).repeat(20)
}

#[test]
fn accepts_canonical_sha40() {
    let s = sha40(0xab);
    assert!(is_canonical_git_sha40(&s));
    assert!(!is_canonical_git_sha40("unknown"));
    assert!(!is_canonical_git_sha40(&s[..39]));
    assert!(!is_canonical_git_sha40(&format!("{}A", &s[..39]))); // uppercase
}

#[test]
fn development_allows_unknown_when_git_absent() {
    let r = resolve_source_revision_with(None, None, false).unwrap();
    assert_eq!(r, "unknown");
}

#[test]
fn development_prefers_git_when_no_force() {
    let head = sha40(0x11);
    let r = resolve_source_revision_with(None, Some(&head), false).unwrap();
    assert_eq!(r, head);
}

#[test]
fn explicit_valid_revision_wins() {
    let forced = sha40(0x22);
    let git = sha40(0x33);
    let r = resolve_source_revision_with(Some(&forced), Some(&git), false).unwrap();
    assert_eq!(r, forced);
}

#[test]
fn malformed_explicit_revision_rejected_in_dev() {
    let err = resolve_source_revision_with(Some("not-a-sha"), None, false).unwrap_err();
    assert!(err.contains("40 lowercase hex"), "{err}");
}

#[test]
fn official_rejects_unknown() {
    let err = resolve_source_revision_with(None, None, true).unwrap_err();
    assert!(err.contains("forbids source_revision=unknown"), "{err}");
}

#[test]
fn official_rejects_malformed() {
    let err = resolve_source_revision_with(Some("deadbeef"), None, true).unwrap_err();
    assert!(err.contains("40 lowercase hex"), "{err}");
}

#[test]
fn official_accepts_canonical_sha() {
    let head = sha40(0x44);
    let r = resolve_source_revision_with(Some(&head), None, true).unwrap();
    assert_eq!(r, head);
}

#[test]
fn amd64_arm64_same_forced_revision() {
    // Dual-arch contract: both builders must receive the same explicit SHA.
    let head = sha40(0x55);
    let amd = resolve_source_revision_with(Some(&head), None, true).unwrap();
    let arm = resolve_source_revision_with(Some(&head), None, true).unwrap();
    assert_eq!(amd, arm);
    assert_eq!(amd, head);
}
