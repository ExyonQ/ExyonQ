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
//! EXY-03: NGINX `root`/`alias` import must preserve the effective file path
//! NGINX would serve — `root` → `root_mode = "full-uri"` (root + URI), `alias`
//! → default alias mode (root + URI − prefix). Unrepresentable cases are
//! reported as errors, never silently transformed.

use exyonq_compat_nginx::{migrate_source, CompatibilityReport, MigrateOptions, NginxCompatStatus};
use exyonq_config_ir::{AppConfig, RootMode};

fn import(src: &str) -> (String, CompatibilityReport) {
    let out = migrate_source("test.conf", src, &MigrateOptions::default()).expect("migrate");
    (out.config, out.report)
}

fn has_error(report: &CompatibilityReport) -> bool {
    report
        .entries
        .iter()
        .any(|e| e.status == NginxCompatStatus::Error)
}

#[test]
fn root_in_location_maps_to_nginx_mode() {
    let src = r#"
server {
    listen 80;
    location /assets {
        root /var/www;
    }
}
"#;
    let (toml, _) = import(src);
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = cfg
        .routes
        .iter()
        .find(|r| r.r#match.path == "/assets")
        .expect("route");
    assert_eq!(route.root_mode, RootMode::FullUri);
    assert_eq!(
        route.root.as_ref().map(|p| p.display().to_string()),
        Some("/var/www".into())
    );
}

#[test]
fn alias_in_location_maps_to_alias_mode() {
    let src = r#"
server {
    listen 80;
    location /assets {
        alias /var/www/files/;
    }
}
"#;
    let (toml, _) = import(src);
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = cfg
        .routes
        .iter()
        .find(|r| r.r#match.path == "/assets")
        .expect("route");
    assert_eq!(route.root_mode, RootMode::Remainder);
    assert_eq!(
        route.root.as_ref().map(|p| p.display().to_string()),
        Some("/var/www/files/".into())
    );
}

#[test]
fn nested_location_preserves_prefix_and_mode() {
    let src = r#"
server {
    listen 80;
    location /a/b {
        root /var;
    }
}
"#;
    let (toml, _) = import(src);
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = cfg
        .routes
        .iter()
        .find(|r| r.r#match.path == "/a/b")
        .expect("route");
    // NGINX serves /var/a/b/x — preserved via root_mode, not by
    // rewriting the root or the prefix.
    assert_eq!(route.root_mode, RootMode::FullUri);
    assert_eq!(
        route.root.as_ref().map(|p| p.display().to_string()),
        Some("/var".into())
    );
}

#[test]
fn inherited_server_root_keeps_nginx_semantics() {
    let src = r#"
server {
    listen 80;
    root /srv/main;
    location /app {
        index index.html;
    }
}
"#;
    let (toml, _) = import(src);
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = cfg
        .routes
        .iter()
        .find(|r| r.r#match.path == "/app")
        .expect("route");
    assert_eq!(route.root_mode, RootMode::FullUri);
    assert_eq!(
        route.root.as_ref().map(|p| p.display().to_string()),
        Some("/srv/main".into())
    );
}

#[test]
fn root_and_alias_in_same_location_is_an_error() {
    let src = r#"
server {
    listen 80;
    location /assets {
        root /a;
        alias /b;
    }
}
"#;
    let (toml, report) = import(src);
    assert!(has_error(&report), "expected report error: {report:?}");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert!(
        cfg.routes.iter().all(|r| r.root.is_none()),
        "conflicting location must not emit a rooted route"
    );
}

#[test]
fn alias_at_server_level_is_an_error() {
    let src = r#"
server {
    listen 80;
    alias /var/www;
    location /app {
        root /srv;
    }
}
"#;
    let (toml, report) = import(src);
    assert!(has_error(&report), "expected report error: {report:?}");
    assert!(
        report
            .entries
            .iter()
            .all(|e| !(e.directive == "alias" && e.status == NginxCompatStatus::Supported)),
        "unsupported alias must never report Supported: {report:?}"
    );
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    assert!(
        cfg.routes
            .iter()
            .all(|r| r.root.is_none() || r.r#match.path != "/alias-root"),
        "rejected alias must not produce a usable route"
    );
}

#[test]
fn alias_under_exact_location_maps_to_exact_route() {
    // EXY-03: `location =` + file `alias` is faithful — match.exact + the alias
    // path as a verbatim root. `/exactx` must not match.
    let src = r#"
server {
    listen 80;
    location = /exact {
        alias /var/www/exact.html;
    }
}
"#;
    let (toml, report) = import(src);
    assert!(
        !has_error(&report),
        "exact alias must not error: {report:?}"
    );
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = cfg
        .routes
        .iter()
        .find(|r| r.r#match.path == "/exact")
        .expect("route");
    assert!(route.r#match.exact, "exact location must emit match.exact");
    assert_eq!(route.root_mode, RootMode::Remainder);
    assert_eq!(
        route.root.as_ref().map(|p| p.display().to_string()),
        Some("/var/www/exact.html".into())
    );
}

#[test]
fn legacy_toml_without_root_mode_defaults_to_remainder() {
    // Backward compatibility: a native config written before `root_mode`
    // existed must parse with historical alias-style resolution.
    let toml = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["legacy"]

[[route]]
name = "legacy"
match = { path = "/assets" }
root = "/var/www/files"
"#;
    let cfg = AppConfig::parse_str(toml).expect("parse");
    let route = &cfg.routes[0];
    assert_eq!(route.root_mode, RootMode::Remainder);
}

#[test]
fn imported_prefix_locations_use_literal_match_and_dir_redirect() {
    let src = r#"
server {
    listen 80;
    location /assets {
        root /var/www;
    }
    location /files {
        alias /var/www/files/;
    }
}
"#;
    let (toml, _) = import(src);
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    for route in cfg.routes.iter().filter(|r| r.root.is_some()) {
        assert!(
            route.r#match.literal,
            "imported NGINX prefix location must use literal matching: {}",
            route.name
        );
        assert!(
            route.dir_redirect,
            "imported static route must emit NGINX dir redirect: {}",
            route.name
        );
    }
}

#[test]
fn native_routes_keep_segment_boundary_matching() {
    // A native (non-imported) route built without `literal` keeps the
    // historical segment-boundary semantics — `/api` must not match `/api2`.
    let toml = r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]

[[route]]
name = "api"
match = { path = "/api" }
root = "/var/www"
"#;
    let cfg = AppConfig::parse_str(toml).expect("parse");
    assert!(!cfg.routes[0].r#match.literal);
    assert!(!cfg.routes[0].dir_redirect);
}

#[test]
fn alias_with_fastcgi_maps_to_route_script_root() {
    // EXY-03: `alias` + `fastcgi_pass` — faithful via route.fastcgi_script_root
    // (alias + URI−prefix), no longer a fail-closed error.
    let src = r#"
server {
    listen 80;
    location /php {
        alias /srv/php/;
        fastcgi_pass unix:/run/php-fpm.sock;
    }
}
"#;
    let (toml, report) = import(src);
    assert!(!has_error(&report), "alias+fcgi must not error: {report:?}");
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let route = cfg
        .routes
        .iter()
        .find(|r| r.r#match.path == "/php")
        .expect("route");
    assert!(route.fastcgi.is_some(), "fcgi route expected");
    assert_eq!(
        route
            .fastcgi_script_root
            .as_ref()
            .map(|p| p.display().to_string()),
        Some("/srv/php/".into())
    );
}

#[test]
fn redirect_directives_import_per_nginx() {
    // EXY-03: absolute_redirect/port_in_redirect/server_name_in_redirect —
    // location scope overrides server scope; defaults on/on/host-echo.
    let src = r#"
server {
    listen 80;
    server_name nameinred.example;
    server_name_in_redirect on;
    root /var/www;
    location /rel {
        root /var/www/rel;
        absolute_redirect off;
    }
    location /noport {
        root /var/www/np;
        port_in_redirect off;
    }
    location /defaults {
        root /var/www/d;
    }
}
"#;
    let (toml, _report) = import(src);
    let cfg = AppConfig::parse_str(&toml).expect("parse");
    let find = |p: &str| {
        cfg.routes
            .iter()
            .find(|r| r.r#match.path == p)
            .expect("route")
    };
    let rel = find("/rel");
    assert!(!rel.dir_redirect_absolute, "absolute_redirect off");
    assert!(rel.dir_redirect_port);
    assert_eq!(rel.dir_redirect_host.as_deref(), Some("nameinred.example"));
    let noport = find("/noport");
    assert!(noport.dir_redirect_absolute);
    assert!(!noport.dir_redirect_port, "port_in_redirect off");
    let def = find("/defaults");
    assert!(def.dir_redirect_absolute);
    assert!(def.dir_redirect_port);
    assert_eq!(def.dir_redirect_host.as_deref(), Some("nameinred.example"));
}
