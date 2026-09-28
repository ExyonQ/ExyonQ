use exyonq_compat_nginx::{migrate_source, MigrateOptions, MigrateProfile};
use exyonq_config_ir::AppConfig;

fn static_opts() -> MigrateOptions {
    MigrateOptions {
        profile: MigrateProfile::StaticMvp,
        ..MigrateOptions::default()
    }
}

#[test]
fn static_mvp_accepts_http_wrapped_static() {
    let src = r#"
http {
    server {
        listen 127.0.0.1:18080;
        server_name static.example;
        root /tmp/exyonq-static-mvp;
        index index.html;
    }
}
"#;
    let out = migrate_source("wrap.conf", src, &static_opts()).expect("migrate");
    assert!(!out.config.is_empty());
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::StaticMvp, true),
        0
    );
    let cfg = AppConfig::parse_str(&out.config).expect("parse ir");
    assert_eq!(cfg.servers.len(), 1);
    assert_eq!(cfg.routes.len(), 1);
    assert_eq!(cfg.routes[0].r#match.path, "/");
}

#[test]
fn static_mvp_default_full_profile_unchanged_for_proxy() {
    let src = r#"
upstream app {
    server 127.0.0.1:3000;
}
server {
    listen 80;
    server_name app.example;
    location / {
        proxy_pass http://app;
    }
}
"#;
    let full = migrate_source("proxy.conf", src, &MigrateOptions::default()).expect("full");
    assert!(!full.config.is_empty(), "full profile must still map proxy");
    let mvp = migrate_source("proxy.conf", src, &static_opts()).expect("mvp");
    assert!(mvp.config.is_empty(), "static-mvp must refuse proxy");
    assert_eq!(
        mvp.exit_code_for_profile(MigrateProfile::StaticMvp, false),
        1
    );
}

#[test]
fn static_mvp_refuses_location_other_than_slash() {
    let src = r#"
server {
    listen 80;
    root /srv/www;
    location /api {
        root /srv/api;
    }
}
"#;
    let out = migrate_source("loc.conf", src, &static_opts()).expect("migrate");
    assert!(out.config.is_empty());
    assert!(out.report.has_errors());
}
