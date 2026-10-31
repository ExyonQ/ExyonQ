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
        report
            .entries
            .iter()
            .any(|e| { e.directive == "add_header" && e.status == NginxCompatStatus::Supported }),
        "{:?}",
        report.entries
    );
    assert!(toml.contains("[[route.response_headers]]"), "{toml}");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let root = cfg
        .routes
        .iter()
        .find(|r| r.r#match.path == "/" && r.root.is_some())
        .expect("root route");
    assert!(root
        .response_headers
        .iter()
        .any(|h| h.name == "X-Frame-Options" && !h.always));
    assert!(root
        .response_headers
        .iter()
        .any(|h| h.name == "X-Content-Type-Options" && h.always));
    let api = cfg
        .routes
        .iter()
        .find(|r| r.upstream.is_some())
        .expect("api");
    assert!(api.response_headers.iter().any(|h| h.name == "X-Api-Tag"));
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

#[test]
fn tier2b_expires_maps_and_inherits_server_scope() {
    let (toml, report) = migrate_fixture("tier2b-expires.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = |path: &str| {
        cfg.routes
            .iter()
            .find(|r| r.r#match.path == path)
            .unwrap_or_else(|| panic!("route {path}"))
    };
    let max_age = |path: &str, mime: &str| {
        route(path)
            .cache_headers
            .as_ref()
            .expect("cache_headers")
            .lookup(mime)
            .expect("mime")
            .max_age
    };

    assert_eq!(max_age("/static/", "text/html"), 2_592_000, "server 30d");
    assert_eq!(max_age("/assets/", "text/css"), 3_600, "location 1h wins");
    assert_eq!(max_age("/api/", "application/json"), 315_360_000, "max");

    assert!(route("/static/")
        .response_headers
        .iter()
        .any(|h| h.name == "X-Server"));
    assert!(route("/assets/")
        .response_headers
        .iter()
        .any(|h| h.name == "X-Server"));
    assert!(route("/api/")
        .response_headers
        .iter()
        .any(|h| h.name == "X-Api"));
    assert!(
        !route("/api/")
            .response_headers
            .iter()
            .any(|h| h.name == "X-Server"),
        "location add_header replaces the inherited set (nginx)"
    );
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "expires" && e.status == NginxCompatStatus::Partial));
}

#[test]
fn tier2b_location_off_and_denied_header_override_inheritance() {
    let (toml, _) = migrate_fixture("tier2b-inheritance-overrides.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = |path: &str| {
        cfg.routes
            .iter()
            .find(|r| r.r#match.path == path)
            .unwrap_or_else(|| panic!("route {path}"))
    };

    assert_eq!(
        route("/inherit/")
            .cache_headers
            .as_ref()
            .expect("inherited expires")
            .lookup("text/html")
            .expect("mime")
            .max_age,
        2_592_000
    );
    assert!(route("/inherit/")
        .response_headers
        .iter()
        .any(|h| h.name == "X-Server"));

    assert!(
        route("/off/").cache_headers.is_none(),
        "location `expires off` must not inherit the server value"
    );
    assert!(
        !route("/denied/")
            .response_headers
            .iter()
            .any(|h| h.name == "X-Server"),
        "a location add_header (even rejected) replaces the inherited set"
    );
}

#[test]
fn ng18_gunzip_http_server_location_and_inheritance() {
    let (toml, report) = migrate_fixture("ng18/gunzip/nginx-migrate.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert!(cfg.modules.gunzip.enabled, "http gunzip on");
    assert_eq!(cfg.modules.gunzip.buffer_bytes, 32 * 1024);
    let route = |name: &str| {
        cfg.routes
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("route {name}"))
    };
    // http on; location /off/ overrides off; location / inherits (None).
    assert_eq!(route("srv1_loc1").gunzip, None);
    assert_eq!(route("srv1_loc2").gunzip, Some(false));
    // server off; location /on/ overrides on; location / inherits off.
    assert_eq!(route("srv2_loc1").gunzip, Some(false));
    assert_eq!(route("srv2_loc2").gunzip, Some(true));
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "gunzip" && e.status == NginxCompatStatus::Supported));
}

#[test]
fn ng18_sub_filter_http_location_inheritance() {
    let (toml, report) = migrate_fixture("ng18/sub_filter/nginx-migrate.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let root = cfg
        .modules
        .sub_filter
        .routes
        .get("srv1_loc1")
        .expect("root route sub_filter");
    assert_eq!(root.rules[0].pattern, "http://");
    assert_eq!(root.rules[0].replacement, "https://");
    assert!(!root.once, "http sub_filter_once off inherited");
    assert_eq!(root.types, vec!["text/html".to_string()]);
    let off = cfg
        .modules
        .sub_filter
        .routes
        .get("srv1_loc2")
        .expect("off route sub_filter");
    assert_eq!(off.rules[0].pattern, "foo");
    assert!(!off.once, "scalar flags inherit from http");
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "sub_filter" && e.status == NginxCompatStatus::Supported));
}

