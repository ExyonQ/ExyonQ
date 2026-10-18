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
fn tier2b_regex_fastcgi_maps_suffix_route() {
    let (toml, report) = migrate_fixture("tier2b-regex-fastcgi.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(report.summary.regex_locations, 1);
    assert_eq!(cfg.routes.len(), 1, "NG-01 maps location ~ \\.php$");
    assert_eq!(
        cfg.routes[0].r#match.suffix.as_deref(),
        Some(".php"),
        "suffix preferred for \\.php$"
    );
    assert!(cfg.routes[0].fastcgi.is_some());
    assert!(has_entry(&report, "location", "regex") || has_entry(&report, "location", "ADR"));
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
fn tier2b_add_header_response_headers() {
    let (toml, report) = migrate_fixture("tier2b-add-header.conf");
    assert!(
        report.entries.iter().any(|e| {
            e.directive == "add_header" && e.status == NginxCompatStatus::Supported
        }),
        "{:?}",
        report.entries
    );
    assert!(
        toml.contains("[[route.response_headers]]"),
        "{toml}"
    );
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let root = cfg
        .routes
        .iter()
        .find(|r| r.r#match.path == "/" && r.root.is_some())
        .expect("root route");
    assert!(
        root.response_headers
            .iter()
            .any(|h| h.name == "X-Frame-Options" && !h.always)
    );
    assert!(
        root.response_headers
            .iter()
            .any(|h| h.name == "X-Content-Type-Options" && h.always)
    );
    let api = cfg
        .routes
        .iter()
        .find(|r| r.upstream.is_some())
        .expect("api");
    assert!(api
        .response_headers
        .iter()
        .any(|h| h.name == "X-Api-Tag"));
}

#[test]
fn tier2b_proxy_set_header_and_map() {
    let (toml, report) = migrate_fixture("tier2b-proxy-set-header.conf");
    assert!(
        report.entries.iter().any(|e| {
            e.directive == "proxy_set_header" && e.status == NginxCompatStatus::Supported
        }),
        "{:?}",
        report.entries
    );
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.directive == "map" && e.status == NginxCompatStatus::Supported),
        "{:?}",
        report.entries
    );
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert!(cfg.maps.contains_key("connection_upgrade"));
    let route = cfg
        .routes
        .iter()
        .find(|r| !r.proxy_set_headers.is_empty())
        .expect("proxy headers");
    assert!(route
        .proxy_set_headers
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case("X-Real-IP") && h.value == "$remote_addr"));
}

#[test]
fn tier2b_if_rejected() {
    let output = exyonq_compat_nginx::migrate_source(
        "tier2b-if-reject.conf",
        &read_fixture("tier2b-if-reject.conf"),
        &MigrateOptions::default(),
    );
    // May still produce config if other bits map; require Error on `if`.
    let report = match output {
        Ok(o) => o.report,
        Err(_) => {
            // Fail-closed refuse is also acceptable for `if`.
            return;
        }
    };
    assert!(
        report.entries.iter().any(|e| {
            e.directive == "if"
                && (e.status == NginxCompatStatus::Error
                    || e.status == NginxCompatStatus::Unsupported)
                && e.message.contains("EXY-NG-IF")
        }),
        "{:?}",
        report.entries
    );
}

#[test]
fn tier2b_error_page_maps() {
    let (toml, report) = migrate_fixture("tier2b-error-page.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let pages = &cfg.servers[0].error_pages;
    assert_eq!(pages.get("404").map(String::as_str), Some("/404.html"));
    assert_eq!(pages.get("500").map(String::as_str), Some("/50x.html"));
    assert_eq!(pages.get("502").map(String::as_str), Some("/50x.html"));
    assert!(
        report
            .entries
            .iter()
            .any(|e| { e.directive == "error_page" && e.status == NginxCompatStatus::Supported }),
        "{:?}",
        report.entries
    );
}

#[test]
fn tier2b_error_page_named_unsupported() {
    let (_, report) = migrate_fixture("tier2b-error-page-named.conf");
    assert!(
        report.entries.iter().any(|e| {
            e.directive == "error_page"
                && e.status == NginxCompatStatus::Unsupported
                && e.message.contains("@notfound")
        }),
        "{:?}",
        report.entries
    );
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
fn tier2b_try_files_spa_fallback_maps() {
    let (toml, report) = migrate_fixture("tier2b-try-files-spa.conf");
    let entry = report
        .entries
        .iter()
        .find(|e| e.directive == "try_files" && e.semantic_form.is_some())
        .expect("try_files");
    assert_eq!(entry.status, NginxCompatStatus::Supported);
    assert!(
        entry
            .migration_action
            .as_deref()
            .unwrap_or("")
            .contains("fallback_rewrite"),
        "{entry:?}"
    );
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = cfg
        .routes
        .iter()
        .find(|r| r.fallback_rewrite.is_some())
        .expect("fallback route");
    assert_eq!(route.fallback_rewrite.as_deref(), Some("/index.html"));
    assert_eq!(route.try_uri, Some(true));
    assert_eq!(route.try_uri_dir, Some(false));
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
fn tier2b_try_files_php_front_controller_maps() {
    let (toml, report) = migrate_fixture("tier2b-try-files-php.conf");
    let entry = report
        .entries
        .iter()
        .find(|e| e.directive == "try_files" && e.semantic_form.is_some())
        .expect("try_files");
    assert_eq!(entry.status, NginxCompatStatus::Supported, "{entry:?}");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = cfg
        .routes
        .iter()
        .find(|r| r.fallback_rewrite.as_deref() == Some("/index.php"))
        .expect("php fallback");
    assert_eq!(route.try_uri, Some(true));
    assert_eq!(route.try_uri_dir, Some(true));
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
fn tier2b_rewrite_capture_maps_rule() {
    let (toml, report) = migrate_fixture("tier2b-rewrite-capture.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(cfg.routes.len(), 1);
    let rw = cfg.routes[0]
        .rewrite
        .as_ref()
        .expect("NG-02 patterned rewrite");
    match rw {
        exyonq_config_ir::RewriteConfig::Rule(rule) => {
            assert!(rule.pattern.contains("old"));
            assert!(rule.replacement.contains("$1"));
            assert_eq!(rule.flag, exyonq_config_ir::RewriteFlagConfig::Permanent);
        }
        exyonq_config_ir::RewriteConfig::Literal(_) => panic!("expected rule form"),
    }
    assert!(has_entry(&report, "rewrite", "ADR-048") || has_entry(&report, "rewrite", "permanent"));
}

#[test]
fn tier2b_rewrite_internal_last_maps_rule() {
    let (toml, report) = migrate_fixture("tier2b-rewrite-internal.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(report.summary.internal_rewrites_unsupported, 0);
    let rw = cfg.routes[0].rewrite.as_ref().expect("rewrite last");
    match rw {
        exyonq_config_ir::RewriteConfig::Rule(rule) => {
            assert_eq!(rule.flag, exyonq_config_ir::RewriteFlagConfig::Last);
            assert_eq!(rule.replacement, "/index.php");
        }
        exyonq_config_ir::RewriteConfig::Literal(_) => panic!("expected rule form"),
    }
    let entry = report
        .entries
        .iter()
        .find(|e| e.directive == "rewrite")
        .expect("rewrite");
    assert_eq!(entry.status, NginxCompatStatus::Supported);
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
