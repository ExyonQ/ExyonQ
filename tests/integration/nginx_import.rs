use exyonq_compat_nginx::migrate;
use exyonq_config_ir::AppConfig;
use exyonq_core::{compile_runtime_plan, AppConfig as CoreAppConfig, Backend};
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/nginx")
        .join(name)
}

fn migrate_fixture(name: &str) -> (String, exyonq_compat_common::MigrationReport) {
    let input = std::fs::read_to_string(fixture_path(name)).expect("read nginx fixture");
    migrate(&input).expect("migrate fixture")
}

#[test]
fn migrates_proxy_fixture_into_valid_app_config() {
    let (output, report) = migrate_fixture("proxy.conf");
    let parsed = AppConfig::parse_str(&output).expect("generated TOML parses");
    assert_eq!(parsed.routes.len(), 1);
    assert_eq!(parsed.upstreams.len(), 1);
    assert!(report.unsupported.is_empty());
}

#[test]
fn migrates_static_fixture_into_valid_app_config() {
    let (output, report) = migrate_fixture("static.conf");
    let parsed = AppConfig::parse_str(&output).expect("generated TOML parses");
    assert_eq!(parsed.routes.len(), 1);
    assert!(parsed.routes[0].root.is_some());
    assert!(report.unsupported.is_empty());
}

#[test]
fn migrates_return_404_fixture_partial() {
    let (output, report) = migrate_fixture("return404.conf");
    let parsed = AppConfig::parse_str(&output).expect("generated TOML parses");
    assert!(parsed.routes.is_empty() || parsed.routes[0].root.is_none());
    assert!(
        report.notes.iter().any(|n| n.message.contains("404"))
            || report.unsupported.iter().any(|u| u.contains("404")),
        "expected return 404 compatibility note"
    );
}

#[test]
fn migrates_v2_fixtures_without_fatal_errors() {
    let v2_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/nginx/v2");
    for entry in std::fs::read_dir(&v2_dir).expect("v2 dir") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("conf") {
            continue;
        }
        let input = std::fs::read_to_string(&path).expect("read v2 fixture");
        let (output, _report) = migrate(&input).expect("migrate v2 fixture");
        let name = path.file_name().unwrap().to_string_lossy();
        AppConfig::parse_str(&output).unwrap_or_else(|e| panic!("{name} TOML invalid: {e}"));
    }
}

#[test]
fn migrated_fastcgi_unix_document_root_reaches_runtime_slots() {
    let (output, _) = migrate_fixture("tier1-fastcgi-unix.conf");
    let parsed: CoreAppConfig = output.parse().expect("generated TOML parses");
    let pool = parsed.pools_fcgi.get("srv1_loc1_fcgi").expect("fcgi pool");
    assert_eq!(
        pool.document_root.as_ref().map(|p| p.display().to_string()),
        Some("/srv/php".into())
    );
    // P1.6-WS2 UNIT_2 / KF-P16-001: compile_snapshot was removed; use compile_runtime_plan.
    let snap = compile_runtime_plan(1, parsed).expect("compile runtime plan");
    let backend = snap.resolve_backend(0).expect("fastcgi route backend");
    let pool_id = match backend {
        Backend::Fastcgi { pool_id } => *pool_id,
        _ => panic!("expected fastcgi backend"),
    };
    assert_eq!(
        snap.fcgi_pool_document_root(pool_id)
            .map(|p| p.display().to_string()),
        Some("/srv/php".into())
    );
}