#[test]
fn ng18_mirror_http_location_inheritance() {
    let (toml, report) = migrate_fixture("ng18/mirror/nginx-migrate.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let root = cfg
        .modules
        .mirror
        .routes
        .get("srv1_loc1")
        .expect("root route mirror");
    assert_eq!(root.destinations, vec!["/mirror".to_string()]);
    assert!(
        root.request_body,
        "nginx 1.27 defaults mirror_request_body to on"
    );
    let off = cfg
        .modules
        .mirror
        .routes
        .get("srv1_loc2")
        .expect("off route mirror");
    assert_eq!(off.destinations, vec!["/off-mirror".to_string()]);
    assert!(!off.request_body, "explicit mirror_request_body off");
    assert!(
        !cfg.modules.mirror.routes.contains_key("srv1_loc3"),
        "`mirror off` must not inherit the http-level destinations"
    );
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "mirror" && e.status == NginxCompatStatus::Supported));
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.directive == "mirror_request_body"
                && e.status == NginxCompatStatus::Supported),
        "mirror_request_body on must be Supported, not Unsupported"
    );
}

#[test]
fn ng18_slice_http_location_inheritance() {
    let (toml, report) = migrate_fixture("ng18/slice/nginx-migrate.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(
        cfg.modules
            .slice
            .routes
            .get("srv1_loc1")
            .expect("root slice")
            .size,
        1024 * 1024
    );
    assert_eq!(
        cfg.modules
            .slice
            .routes
            .get("srv1_loc2")
            .expect("off slice")
            .size,
        2 * 1024 * 1024
    );
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "slice" && e.status == NginxCompatStatus::Supported));
}

#[test]
fn ng07_proxy_redirect_scheme_host_port_vars() {
    let (toml, report) = migrate_fixture("ng18/proxy-redirect-vars.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(
        cfg.routes[0].location_rewrite[0].to,
        "$scheme://$host:$server_port/"
    );
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "proxy_redirect" && e.status == NginxCompatStatus::Supported));
}

#[test]
fn ng07_proxy_redirect_regex_captures() {
    let (toml, report) = migrate_fixture("ng18/proxy-redirect-regex.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert_eq!(
        cfg.routes[0].location_rewrite[0].from,
        "~^http://127.0.0.1:9000/(.*)$"
    );
    assert_eq!(cfg.routes[0].location_rewrite[0].to, "/$1");
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "proxy_redirect" && e.status == NginxCompatStatus::Supported));
}

#[test]
fn ng05_auth_basic_maps_realm_and_file() {
    let (toml, report) = migrate_fixture("ng18/auth-basic.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let auth = cfg
        .modules
        .auth
        .routes
        .get("srv1_loc1")
        .expect("route auth");
    assert_eq!(auth.realm, "Restricted");
    assert_eq!(auth.user_file, "/etc/nginx/.htpasswd");
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "auth_basic" && e.status == NginxCompatStatus::Supported));
}

#[test]
fn ng05_limit_req_zone_importer() {
    let (toml, report) = migrate_fixture("ng18/limit-req.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let zone = cfg.modules.ratelimit.zones.get("one").expect("zone");
    assert_eq!(zone.key, "ip");
    assert_eq!(zone.requests_per_second, 1);
    assert_eq!(zone.burst, 5);
    assert_eq!(
        cfg.modules
            .ratelimit
            .routes
            .get("srv1_loc1")
            .map(String::as_str),
        Some("one")
    );
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "limit_req_zone" && e.status == NginxCompatStatus::Supported));
}

#[test]
fn tier2b_body_transforms_are_imported_not_dropped() {
    let (toml, report) = migrate_fixture("tier2b-body-transforms.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    for directive in [
        "sub_filter",
        "sub_filter_types",
        "sub_filter_once",
        "slice",
        "mirror",
    ] {
        assert!(
            report
                .entries
                .iter()
                .any(|e| { e.directive == directive && e.status == NginxCompatStatus::Supported }),
            "{directive} must be reported Supported (NG-18)"
        );
    }
    assert!(report
        .entries
        .iter()
        .any(|e| e.directive == "gunzip" && e.status == NginxCompatStatus::Supported));
    assert!(!cfg.modules.sub_filter.routes.is_empty());
    assert!(!cfg.modules.slice.routes.is_empty());
    let mirror = cfg
        .modules
        .mirror
        .routes
        .get("srv1_loc1")
        .expect("mirror route");
    assert_eq!(mirror.destinations, vec!["/mirror".to_string()]);
    assert!(mirror.request_body, "mirror_request_body defaults to on");
}

#[test]
fn tier2b_expires_epoch_and_modified_are_reported_not_dropped() {
    let (toml, report) = migrate_fixture("tier2b-expires-unsupported.conf");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert!(cfg.routes.iter().all(|r| r.cache_headers.is_none()));
    assert_eq!(
        report
            .entries
            .iter()
            .filter(|e| e.directive == "expires" && e.status == NginxCompatStatus::Unsupported)
            .count(),
        2
    );
}
