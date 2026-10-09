use exyonq_compat_nginx::{migrate_source, MigrateOptions, MigrateProfile};
use exyonq_config_ir::{AppConfig, HtaccessMode};

fn opts() -> MigrateOptions {
    MigrateOptions {
        profile: MigrateProfile::Wordpress,
        ..MigrateOptions::default()
    }
}

const STOCK: &str = r#"
server {
    listen 80;
    listen [::]:80;
    server_name example.com;
    root /var/www/html;
    index index.php;

    location / {
        try_files $uri $uri/ /index.php?$args;
    }

    location ~ \.php$ {
        include snippets/fastcgi-php.conf;
        fastcgi_pass unix:/run/php/php-fpm.sock;
    }

    location /wp-content/ {
        try_files $uri $uri/ =404;
    }
}
"#;

#[test]
fn stock_wordpress_emits_product_toml() {
    let out = migrate_source("wordpress.conf", STOCK, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::Wordpress, false),
        0,
        "{:?}",
        out.report.entries
    );
    let cfg = AppConfig::parse_str(&out.config).expect("emitted toml must load");
    let php = cfg
        .routes
        .iter()
        .find(|route| route.name == "wordpress")
        .expect("wordpress route");
    assert_eq!(php.fastcgi.as_deref(), Some("php"));
    assert_eq!(php.htaccess, HtaccessMode::Overlay);
    assert!(php.root.is_none());
    assert!(php.rewrite.is_none());
    let includes = cfg
        .routes
        .iter()
        .find(|route| route.name == "wp-includes")
        .expect("wp-includes");
    assert_eq!(
        includes.root.as_deref().map(|path| path.to_string_lossy().into_owned()),
        Some("/var/www/html/wp-includes".to_string())
    );
    let pool = cfg.pools_fcgi.get("php").expect("php pool");
    assert_eq!(pool.address, "/run/php/php-fpm.sock");
    assert!(out.config.contains("transport = \"unix\""));
    assert!(!out.config.contains("front_controller"));
}

#[test]
fn upstream_unix_socket_is_accepted() {
    let src = r#"
upstream php {
    server unix:/run/php/php8.3-fpm.sock;
}
server {
    listen 127.0.0.1:8080;
    root /srv/www;
    location / {
        try_files $uri $uri/ /index.php$is_args$args;
    }
    location ~ \.php$ {
        fastcgi_pass php;
    }
}
"#;
    let out = migrate_source("upstream.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::Wordpress, false),
        0
    );
    assert!(out.config.contains("address = \"/run/php/php8.3-fpm.sock\""));
    assert!(out.config.contains("listen = \"127.0.0.1:8080\""));
}

#[test]
fn rewrite_is_refused() {
    let src = r#"
server {
    listen 80;
    root /var/www/html;
    location / {
        rewrite ^ /index.php last;
        try_files $uri $uri/ /index.php?$args;
    }
    location ~ \.php$ {
        fastcgi_pass unix:/run/php/php-fpm.sock;
    }
}
"#;
    let out = migrate_source("rewrite.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::Wordpress, false),
        1
    );
    assert!(out.config.trim().is_empty());
    assert!(out
        .report
        .entries
        .iter()
        .any(|entry| entry.directive == "rewrite"));
}

#[test]
fn tcp_fastcgi_is_refused() {
    let src = r#"
server {
    listen 80;
    root /var/www/html;
    location / {
        try_files $uri $uri/ /index.php?$args;
    }
    location ~ \.php$ {
        fastcgi_pass 127.0.0.1:9000;
    }
}
"#;
    let out = migrate_source("tcp.conf", src, &opts()).expect("migrate");
    assert_eq!(
        out.exit_code_for_profile(MigrateProfile::Wordpress, false),
        1
    );
    assert!(out.config.trim().is_empty());
}
