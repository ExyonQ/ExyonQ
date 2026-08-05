use exyonq_compat_nginx::{migrate_source, MigrateOptions, NginxCompatStatus};
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

fn has_entry(
    report: &exyonq_compat_nginx::CompatibilityReport,
    directive: &str,
    needle: &str,
) -> bool {
    report.entries.iter().any(|e| {
        e.directive == directive
            && (e.message.contains(needle)
                || e.semantic_form
                    .as_deref()
                    .is_some_and(|s| s.contains(needle)))
    })
}

#[test]
fn tier2b_exact_and_prefix_precedence_risk() {
    let (toml, report) = migrate_fixture("tier2b-exact-prefix.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(report.summary.exact_locations, 1);
    assert_eq!(report.summary.prefix_locations, 1);
    assert!(report.summary.location_precedence_risks >= 1);
    assert!(has_entry(&report, "location", "/health"));
    assert_eq!(
        cfg.routes.len(),
        0,
        "empty location blocks produce no routes"
    );
}

#[test]
fn tier2b_preferential_prefix_warns_regex_priority() {
    let (_, report) = migrate_fixture("tier2b-preferential-prefix.conf");
    assert_eq!(report.summary.preferential_prefixes, 1);
    assert_eq!(report.summary.regex_locations, 1);
    assert!(report.summary.location_precedence_risks >= 1);
    assert!(has_entry(&report, "location", "^~"));
}

#[test]
fn tier2b_regex_fastcgi_does_not_emit_literal_route() {
    let (toml, report) = migrate_fixture("tier2b-regex-fastcgi.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(report.summary.regex_locations, 1);
    assert!(
        cfg.routes.is_empty(),
        "regex location must not map to route"
    );
    assert!(!toml.contains(r"\.php$"));
    assert!(has_entry(&report, "location", "regex"));
}

#[test]
fn tier2b_named_location_unsupported() {
    let (toml, report) = migrate_fixture("tier2b-named-location.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(report.summary.named_locations, 1);
    assert!(cfg.routes.is_empty());
    assert!(has_entry(&report, "location", "@fallback"));
}

#[test]
fn tier2b_try_files_404_partial() {
    let (_, report) = migrate_fixture("tier2b-try-files-404.conf");
    assert_eq!(report.summary.try_files_directives, 1);
    let entry = report
        .entries
        .iter()
        .find(|e| e.directive == "try_files" && e.semantic_form.is_some())
        .expect("try_files entry");
    assert_eq!(entry.status, NginxCompatStatus::Partial);
    assert!(entry
        .semantic_form
        .as_deref()
        .unwrap_or("")
        .contains("=404"));
}

#[test]
fn tier2b_try_files_spa_fallback_partial() {
    let (_, report) = migrate_fixture("tier2b-try-files-spa.conf");
    let entry = report
        .entries
        .iter()
        .find(|e| e.directive == "try_files" && e.semantic_form.is_some())
        .expect("try_files");
    assert_eq!(entry.status, NginxCompatStatus::Partial);
    assert!(entry
        .migration_action
        .as_deref()
        .unwrap_or("")
        .contains("internal fallback"));
}

#[test]
fn tier2b_try_files_named_fallback_unsupported() {
    let (_, report) = migrate_fixture("tier2b-try-files-named.conf");
    let entry = report
        .entries
        .iter()
        .find(|e| e.directive == "try_files" && e.message.contains("@fallback"))
        .expect("named fallback");
    assert_eq!(entry.status, NginxCompatStatus::Unsupported);
    assert!(entry
        .migration_action
        .as_deref()
        .unwrap_or("")
        .contains("named-location"));
}

#[test]
fn tier2b_try_files_missing_named_location_error() {
    let output = migrate_source(
        "tier2b-try-files-named-missing.conf",
        &read_fixture("tier2b-try-files-named-missing.conf"),
        &MigrateOptions::default(),
    )
    .expect("migrate");
    assert!(output.report.summary.errors >= 1);
    assert_eq!(output.report.summary.unresolved_named_locations, 1);
    assert_eq!(output.exit_code(true), 1);
}

#[test]
fn tier2b_try_files_php_front_controller_unsupported() {
    let (_, report) = migrate_fixture("tier2b-try-files-php.conf");
    let entry = report
        .entries
        .iter()
        .find(|e| e.directive == "try_files" && e.semantic_form.is_some())
        .expect("try_files");
    assert_eq!(entry.status, NginxCompatStatus::Unsupported);
    assert!(entry.message.contains("query"));
}

#[test]
fn tier2b_rewrite_safe_redirect_maps_route() {
    let (toml, report) = migrate_fixture("tier2b-rewrite-safe.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(report.summary.safe_redirect_rewrites, 1);
    let redirect = cfg.routes[0].redirect.as_ref().expect("redirect");
    assert_eq!(redirect.status, 301);
    assert_eq!(redirect.location, "/new");
}

#[test]
fn tier2b_rewrite_capture_unsupported() {
    let (toml, report) = migrate_fixture("tier2b-rewrite-capture.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert!(cfg.routes.is_empty() || cfg.routes[0].redirect.is_none());
    assert_eq!(report.summary.internal_rewrites_unsupported, 0);
    assert!(has_entry(&report, "rewrite", "capture"));
}

#[test]
fn tier2b_rewrite_internal_last_unsupported() {
    let (_, report) = migrate_fixture("tier2b-rewrite-internal.conf");
    assert_eq!(report.summary.internal_rewrites_unsupported, 1);
    let entry = report
        .entries
        .iter()
        .find(|e| e.directive == "rewrite")
        .expect("rewrite");
    assert_eq!(entry.status, NginxCompatStatus::Unsupported);
    assert!(entry.message.contains("last"));
}

#[test]
fn tier2b_return_special_codes() {
    let (toml, report) = migrate_fixture("tier2b-return-codes.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert!(cfg.routes.is_empty());
    assert!(has_entry(&report, "return", "444"));
    assert!(has_entry(&report, "return", "204"));
    assert!(has_entry(&report, "return", "200"));
}

#[test]
fn tier2b_strict_exit_on_partial() {
    let output = migrate_source(
        "tier2b-try-files-spa.conf",
        &read_fixture("tier2b-try-files-spa.conf"),
        &MigrateOptions {
            strict: true,
            ..Default::default()
        },
    )
    .expect("migrate");
    assert_eq!(output.exit_code(true), 1);
}

#[test]
fn tier2b_json_report_has_stable_fields() {
    let output = migrate_source(
        "tier2b-rewrite-safe.conf",
        &read_fixture("tier2b-rewrite-safe.conf"),
        &MigrateOptions {
            report_format: exyonq_compat_nginx::ReportFormat::Json,
            ..Default::default()
        },
    )
    .expect("migrate");
    let json = output.report.render_json().expect("json");
    assert!(json.contains("migration_action"));
    assert!(json.contains("semantic_form"));
    assert!(json.contains("rewrite"));
}
