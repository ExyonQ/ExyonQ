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
//! Source-revision contract tests (P14V042 Phase 1).
//! Exercises the same resolver used by `build.rs` without mutating process env.

#[path = "../build_support.rs"]
mod build_support;

use build_support::{
    is_canonical_git_sha40, resolve_source_revision_with, resolve_source_tag_with,
};

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

#[test]
fn source_tag_empty_without_forced_or_git() {
    let r = resolve_source_tag_with(None, None, false, "0.9.0").unwrap();
    assert_eq!(r, "");
}

#[test]
fn source_tag_prefers_forced_env() {
    let r = resolve_source_tag_with(Some("v0.9.1"), Some("v0.9.0"), false, "0.9.0").unwrap();
    assert_eq!(r, "v0.9.1");
}

#[test]
fn source_tag_uses_exact_git_tag() {
    let r = resolve_source_tag_with(None, Some("v0.9.0"), false, "0.9.0").unwrap();
    assert_eq!(r, "v0.9.0");
}

#[test]
fn source_tag_whitespace_forced_falls_back_to_git() {
    let r = resolve_source_tag_with(Some("  "), Some("v0.9.0"), false, "0.9.0").unwrap();
    assert_eq!(r, "v0.9.0");
}

#[test]
fn official_source_tag_required() {
    let err = resolve_source_tag_with(None, None, true, "0.9.0").unwrap_err();
    assert!(err.contains("EXYONQ_SOURCE_TAG"), "{err}");
}

#[test]
fn official_source_tag_must_match_product_version() {
    let err = resolve_source_tag_with(Some("v0.9.1"), None, true, "0.9.0").unwrap_err();
    assert!(err.contains("v0.9.0"), "{err}");
}

#[test]
fn official_source_tag_accepts_matching_version() {
    let r = resolve_source_tag_with(Some("v0.9.0"), None, true, "0.9.0").unwrap();
    assert_eq!(r, "v0.9.0");
}
