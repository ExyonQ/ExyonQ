use exyonq_compat_nginx::{migrate, migrate_file, migrate_source, MigrateOptions};
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

#[test]
fn tier1_static_site_maps_root_route() {
    let (toml, _) = migrate(&read_fixture("tier1-static.conf")).expect("migrate");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(cfg.routes.len(), 1);
    assert_eq!(
        cfg.routes[0].root.as_ref().map(|p| p.display().to_string()),
        Some("/srv/example".into())
    );
    assert!(toml.contains("server_name = \"example.com\""));
}

#[test]
fn tier1_reverse_proxy_maps_upstream() {
    let (toml, _) = migrate(&read_fixture("tier1-proxy.conf")).expect("migrate");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(cfg.routes.len(), 1);
    assert_eq!(cfg.upstreams.len(), 1);
    assert!(cfg.routes[0].upstream.is_some());
    assert!(toml.contains("127.0.0.1:3000"));
}

#[test]
fn tier1_fastcgi_unix_maps_pool_and_root() {
    let (toml, report) = migrate(&read_fixture("tier1-fastcgi-unix.conf")).expect("migrate");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(cfg.pools_fcgi.len(), 1);
    assert!(cfg.routes[0].fastcgi.is_some());
    assert!(cfg.routes[0].root.is_none());
    let pool = cfg.pools_fcgi.values().next().expect("pool");
    assert_eq!(
        pool.document_root.as_ref().map(|p| p.display().to_string()),
        Some("/srv/php".into())
    );
    assert!(report.unsupported.is_empty());
    assert!(toml.contains("unix:/run/php/php-fpm.sock"));
}

#[test]
fn tier1_return_redirect_maps_route_redirect() {
    let (toml, _) = migrate(&read_fixture("tier1-return-redirect.conf")).expect("migrate");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(cfg.routes.len(), 1);
    let redirect = cfg.routes[0].redirect.as_ref().expect("redirect");
    assert_eq!(redirect.status, 301);
    assert!(redirect.location.contains("new.example.com"));
}

#[test]
fn tier1_regex_location_is_unsupported() {
    let (_, report) = migrate(&read_fixture("tier1-regex-location.conf")).expect("migrate");
    assert!(
        report
            .unsupported
            .iter()
            .any(|u| u.contains("regex") || u.contains("location")),
        "expected regex location diagnostic: {:?}",
        report.unsupported
    );
}

#[test]
fn tier1_invalid_syntax_records_error() {
    let result = migrate(&read_fixture("tier1-invalid-syntax.conf"));
    assert!(result.is_err());
}

#[test]
fn tier1_include_expands_from_file() {
    let path = fixture("tier1-include-main.conf");
    let output = migrate_file(&path, &MigrateOptions::default()).expect("migrate file");
    let cfg = AppConfig::parse_str(&output.config).expect("parse");
    assert!(
        cfg.servers.iter().any(|s| s
            .server_name
            .to_vec()
            .contains(&"included.example.com".to_string())),
        "expected included server: {:?}",
        cfg.servers
    );
}

#[test]
fn tier1_report_counts_supported_entries() {
    let output = migrate_source(
        "tier1-static.conf",
        &read_fixture("tier1-static.conf"),
        &MigrateOptions::default(),
    )
    .expect("migrate");
    assert!(output.summary.supported_directives > 0);
    assert_eq!(output.summary.errors, 0);
}

#[test]
fn tier1_strict_exit_on_partial() {
    let output = migrate_source(
        "tier1-regex-location.conf",
        &read_fixture("tier1-regex-location.conf"),
        &MigrateOptions::default(),
    )
    .expect("migrate");
    assert_ne!(output.exit_code(true), 0);
}
