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
    let backend = cfg.upstreams.get("backend").expect("backend upstream");
    // NG-07: every `server` member maps to an endpoint, not just the first.
    assert_eq!(backend.endpoint_set.endpoints().len(), 2);
    let mut addrs: Vec<String> = backend
        .endpoint_set
        .endpoints()
        .iter()
        .map(|e| format!("{}:{}", e.address.as_host_str(), e.port))
        .collect();
    addrs.sort();
    assert_eq!(addrs, ["127.0.0.1:3000", "127.0.0.1:3001"]);
    assert!(cfg.routes[0].upstream.as_deref() == Some("backend"));
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
fn tier2a_strict_passes_on_multi_endpoint_upstream() {
    // NG-07: multi-endpoint upstreams fully map; strict mode no longer fails.
    let output = migrate_source(
        "tier2a-named-http-upstream.conf",
        &read_fixture("tier2a-named-http-upstream.conf"),
        &MigrateOptions::default(),
    )
    .expect("migrate");
    assert_eq!(output.exit_code(true), 0);
}

#[test]
fn ng07_upstream_lb_methods_map_to_selection_policy() {
    use exyonq_config_ir::EndpointSelectionPolicy;
    let src = r#"
upstream least { least_conn; server 10.0.0.1:8001; server 10.0.0.2:8001; }
upstream iph { ip_hash; server 10.0.0.1:8001; server 10.0.0.2:8001; }
upstream ch { hash $uri consistent; server 10.0.0.1:8001; server 10.0.0.2:8001; }
upstream st { sticky cookie srv_id; server 10.0.0.1:8001; server 10.0.0.2:8001; }
upstream plain { server 10.0.0.1:8001; }
upstream weighted { server 10.0.0.1:8001 weight=3; server 10.0.0.2:8001; }
server {
    listen 80;
    location / { proxy_pass http://least/; }
}
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    assert_eq!(
        cfg.upstreams["least"].selection_policy,
        EndpointSelectionPolicy::LeastConn
    );
    assert_eq!(
        cfg.upstreams["iph"].selection_policy,
        EndpointSelectionPolicy::SourceAffinity
    );
    assert_eq!(
        cfg.upstreams["ch"].selection_policy,
        EndpointSelectionPolicy::ConsistentHash
    );
    assert_eq!(
        cfg.upstreams["st"].selection_policy,
        EndpointSelectionPolicy::CookieAffinity
    );
    assert_eq!(cfg.upstreams["st"].sticky_cookie.as_deref(), Some("srv_id"));
    assert_eq!(
        cfg.upstreams["plain"].selection_policy,
        EndpointSelectionPolicy::WeightedRoundRobin
    );
    assert!(
        cfg.upstreams["weighted"]
            .endpoint_set
            .endpoints()
            .iter()
            .any(|e| e.weight == 3),
        "expected weight=3 flag mapped to an endpoint"
    );
    // Duplicate LB method → Error (nginx semantics).
    let dup = migrate_source(
        "dup.conf",
        "upstream d { ip_hash; least_conn; server 10.0.0.1:8001; } server { listen 80; }",
        &MigrateOptions::default(),
    )
    .expect("migrate");
    assert!(dup.summary.errors > 0, "expected duplicate-method error");
}

#[test]
fn ng07_hash_without_consistent_stays_partial() {
    let src = "upstream h { hash $arg_k; server 10.0.0.1:8001; } server { listen 80; }";
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    assert!(
        out.report.entries.iter().any(|e| e.status
            == exyonq_compat_nginx::NginxCompatStatus::Partial
            && e.directive == "hash"),
        "expected PARTIAL for non-consistent hash"
    );
}

#[test]
fn ng07_malformed_weight_fails_closed() {
    // F3 regression: `weight=abc` must Error, not silently become weight 1.
    let src = "upstream w { server 10.0.0.1:8001 weight=abc; server 10.0.0.2:8001; } server { listen 80; }";
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    assert!(out.summary.errors > 0, "malformed weight must error");
    assert!(
        out.report.entries.iter().any(|e| e.status
            == exyonq_compat_nginx::NginxCompatStatus::Error
            && e.message.contains("weight")),
        "expected malformed-weight error entry: {:?}",
        out.report.entries
    );
}

#[test]
fn ng07_down_backup_flags_map_endpoint_state() {
    use exyonq_config_ir::{AdminEndpointState, EndpointSelectionPolicy};
    // nginx: `down` = never selected; `backup` = spare band used on primary
    // failure → IR admin_state=disabled / priority=1.
    let src = r#"
upstream mix {
    server 10.0.0.1:8001;
    server 10.0.0.2:8001 down;
    server 10.0.0.3:8001 backup;
}
server { listen 80; location / { proxy_pass http://mix/; } }
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    let set = &cfg.upstreams["mix"].endpoint_set;
    let down = set
        .endpoints()
        .iter()
        .find(|e| e.admin_state == AdminEndpointState::Disabled)
        .expect("down member must map to disabled endpoint");
    assert_eq!(down.port, 8001);
    let backup = set
        .endpoints()
        .iter()
        .find(|e| e.priority == 1)
        .expect("backup member must map to the spare priority band");
    assert_eq!(backup.admin_state, AdminEndpointState::Enabled);
    // WRR policy preserved; bands keep the backup out of the preferred set.
    assert_eq!(
        cfg.upstreams["mix"].selection_policy,
        EndpointSelectionPolicy::WeightedRoundRobin
    );
}

#[test]
fn f3r20_proxy_redirect_maps_location_rewrite() {
    // F3-R20: `proxy_redirect` → route.location_rewrite (IR + report).
    let src = r#"
server {
    listen 80;
    location /app/ {
        proxy_pass http://10.0.0.1:8080/;
        proxy_redirect http://10.0.0.1:8080/ /app/;
    }
}
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    let route = &cfg.routes[0];
    assert_eq!(
        route.location_rewrite.len(),
        1,
        "{:?}",
        route.location_rewrite
    );
    assert_eq!(route.location_rewrite[0].from, "http://10.0.0.1:8080/");
    assert_eq!(route.location_rewrite[0].to, "/app/");
    assert!(
        out.report
            .entries
            .iter()
            .any(|e| e.directive == "proxy_redirect"
                && matches!(e.status, exyonq_compat_nginx::CompatStatus::Supported)),
        "proxy_redirect must report Supported"
    );
}

#[test]
fn f3r20_proxy_redirect_default_derives_from_proxy_pass() {
    let src = r#"
server {
    listen 80;
    location /app/ {
        proxy_pass http://10.0.0.1:8080/;
        proxy_redirect default;
    }
}
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    let rule = &cfg.routes[0].location_rewrite[0];
    assert_eq!(rule.from, "http://10.0.0.1:8080/");
    assert_eq!(rule.to, "/app/");
}

#[test]
fn f3r20_proxy_redirect_off_maps_to_no_rules() {
    let src = r#"
server {
    listen 80;
    location /app/ {
        proxy_pass http://10.0.0.1:8080/;
        proxy_redirect off;
    }
}
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    assert!(cfg.routes[0].location_rewrite.is_empty());
}

#[test]
fn f3r20_proxy_redirect_variables_degrade_partial() {
    let src = r#"
server {
    listen 80;
    location /app/ {
        proxy_pass http://10.0.0.1:8080/;
        proxy_redirect http://10.0.0.1:8080/$x /app/;
    }
}
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    assert!(cfg.routes[0].location_rewrite.is_empty());
    assert!(
        out.report
            .entries
            .iter()
            .any(|e| e.directive == "proxy_redirect"
                && matches!(e.status, exyonq_compat_nginx::CompatStatus::Partial)),
        "variables must degrade to Partial"
    );
}

#[test]
fn ng07_duplicate_identical_server_lines_fold_into_weight() {
    // nginx tolerates identical `server` entries; the IR set must not.
    let src = "upstream d { server 10.0.0.1:8001; server 10.0.0.1:8001; } server { listen 80; location / { proxy_pass http://d/; } }";
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    let eps = cfg.upstreams["d"].endpoint_set.endpoints();
    assert_eq!(eps.len(), 1, "duplicate members fold into weight");
    assert_eq!(eps[0].weight, 2);
}

#[test]
fn ng08_max_fails_fail_timeout_slow_start_map_endpoint() {
    // NG-08: nginx passive-health server params → IR endpoint fields.
    let src = r#"
upstream h {
    server 10.0.0.1:8001 max_fails=3 fail_timeout=5s slow_start=30s;
    server 10.0.0.2:8001 fail_timeout=500ms;
    server 10.0.0.3:8001;
}
server { listen 80; location / { proxy_pass http://h/; } }
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    let eps = cfg.upstreams["h"].endpoint_set.endpoints();
    let tuned = eps
        .iter()
        .find(|e| e.address.as_host_str() == "10.0.0.1")
        .expect("tuned endpoint");
    assert_eq!(tuned.max_fails, 3);
    assert_eq!(tuned.fail_timeout_ms, 5_000);
    assert_eq!(tuned.slow_start_ms, 30_000);
    let ms = eps
        .iter()
        .find(|e| e.address.as_host_str() == "10.0.0.2")
        .expect("ms endpoint");
    assert_eq!(ms.fail_timeout_ms, 500);
    let plain = eps
        .iter()
        .find(|e| e.address.as_host_str() == "10.0.0.3")
        .expect("default endpoint");
    assert_eq!(plain.max_fails, 1);
    assert_eq!(plain.fail_timeout_ms, 10_000);
    assert_eq!(plain.slow_start_ms, 0);
    // TOML round-trip keeps the tuned row (render must not drop the fields).
    assert!(out.config.contains("max_fails = 3"));
    assert!(out.config.contains("fail_timeout_ms = 5000"));
    assert!(out.config.contains("slow_start_ms = 30000"));
}

#[test]
fn ng08_malformed_max_fails_fails_closed() {
    let src = "upstream m { server 10.0.0.1:8001 max_fails=abc; server 10.0.0.2:8001; } server { listen 80; }";
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    assert!(out.summary.errors > 0, "malformed max_fails must error");
    assert!(
        out.report.entries.iter().any(|e| e.status
            == exyonq_compat_nginx::NginxCompatStatus::Error
            && e.message.contains("max_fails")),
        "expected malformed max_fails error: {:?}",
        out.report.entries
    );
}

#[test]
fn ng08_invalid_durations_fail_closed() {
    // `fail_timeout=0` (empty ban window) and a garbage `slow_start` must
    // Error, not silently map to defaults.
    for flag in [
        "fail_timeout=0",
        "fail_timeout=abc",
        "slow_start=",
        "slow_start=-5s",
    ] {
        let src = format!(
            "upstream d {{ server 10.0.0.1:8001 {flag}; server 10.0.0.2:8001; }} server {{ listen 80; }}"
        );
        let out = migrate_source("inline.conf", &src, &MigrateOptions::default()).expect("migrate");
        assert!(
            out.summary.errors > 0,
            "flag `{flag}` must error: {:?}",
            out.report.entries
        );
    }
}

#[test]
fn ng08_time_suffixes_parse() {
    // nginx `time` units — bare = seconds; ms/s/m/h/d accepted.
    let src = "upstream t { server 10.0.0.1:8001 fail_timeout=2m slow_start=1h; server 10.0.0.2:8001 fail_timeout=30; } server { listen 80; }";
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    let eps = cfg.upstreams["t"].endpoint_set.endpoints();
    let t1 = eps
        .iter()
        .find(|e| e.address.as_host_str() == "10.0.0.1")
        .expect("minutes endpoint");
    assert_eq!(t1.fail_timeout_ms, 120_000);
    assert_eq!(t1.slow_start_ms, 3_600_000);
    let t2 = eps
        .iter()
        .find(|e| e.address.as_host_str() == "10.0.0.2")
        .expect("bare-seconds endpoint");
    assert_eq!(t2.fail_timeout_ms, 30_000);
}

#[test]
fn ng08_bare_passive_flags_fail_closed() {
    // F8: `max_fails`/`fail_timeout`/`slow_start` without `=value` — nginx
    // rejects a missing parameter; must Error, not Partial or silent.
    for flag in ["max_fails", "fail_timeout", "slow_start"] {
        let src = format!(
            "upstream b {{ server 10.0.0.1:8001 {flag}; server 10.0.0.2:8001; }} server {{ listen 80; }}"
        );
        let out = migrate_source("inline.conf", &src, &MigrateOptions::default()).expect("migrate");
        assert!(
            out.summary.errors > 0,
            "bare flag `{flag}` must error: {:?}",
            out.report.entries
        );
    }
}

#[test]
fn ng08_duplicate_servers_with_different_passive_params_not_folded() {
    // F7: identical address/port rows fold into weight ONLY when every param
    // matches — differing passive-health params are distinct rows.
    let src = r#"
upstream d {
    server 10.0.0.1:8001 weight=2 max_fails=5;
    server 10.0.0.1:8001 weight=3 fail_timeout=1s;
    server 10.0.0.2:8001 weight=2;
    server 10.0.0.2:8001 weight=3;
}
server { listen 80; location / { proxy_pass http://d/; } }
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    let eps = cfg.upstreams["d"].endpoint_set.endpoints();
    // .1 rows differ in passive params → kept separate; .2 rows identical → folded.
    let one: Vec<_> = eps
        .iter()
        .filter(|e| e.address.as_host_str() == "10.0.0.1")
        .collect();
    assert_eq!(one.len(), 2, "differing passive params must not fold");
    let two: Vec<_> = eps
        .iter()
        .filter(|e| e.address.as_host_str() == "10.0.0.2")
        .collect();
    assert_eq!(two.len(), 1, "identical rows must fold");
    assert_eq!(two[0].weight, 5);
}

#[test]
fn ng09_resolver_valid_maps_to_dns_refresh() {
    // NG-09: `resolver <addrs> valid=T` inside upstream → dns_refresh_ms.
    let src = r#"
upstream h {
    resolver 10.0.0.53 valid=15s;
    server backend-a.internal:8001;
    server 10.0.0.2:8001;
}
server { listen 80; location / { proxy_pass http://h/; } }
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    assert_eq!(cfg.upstreams["h"].dns_refresh_ms, 15_000);
    assert!(out.config.contains("dns_refresh_ms = 15000"));
    assert!(out.report.entries.iter().any(|e| e.directive == "resolver"
        && e.status == exyonq_compat_nginx::NginxCompatStatus::Supported));
}

#[test]
fn ng09_resolver_without_valid_defaults_30s() {
    let src = "upstream h { resolver 10.0.0.53; server a.internal:8001; } server { listen 80; }";
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    assert_eq!(cfg.upstreams["h"].dns_refresh_ms, 30_000);
}

#[test]
fn ng09_malformed_resolver_valid_is_partial_not_silent() {
    let src =
        "upstream h { resolver 10.0.0.53 valid=garbage; server a.internal:8001; } server { listen 80; }";
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    assert_eq!(cfg.upstreams["h"].dns_refresh_ms, 0);
    assert!(out.report.entries.iter().any(|e| e.directive == "resolver"
        && e.status == exyonq_compat_nginx::NginxCompatStatus::Partial));
}

#[test]
fn ng09_over_limit_resolver_valid_degrades_to_partial() {
    // `valid=2d` exceeds MAX_DNS_REFRESH_MS (24h) — must degrade to Partial and
    // keep the upstream, not drop it with an IR error.
    let src =
        "upstream h { resolver 10.0.0.53 valid=2d; server a.internal:8001; } server { listen 80; }";
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    let up = cfg
        .upstreams
        .get("h")
        .expect("upstream must survive over-limit valid=");
    assert_eq!(up.dns_refresh_ms, 0);
    assert!(out.report.entries.iter().any(|e| e.directive == "resolver"
        && e.status == exyonq_compat_nginx::NginxCompatStatus::Partial));
}

#[test]
fn ng11_set_real_ip_from_maps_to_modules() {
    // NG-11: `set_real_ip_from` + `real_ip_header` at http level → modules.real_ip.
    let src = r#"
http {
    set_real_ip_from 10.0.0.0/8;
    set_real_ip_from 192.0.2.10;
    real_ip_header X-Real-IP;
    server { listen 80; location / { return 200 "ok"; } }
}
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    assert!(cfg.modules.real_ip.enabled);
    assert_eq!(
        cfg.modules.real_ip.trusted,
        vec!["10.0.0.0/8".to_string(), "192.0.2.10".to_string()]
    );
    assert_eq!(cfg.modules.real_ip.header, "X-Real-IP");
    assert!(out.config.contains("[modules.real_ip]"));
    assert!(out
        .report
        .entries
        .iter()
        .any(|e| e.directive == "set_real_ip_from"
            && e.status == exyonq_compat_nginx::NginxCompatStatus::Supported));
}

#[test]
fn ng11_server_level_set_real_ip_from() {
    let src = r#"
server {
    listen 80;
    set_real_ip_from 10.1.0.0/16;
    location / { return 200 "ok"; }
}
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    assert!(cfg.modules.real_ip.enabled);
    assert_eq!(cfg.modules.real_ip.trusted, vec!["10.1.0.0/16".to_string()]);
    // default header
    assert_eq!(cfg.modules.real_ip.header, "x-forwarded-for");
}

#[test]
fn ng11_invalid_cidr_is_partial() {
    let src = r#"
http {
    set_real_ip_from not-an-ip;
    server { listen 80; location / { return 200 "ok"; } }
}
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    assert!(out
        .report
        .entries
        .iter()
        .any(|e| e.directive == "set_real_ip_from"
            && e.status == exyonq_compat_nginx::NginxCompatStatus::Partial));
    let cfg = AppConfig::parse_str(&out.config).expect("parse toml");
    assert!(!cfg.modules.real_ip.enabled);
}

#[test]
fn ng11_location_scoped_real_ip_is_partial() {
    let src = r#"
server {
    listen 80;
    location / { set_real_ip_from 10.0.0.0/8; return 200 "ok"; }
}
"#;
    let out = migrate_source("inline.conf", src, &MigrateOptions::default()).expect("migrate");
    assert!(out
        .report
        .entries
        .iter()
        .any(|e| e.directive == "set_real_ip_from"
            && e.status == exyonq_compat_nginx::NginxCompatStatus::Partial));
}
