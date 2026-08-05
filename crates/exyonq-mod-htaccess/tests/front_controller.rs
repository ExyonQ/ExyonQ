//! Front-controller compiler recognition tests (Plan 11 tranche C).

use exyonq_mod_htaccess::compile_from_discovered;
use exyonq_mod_htaccess::DiscoveredFile;
use exyonq_module_api::{lookup_overlay, OverlayLookupResult};
use std::path::{Path, PathBuf};

fn compile_htaccess(content: &str) -> exyonq_mod_htaccess::CompileOutput {
    let files = vec![DiscoveredFile {
        absolute: PathBuf::from("/virtual/.htaccess"),
        relative_directory: "/".into(),
        content: content.into(),
    }];
    compile_from_discovered("site", Path::new("/virtual"), 1, files).expect("compile")
}

fn root_fc(out: &exyonq_mod_htaccess::CompileOutput) -> &exyonq_module_api::OverlayEntry {
    out.overlay
        .entries
        .iter()
        .find(|e| e.directory.0 == "/")
        .expect("root entry")
}

#[test]
fn canonical_front_controller_pattern_recognized() {
    let htaccess = r"
RewriteEngine On
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
";
    let out = compile_htaccess(htaccess);
    let entry = root_fc(&out);
    let fc = entry.front_controller.as_ref().expect("front controller");
    assert_eq!(fc.target_uri.as_ref(), "/index.php");
    assert!(fc.require_not_file && fc.require_not_directory && fc.preserve_query);
}

#[test]
fn reverse_cond_order_recognized() {
    let htaccess = r"
RewriteEngine On
RewriteCond %{REQUEST_FILENAME} !-d
RewriteCond %{REQUEST_FILENAME} !-f
RewriteRule ^ /index.php [L]
";
    let out = compile_htaccess(htaccess);
    assert!(root_fc(&out).front_controller.is_some());
}

#[test]
fn missing_not_file_cond_unsupported() {
    let htaccess = r"
RewriteEngine On
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
";
    let out = compile_htaccess(htaccess);
    assert!(root_fc(&out).front_controller.is_none());
}

#[test]
fn missing_not_directory_cond_unsupported() {
    let htaccess = r"
RewriteEngine On
RewriteCond %{REQUEST_FILENAME} !-f
RewriteRule . /index.php [L]
";
    let out = compile_htaccess(htaccess);
    assert!(root_fc(&out).front_controller.is_none());
}

#[test]
fn request_uri_cond_unsupported() {
    let htaccess = r"
RewriteEngine On
RewriteCond %{REQUEST_URI} !^/api
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
";
    let out = compile_htaccess(htaccess);
    assert!(root_fc(&out).front_controller.is_none());
}

#[test]
fn capture_target_unsupported() {
    let htaccess = r"
RewriteEngine On
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php?p=$1 [L]
";
    let out = compile_htaccess(htaccess);
    assert!(root_fc(&out).front_controller.is_none());
}

#[test]
fn external_target_rejected() {
    let htaccess = r"
RewriteEngine On
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . http://example.com/index.php [L]
";
    let out = compile_htaccess(htaccess);
    assert!(!out.report.errors.is_empty());
    assert!(root_fc(&out).front_controller.is_none());
}

#[test]
fn qsa_flag_unsupported() {
    let htaccess = r"
RewriteEngine On
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L,QSA]
";
    let out = compile_htaccess(htaccess);
    assert!(root_fc(&out).front_controller.is_none());
}

#[test]
fn rewrite_engine_off_prevents_execution() {
    let htaccess = r"
RewriteEngine Off
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
";
    let out = compile_htaccess(htaccess);
    assert!(root_fc(&out).front_controller.is_none());
}

#[test]
fn root_target_loop_rejected_at_compile() {
    let htaccess = r"
RewriteEngine On
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . / [L]
";
    let out = compile_htaccess(htaccess);
    assert!(!out.report.errors.is_empty());
}

#[test]
fn wordpress_fixture_compiles_front_controller() {
    let htaccess = r#"# BEGIN WordPress
<IfModule mod_rewrite.c>
RewriteEngine On
RewriteBase /
RewriteRule ^index\.php$ - [L]
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
</IfModule>
# END WordPress
"#;
    let out = compile_htaccess(htaccess);
    let fc = root_fc(&out)
        .front_controller
        .as_ref()
        .expect("wordpress front controller");
    assert_eq!(fc.target_uri.as_ref(), "/index.php");
    if let OverlayLookupResult::Continue {
        front_controller, ..
    } = lookup_overlay("/posts/hello", &out.overlay)
    {
        assert!(front_controller.is_some());
    } else {
        panic!("expected continue");
    }
}

#[test]
fn child_override_replaces_parent_front_controller() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir(root.join("app")).expect("mkdir");
    std::fs::write(
        root.join(".htaccess"),
        "RewriteEngine On\nRewriteCond %{REQUEST_FILENAME} !-f\nRewriteCond %{REQUEST_FILENAME} !-d\nRewriteRule . /index.php [L]\n",
    )
    .expect("root");
    std::fs::write(
        root.join("app/.htaccess"),
        "RewriteEngine On\nRewriteCond %{REQUEST_FILENAME} !-f\nRewriteCond %{REQUEST_FILENAME} !-d\nRewriteRule . /app.php [L]\n",
    )
    .expect("child");
    let out = exyonq_mod_htaccess::compile_vhost_overlay("site", root, 1).expect("compile");
    let app = out
        .overlay
        .entries
        .iter()
        .find(|e| e.directory.0 == "/app/")
        .expect("app entry");
    assert_eq!(
        app.front_controller
            .as_ref()
            .map(|fc| fc.target_uri.as_ref()),
        Some("/app.php")
    );
}
