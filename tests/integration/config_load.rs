use exyonq_core::AppConfig;
use std::path::PathBuf;

#[test]
fn rejects_invalid_config_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/invalid-version.toml");
    let err = AppConfig::from_file(&path).unwrap_err();
    assert!(matches!(
        err,
        exyonq_core::ConfigError::UnsupportedVersion { found: 99, .. }
    ));
}

#[test]
fn loads_minimal_config_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/minimal.toml");
    let config = AppConfig::from_file(&path).expect("minimal config");
    assert_eq!(config.routes.len(), 1);
}
