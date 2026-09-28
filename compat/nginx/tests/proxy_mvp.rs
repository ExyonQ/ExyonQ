use exyonq_compat_nginx::{migrate_source, MigrateOptions, MigrateProfile};

fn opts() -> MigrateOptions {
    MigrateOptions {
        profile: MigrateProfile::ReverseProxyMvp,
        ..MigrateOptions::default()
    }
}

#[test]
fn accept_prefix_proxy_emits_upstream_ir() {
    let src = r#"
server {
    listen 127.0.0.1:18080;
    server_name proxy-mvp.local;
    location /api/ {
        proxy_pass http://127.0.0.1:19000/;
    }
}
"#;
    let out = migrate_source("proxy-accept.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
        0
    );
    assert!(!out.config.trim().is_empty());
    assert!(out.config.contains("[[upstream]]"));
    assert!(out.config.contains("target = \"http://127.0.0.1:19000\""));
    assert!(out.config.contains("path = \"/api/\"") || out.config.contains("path = \"/api\""));
}

#[test]
fn reject_https_proxy_pass() {
    let src = r#"
server {
    listen 80;
    location / {
        proxy_pass https://127.0.0.1:3000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_upstream_block() {
    let src = r#"
upstream backend { server 127.0.0.1:3000; }
server {
    listen 80;
    location / {
        proxy_pass http://backend;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_proxy_set_header() {
    let src = r#"
server {
    listen 80;
    location / {
        proxy_set_header Host $host;
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_include() {
    let src = r#"
server {
    listen 80;
    include mime.types;
    location / {
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_variable_in_proxy_pass() {
    let src = r#"
server {
    listen 80;
    location / {
        proxy_pass http://$host:3000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_unknown_directive() {
    let src = r#"
server {
    listen 80;
    gzip on;
    location / {
        proxy_pass http://127.0.0.1:3000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_uri_bearing_proxy_pass() {
    let src = r#"
server {
    listen 80;
    location / {
        proxy_pass http://127.0.0.1:3000/app;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}
