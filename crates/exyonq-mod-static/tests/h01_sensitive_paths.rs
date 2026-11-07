use exyonq_mod_static::sendfile::{OpenUnderRoot, SendfileAsset};
use exyonq_mod_static::{
    read_file_bytes_with_budget, read_file_range, request_path_may_be_sensitive, serve_file_sync,
    serve_head_sync, StaticError, StaticRoot, StaticRuntime,
};
use exyonq_module_api::static_dispatch::{
    StaticDispatchBody, StaticDispatchRequest, StaticDispatchService, StaticMethod,
};
use std::sync::Arc;

fn test_root() -> (tempfile::TempDir, StaticRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join(".git")).expect("git dir");
    std::fs::create_dir_all(root.join(".well-known")).expect("well-known dir");
    std::fs::write(root.join(".env"), b"SECRET=one").expect("env");
    std::fs::write(root.join(".git/config"), b"[core]").expect("git config");
    std::fs::write(root.join("config.php"), b"<?php echo 1;").expect("php");
    std::fs::write(root.join("shell.phtml"), b"<?php echo 2;").expect("phtml");
    std::fs::write(root.join("x.phar"), b"PHAR").expect("phar");
    std::fs::write(root.join("legacy.inc"), b"include").expect("inc");
    std::fs::write(root.join("ok.txt"), b"ok-body").expect("ok");
    std::fs::write(root.join(".well-known/test"), b"acme").expect("acme");
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join(".env"), root.join("linkenv")).expect("symlink");
    let mut root = StaticRoot::new(dir.path(), "/", None).expect("static root");
    root.preload_tree().expect("preload");
    (dir, root)
}

fn assert_sensitive(err: StaticError) {
    assert!(
        matches!(err, StaticError::SensitivePath | StaticError::PathTraversal),
        "unexpected error: {err:?}"
    );
}

fn assert_sensitive_result_at<T>(path: &str, result: Result<T, StaticError>) {
    match result {
        Ok(_) => panic!("expected sensitive-path error for {path}"),
        Err(err) => assert_sensitive(err),
    }
}

fn assert_sensitive_result<T>(result: Result<T, StaticError>) {
    assert_sensitive_result_at("<unknown>", result);
}

#[test]
fn preload_and_get_head_deny_sensitive_and_keep_safe_assets() {
    let (dir, root) = test_root();
    assert_eq!(root.cache_len(), 2, "only safe assets may be preloaded");

    for path in [
        "/.env",
        "/.git/config",
        "/config.php",
        "/CONFIG.PHP",
        "/shell.phtml",
        "/x.phar",
        "/legacy.inc",
        "/%2eenv",
        "/config%2Ephp",
        "/.well-known/.env",
        "/.well-known/../.env",
        "/%2e%2e/ok.txt",
    ] {
        assert_sensitive_result_at(path, root.serve_request(path));
        assert_sensitive_result_at(path, root.serve_head_request(path));
        assert_sensitive_result_at(path, root.resolved_file_path(path));
        assert!(root.lookup_h3_static(path).is_none());
    }

    // A safe-named symlink is a preload miss, but live resolution must reject
    // its canonical `.env` target. The runtime test below exercises the same
    // fallback that Hyper uses.
    assert!(matches!(
        root.serve_request("/linkenv"),
        Err(StaticError::NotFound | StaticError::SensitivePath)
    ));

    let ok = root.serve_request("/ok.txt").expect("ok GET");
    assert_eq!(ok.status(), 200);
    let acme = root.serve_request("/.well-known/test").expect("acme GET");
    assert_eq!(acme.status(), 200);
    let head = root
        .serve_head_request("/.well-known/test")
        .expect("acme HEAD");
    assert_eq!(head.status(), 200);

    let sensitive_file = dir.path().join("config.php");
    assert_sensitive_result(read_file_bytes_with_budget(&sensitive_file, None));
    assert_sensitive_result(read_file_range(&sensitive_file, 0, 1));
    assert_sensitive_result(serve_file_sync(&sensitive_file));
    assert_sensitive_result(serve_head_sync(&sensitive_file));
    assert!(read_file_bytes_with_budget(&dir.path().join("ok.txt"), None).is_ok());

    assert!(request_path_may_be_sensitive("/site/config.php"));
    assert!(request_path_may_be_sensitive("/site/%2eenv"));
    assert!(!request_path_may_be_sensitive("/site/ok.txt"));
}

#[test]
fn direct_file_helpers_deny_sensitive_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sensitive = dir.path().join("config.php");
    std::fs::write(&sensitive, b"SECRET=two").expect("env");
    assert_sensitive_result(read_file_bytes_with_budget(&sensitive, None));
    assert_sensitive_result(read_file_range(&sensitive, 0, 1));
    assert_sensitive_result(serve_file_sync(&sensitive));
    assert_sensitive_result(serve_head_sync(&sensitive));
}

#[test]
fn sendfile_helpers_reject_sensitive_targets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sensitive = dir.path().join("config.php");
    std::fs::write(&sensitive, b"source").expect("write");
    assert!(SendfileAsset::open(&sensitive, 6).is_err());

    let dotfile = dir.path().join(".env");
    std::fs::write(&dotfile, b"secret").expect("write env");
    let dotfile = dotfile.canonicalize().expect("canonical env");
    let root = StaticRoot::new(dir.path(), "/", None).expect("root");
    #[cfg(target_os = "linux")]
    let open_root = OpenUnderRoot {
        canonical_root: root.canonical_root(),
        root_dir_fd: root.root_dir_fd().expect("dirfd"),
    };
    #[cfg(not(target_os = "linux"))]
    let open_root = OpenUnderRoot {
        canonical_root: root.canonical_root(),
    };
    assert!(SendfileAsset::open_under_root_prepared(&dotfile, Some(open_root), 6, None).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_with_safe_name_does_not_smuggle_sensitive_target() {
    let (dir, root) = test_root();
    let link = dir.path().join("linkenv");
    assert!(link.is_symlink());
    assert_sensitive_result_at("/linkenv", root.resolved_file_path("/linkenv"));
    assert!(matches!(
        root.serve_request("/linkenv"),
        Err(StaticError::NotFound | StaticError::SensitivePath)
    ));
    assert_eq!(root.cache_len(), 2);
}

#[tokio::test]
async fn runtime_denies_sensitive_head_and_range() {
    let (_dir, root) = test_root();
    let runtime = StaticRuntime::new();
    runtime.bind_roots(1, vec![Arc::new(root)].into_boxed_slice());

    for (method, headers) in [
        (StaticMethod::Head, Vec::new()),
        (
            StaticMethod::Get,
            vec![("range".into(), "bytes=0-1".into())],
        ),
    ] {
        let outcome = runtime
            .dispatch(StaticDispatchRequest {
                root_slot: 0,
                method,
                request_path: Arc::from("/config.php"),
                scheme: "http",
                local_port: None,
                request_query: None,
                headers,
                materialization_budget_bytes: None,
            })
            .await;
        assert_eq!(outcome.status, 404);
        assert_eq!(
            outcome.body,
            StaticDispatchBody::Inline(b"not found".to_vec())
        );
    }
}

#[test]
fn file_root_named_sensitive_is_rejected_at_construction() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.php");
    std::fs::write(&path, b"source").expect("write");
    assert_sensitive_result(StaticRoot::new(&path, "/", None));
}
