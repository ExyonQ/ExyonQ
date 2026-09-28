//! Workspace bootstrap integration check (not a product protocol E2E).

#[test]
fn workspace_version_nonempty() {
    assert!(!exyonq_core::VERSION.is_empty());
}
