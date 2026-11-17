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
//! NGINX `ssl_certificate` + `ssl_certificate_key` → `[server.tls]`.
//!
//! These directives used to be reported as lossy (TLS silently off after
//! migration). The pair now lands in the IR and the `ssl` listen flag is served
//! by it, so the only remaining lossy case is a lone half of the pair.

use exyonq_compat_nginx::{migrate_source, MigrateOptions, NginxCompatStatus};
use exyonq_config_ir::AppConfig;

const TLS_SERVER: &str = r#"
server {
    listen 443 ssl;
    server_name app.example.com;
    root /var/www/app/public;
    ssl_certificate /etc/ssl/app.crt;
    ssl_certificate_key /etc/ssl/app.key;
    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }
}
"#;

fn migrate(conf: &str) -> (String, exyonq_compat_nginx::CompatibilityReport) {
    let output =
        migrate_source("tls.conf", conf, &MigrateOptions::default()).expect("migrate TLS config");
    (output.config, output.report)
}

fn entry<'a>(
    report: &'a exyonq_compat_nginx::CompatibilityReport,
    directive: &str,
) -> Vec<&'a exyonq_compat_nginx::CompatEntry> {
    report
        .entries
        .iter()
        .filter(|e| e.directive == directive)
        .collect()
}

#[test]
fn ssl_pair_maps_to_server_tls_and_is_not_lossy() {
    let (toml, report) = migrate(TLS_SERVER);

    assert!(
        toml.contains("tls = { cert = \"/etc/ssl/app.crt\", key = \"/etc/ssl/app.key\" }"),
        "migrated TOML must carry the TLS block:\n{toml}"
    );

    let cfg = AppConfig::parse_str(&toml).expect("parse the migrated config");
    let tls = cfg.servers[0].tls.as_ref().expect("[server.tls] emitted");
    assert_eq!(tls.cert.to_string_lossy(), "/etc/ssl/app.crt");
    assert_eq!(tls.key.to_string_lossy(), "/etc/ssl/app.key");

    let cert = entry(&report, "ssl_certificate");
    assert_eq!(cert.len(), 1);
    assert_eq!(cert[0].status, NginxCompatStatus::Supported);
    let key = entry(&report, "ssl_certificate_key");
    assert_eq!(key.len(), 1);
    assert_eq!(key[0].status, NginxCompatStatus::Supported);
}

#[test]
fn ssl_listen_flag_is_served_by_the_tls_block() {
    let (_toml, report) = migrate(TLS_SERVER);

    let ssl_flag: Vec<_> = entry(&report, "listen")
        .into_iter()
        .filter(|e| e.message.contains("`ssl`"))
        .collect();
    assert_eq!(
        ssl_flag.len(),
        1,
        "the `ssl` flag must be reported once: {:#?}",
        entry(&report, "listen")
    );
    assert_eq!(ssl_flag[0].status, NginxCompatStatus::Supported);
    assert!(ssl_flag[0].message.contains("[server.tls]"));
}

#[test]
fn lone_half_of_the_pair_keeps_tls_off_and_is_reported_lossy() {
    let conf = r#"
server {
    listen 443 ssl;
    server_name app.example.com;
    root /var/www/app;
    ssl_certificate /etc/ssl/app.crt;
}
"#;
    let (toml, report) = migrate(conf);

    assert!(
        !toml.contains("tls ="),
        "a certificate without its key must not emit a half TLS block:\n{toml}"
    );
    let cfg = AppConfig::parse_str(&toml).expect("parse the migrated config");
    assert!(cfg.servers[0].tls.is_none());

    let cert = entry(&report, "ssl_certificate");
    assert_eq!(cert.len(), 1);
    assert_eq!(cert[0].status, NginxCompatStatus::Partial);
    assert!(cert[0].message.contains("ssl_certificate_key"));
}
