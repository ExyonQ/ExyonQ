use exyonq_compat_nginx::{migrate_source, MigrateOptions};
use exyonq_config_ir::AppConfig;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/nginx")
        .join(name)
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).expect("read fixture")
}

fn migrate_fixture(name: &str) -> (String, exyonq_compat_nginx::CompatibilityReport) {
    let output = migrate_source(name, &read_fixture(name), &MigrateOptions::default())
        .expect("migrate fixture");
    (output.config, output.report)
}

#[test]
fn tier2a_named_http_upstream_maps_proxy_route() {
    let (toml, report) = migrate_fixture("tier2a-named-http-upstream.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(cfg.upstreams.len(), 1);
    assert!(cfg.upstreams.contains_key("backend"));
    assert!(cfg.routes[0].upstream.as_deref() == Some("backend"));
    assert!(toml.contains("127.0.0.1:3000"));
    assert!(
        report.entries.iter().any(
            |e| e.status == exyonq_compat_nginx::NginxCompatStatus::Partial
                && e.message.contains("127.0.0.1:3001")
        ),
        "expected PARTIAL for second endpoint"
    );
    assert_eq!(report.summary.http_upstreams_generated, 1);
}

#[test]
fn tier2a_named_fcgi_upstream_maps_pool_and_document_root() {
    let (toml, report) = migrate_fixture("tier2a-named-fcgi-upstream.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let pool = cfg.pools_fcgi.get("php_backend").expect("pool");
    assert_eq!(pool.address, "unix:/run/php/php-fpm.sock");
    assert_eq!(
        pool.document_root.as_ref().map(|p| p.display().to_string()),
        Some("/srv/php".into())
    );
    assert_eq!(cfg.routes[0].fastcgi.as_deref(), Some("php_backend"));
    assert_eq!(report.summary.fcgi_pools_from_upstreams, 1);
    assert!(report.summary.errors == 0);
}

#[test]
fn tier2a_root_inheritance_binds_fcgi_document_root() {
    let (toml, report) = migrate_fixture("tier2a-root-inheritance.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let pool = cfg.pools_fcgi.values().next().expect("pool");
    assert_eq!(
        pool.document_root.as_ref().map(|p| p.display().to_string()),
        Some("/srv/site".into())
    );
    assert!(report.summary.inherited_roots >= 1);
}

#[test]
fn tier2a_root_override_wins_over_server_root() {
    let (toml, _) = migrate_fixture("tier2a-root-override.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let pool = cfg.pools_fcgi.values().next().expect("pool");
    assert_eq!(
        pool.document_root.as_ref().map(|p| p.display().to_string()),
        Some("/srv/app".into())
    );
}

#[test]
fn tier2a_multi_fcgi_upstream_is_unsupported() {
    let output = migrate_source(
        "tier2a-multi-fcgi-upstream.conf",
        &read_fixture("tier2a-multi-fcgi-upstream.conf"),
        &MigrateOptions::default(),
    )
    .expect("migrate");
    assert!(
        output.report.entries.iter().any(|e| e.status
            == exyonq_compat_nginx::NginxCompatStatus::Unsupported
            && e.message.contains("Unix sockets")),
        "expected multi-socket unsupported: {:?}",
        output.report.entries
    );
    assert!(output.report.has_partial_or_unsupported());
    assert!(!output.report.has_errors());
}

#[test]
fn tier2a_missing_upstream_records_error() {
    let output = migrate_source(
        "tier2a-missing-upstream.conf",
        &read_fixture("tier2a-missing-upstream.conf"),
        &MigrateOptions::default(),
    )
    .expect("migrate");
    assert!(output.summary.errors > 0);
    assert!(output.summary.unresolved_upstream_references > 0);
    assert_ne!(output.exit_code(false), 0);
}

#[test]
fn tier2a_duplicate_upstream_records_error() {
    let output = migrate_source(
        "tier2a-duplicate-upstream.conf",
        &read_fixture("tier2a-duplicate-upstream.conf"),
        &MigrateOptions::default(),
    )
    .expect("migrate");
    assert!(
        output.report.entries.iter().any(|e| e.status
            == exyonq_compat_nginx::NginxCompatStatus::Error
            && e.message.contains("duplicate")),
        "expected duplicate upstream error"
    );
}

#[test]
fn tier2a_wildcard_server_name_is_partial_not_literal() {
    let (toml, report) = migrate_fixture("tier2a-wildcard-server-name.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert!(
        cfg.servers
            .iter()
            .all(|s| s.server_name.is_empty() || s.server_name.to_vec().is_empty()),
        "wildcards must not become literal server_name"
    );
    assert!(
        report.entries.iter().any(
            |e| e.status == exyonq_compat_nginx::NginxCompatStatus::Partial
                && e.directive == "server_name"
        ),
        "expected PARTIAL wildcard diagnostics"
    );
    assert!(!toml.contains("*.example.com"));
}

#[test]
fn tier2a_return_body_is_partial_not_mapped() {
    let (toml, report) = migrate_fixture("tier2a-return-body.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert!(cfg.routes.is_empty());
    assert!(
        report.entries.iter().any(
            |e| e.status == exyonq_compat_nginx::NginxCompatStatus::Partial
                && e.directive == "return"
        ),
        "expected PARTIAL return body diagnostics"
    );
}

#[test]
fn tier2a_strict_fails_on_multi_endpoint_upstream() {
    let output = migrate_source(
        "tier2a-named-http-upstream.conf",
        &read_fixture("tier2a-named-http-upstream.conf"),
        &MigrateOptions::default(),
    )
    .expect("migrate");
    assert_ne!(output.exit_code(true), 0);
}
