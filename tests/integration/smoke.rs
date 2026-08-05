//! Integration tests for ExyonQ (expanded in Fase 1a).

#[test]
fn workspace_smoke() {
    assert!(!exyonq_core::VERSION.is_empty());
}
