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
//! EXY-03 — `return <status> "<body>"` → `route.respond`.
//!
//! The mapping is only honest if the three shapes are distinguished: a literal
//! body maps exactly, a body with nginx variables stays lossy (an IR body is
//! not interpolated), and a bodyless status without a construct stays refused.

use exyonq_compat_nginx::{migrate_source, MigrateOptions, NginxCompatStatus};
use exyonq_config_ir::AppConfig;

fn migrate(conf: &str) -> (String, exyonq_compat_nginx::CompatibilityReport) {
    let output =
        migrate_source("literal.conf", conf, &MigrateOptions::default()).expect("migrate config");
    (output.config, output.report)
}

fn return_entries(
    report: &exyonq_compat_nginx::CompatibilityReport,
) -> Vec<&exyonq_compat_nginx::CompatEntry> {
    report
        .entries
        .iter()
        .filter(|e| e.directive == "return")
        .collect()
}

#[test]
fn literal_body_maps_to_respond_and_survives_validation() {
    let conf = r#"
server {
    listen 80;
    server_name spa.example.com;
    root /var/www/spa/dist;
    index index.html;

    location = /healthz {
        return 200 "ok";
    }
}
"#;
    let (toml, report) = migrate(conf);

    assert!(
        toml.contains(r#"respond = { status = 200, body = "ok" }"#),
        "migrated TOML must carry the literal response:\n{toml}"
    );
    let cfg = AppConfig::parse_str(&toml).expect("the migrated config must validate");
    let route = cfg
        .routes
        .iter()
        .find(|route| route.r#match.path == "/healthz")
        .expect("/healthz route");
    let respond = route.respond.as_ref().expect("route.respond");
    assert_eq!(respond.status, 200);
    assert_eq!(respond.body, "ok");
    assert_eq!(respond.content_type, None);

    let entries = return_entries(&report);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, NginxCompatStatus::Supported);
    assert!(entries[0].message.contains("route.respond"));
}

#[test]
fn literal_body_with_variables_stays_lossy_and_is_not_mapped() {
    let conf = r#"
server {
    listen 80;
    root /var/www/app;
    location = /echo {
        return 200 "uri=$uri";
    }
}
"#;
    let (toml, report) = migrate(conf);

    assert!(
        !toml.contains("respond"),
        "an interpolated body must not become a literal response:\n{toml}"
    );
    let entries = return_entries(&report);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, NginxCompatStatus::Partial);
    assert!(
        entries[0].message.contains("variables"),
        "the loss must name the cause: {}",
        entries[0].message
    );
}

#[test]
fn bodyless_204_is_still_refused() {
    let conf = r#"
server {
    listen 80;
    root /var/www/app;
    location = /empty {
        return 204;
    }
}
"#;
    let (toml, report) = migrate(conf);

    assert!(!toml.contains("respond"), "{toml}");
    let entries = return_entries(&report);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, NginxCompatStatus::Partial);
}
