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
use exyonq_compat_nginx::{migrate_source, MigrateOptions, MigrateProfile};

fn opts() -> MigrateOptions {
    MigrateOptions {
        profile: MigrateProfile::FastcgiPhpMvp,
        ..MigrateOptions::default()
    }
}

#[test]
fn accept_tcp_php_emits_fcgi_pool_ir() {
    let src = r#"
server {
    listen 127.0.0.1:18080;
    server_name php-mvp.local;
    root /var/www/html;
    location /index.php {
        fastcgi_pass 127.0.0.1:19000;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
    }
}
"#;
    let out = migrate_source("fcgi-accept.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        0
    );
    assert!(!out.config.trim().is_empty());
    assert!(out.config.contains("[[fcgi_pool]]"));
    assert!(out.config.contains("transport = \"tcp\""));
    assert!(out.config.contains("address = \"127.0.0.1:19000\""));
    assert!(out.config.contains("document_root = \"/var/www/html\""));
    assert!(out.config.contains("path = \"/index.php\""));
    assert!(out.config.contains("fastcgi ="));
    assert!(!out.config.contains("index ="));
}

#[test]
fn reject_unix_socket() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    location /index.php {
        fastcgi_pass unix:/run/php/php-fpm.sock;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_include_fastcgi_params() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    location /index.php {
        include fastcgi_params;
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_variable_fastcgi_pass() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    location /index.php {
        fastcgi_pass $backend;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_non_script_filename_param() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    location /index.php {
        fastcgi_pass 127.0.0.1:9000;
        fastcgi_param QUERY_STRING $query_string;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_try_files() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    location /index.php {
        try_files $uri =404;
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_unknown_directive() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    gzip on;
    location /index.php {
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_regex_location() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    location ~ \.php$ {
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_index_and_fastcgi_index() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    index index.php;
    location /index.php {
        fastcgi_index index.php;
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reject_hostname_fastcgi_pass() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    location /index.php {
        fastcgi_pass php-fpm:9000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn full_importer_still_blocks_tcp() {
    let src = r#"
server {
    listen 80;
    root /var/www;
    location /index.php {
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
    let out = migrate_source(
        "full.conf",
        src,
        &MigrateOptions {
            profile: MigrateProfile::Full,
            ..MigrateOptions::default()
        },
    )
    .expect("migrate");
    assert!(!out.config.contains("transport = \"tcp\""));
    assert!(!out.config.contains("[[fcgi_pool]]"));
    assert!(!out.config.contains("fastcgi ="));
}

#[test]
fn refuse_localhost_listen_false_success() {
    let src = r#"
server {
    listen localhost:8080;
    root /var/www;
    location /index.php {
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
    let out = migrate_source("rej.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::FastcgiPhpMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}

#[test]
fn reverse_proxy_mvp_still_refuses_fastcgi() {
    let src = r#"
server {
    listen 80;
    location / {
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
    let out = migrate_source(
        "proxy.conf",
        src,
        &MigrateOptions {
            profile: MigrateProfile::ReverseProxyMvp,
            ..MigrateOptions::default()
        },
    )
    .expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::ReverseProxyMvp, false),
        1
    );
    assert!(out.config.trim().is_empty());
}
