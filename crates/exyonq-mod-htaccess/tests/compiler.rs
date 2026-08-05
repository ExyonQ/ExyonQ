//! Compiler and inheritance tests.

use exyonq_mod_htaccess::{
    compile_from_discovered, compile_vhost_overlay, DiscoveredFile, OverlayPublisher,
};
use exyonq_module_api::lookup_overlay;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn redirect_compiles_and_lookup() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(root.join(".htaccess"), "Redirect 301 /old /new\n").expect("write");
    let out = compile_vhost_overlay("site-a", root, 1).expect("compile");
    let result = lookup_overlay("/old", &out.overlay);
    assert!(matches!(
        result,
        exyonq_module_api::OverlayLookupResult::Redirect { status: 301, .. }
    ));
}

#[test]
fn subdir_inheritance_override() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir(root.join("admin")).expect("mkdir");
    std::fs::write(root.join(".htaccess"), "Redirect 301 /old /root-new\n").expect("write");
    std::fs::write(
        root.join("admin/.htaccess"),
        "Redirect 301 /admin-only /admin-new\n",
    )
    .expect("write");
    let out = compile_vhost_overlay("site-a", root, 1).expect("compile");
    assert!(out
        .overlay
        .entries
        .iter()
        .any(|e| e.directory.0 == "/admin/"));
}

#[test]
fn invalid_redirect_rejected_in_report() {
    let files = vec![DiscoveredFile {
        absolute: PathBuf::from("/virtual/.htaccess"),
        relative_directory: "/".into(),
        content: "Redirect 999 /a /b\n".into(),
    }];
    let out = compile_from_discovered("site", Path::new("/virtual"), 1, files).expect("compile");
    assert!(!out.report.errors.is_empty());
}

#[test]
fn publish_fail_closed_keeps_previous_generation() {
    let publisher = Arc::new(OverlayPublisher::new(1));
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".htaccess"), "Redirect 301 /old /new\n").expect("write");
    let good = compile_vhost_overlay("site-a", tmp.path(), 1).expect("compile");
    publisher
        .publish(exyonq_module_api::RuntimePatchVhostOverlay {
            site_id: "site-a".into(),
            plan_generation: 1,
            overlay: good.overlay,
        })
        .expect("publish");
    let gen1 = publisher.overlay_generation();
    publisher.record_compile_failure();
    assert_eq!(publisher.overlay_generation(), gen1);
}

#[test]
fn directory_index_inheritance_and_override() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir(root.join("admin")).expect("mkdir");
    std::fs::write(root.join(".htaccess"), "DirectoryIndex index.html\n").expect("write");
    std::fs::write(root.join("admin/.htaccess"), "DirectoryIndex admin.html\n").expect("write");
    let out = compile_vhost_overlay("site-a", root, 1).expect("compile");
    let admin_entry = out
        .overlay
        .entries
        .iter()
        .find(|e| e.directory.0 == "/admin/")
        .expect("admin entry");
    assert_eq!(
        admin_entry.directory_index.as_ref().map(|v| v[0].as_str()),
        Some("admin.html")
    );
    let root_lookup = lookup_overlay("/blog/", &out.overlay);
    if let exyonq_module_api::OverlayLookupResult::Continue {
        directory_index, ..
    } = root_lookup
    {
        assert_eq!(
            directory_index.as_ref().map(|v| v[0].as_str()),
            Some("index.html")
        );
    } else {
        panic!("expected continue at /blog/");
    }
}

#[test]
fn directory_index_invalid_candidate_rejected() {
    let files = vec![DiscoveredFile {
        absolute: PathBuf::from("/virtual/.htaccess"),
        relative_directory: "/".into(),
        content: "DirectoryIndex ../escape\n".into(),
    }];
    let out = compile_from_discovered("site", Path::new("/virtual"), 1, files).expect("compile");
    assert!(!out.report.errors.is_empty());
}

#[test]
fn directory_index_candidate_limit_enforced() {
    let args = (0..33)
        .map(|i| format!("index{i}.html"))
        .collect::<Vec<_>>();
    let content = format!("DirectoryIndex {}\n", args.join(" "));
    let files = vec![DiscoveredFile {
        absolute: PathBuf::from("/virtual/.htaccess"),
        relative_directory: "/".into(),
        content,
    }];
    let out = compile_from_discovered("site", Path::new("/virtual"), 1, files).expect("compile");
    assert!(!out.report.errors.is_empty());
}

#[test]
fn directory_index_order_preserved() {
    let files = vec![DiscoveredFile {
        absolute: PathBuf::from("/virtual/.htaccess"),
        relative_directory: "/".into(),
        content: "DirectoryIndex a.html b.html c.html\n".into(),
    }];
    let out = compile_from_discovered("site", Path::new("/virtual"), 1, files).expect("compile");
    let entry = &out.overlay.entries[0];
    let list = entry.directory_index.as_ref().expect("list");
    assert_eq!(list[0], "a.html");
    assert_eq!(list[1], "b.html");
    assert_eq!(list[2], "c.html");
}

#[test]
fn traversal_path_rejected_at_compile() {
    let files = vec![DiscoveredFile {
        absolute: PathBuf::from("/virtual/.htaccess"),
        relative_directory: "/".into(),
        content: "Redirect 301 /../escape /x\n".into(),
    }];
    let out = compile_from_discovered("site", Path::new("/virtual"), 1, files).expect("compile");
    assert!(!out.report.errors.is_empty());
}
