//! Watcher integration check (temp dir).

use exyonq_mod_htaccess::{compile_and_publish, spawn_watcher, HtaccessSite, OverlayPublisher};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn spawn_watcher_publishes_initial_overlay() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join(".htaccess"), "Redirect 301 /old /new\n").expect("write");
        let publisher = Arc::new(OverlayPublisher::new(1));
        let site = HtaccessSite {
            site_id: "site-a".into(),
            document_root: tmp.path().to_path_buf(),
        };
        spawn_watcher(vec![site], Arc::clone(&publisher)).expect("spawn");
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(publisher.get("site-a").is_some());
        assert!(publisher.overlay_generation() >= 1);
    });
}

#[test]
fn compile_generations_monotonic_and_fail_closed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".htaccess"), "Redirect 301 /old /new\n").expect("write");
    let publisher = Arc::new(OverlayPublisher::new(1));
    let site = HtaccessSite {
        site_id: "site-a".into(),
        document_root: tmp.path().to_path_buf(),
    };
    compile_and_publish(&site, &publisher).expect("gen1");
    let gen1 = publisher.overlay_generation();
    std::fs::write(tmp.path().join(".htaccess"), "Redirect 999 /bad /worse\n").expect("write");
    assert!(compile_and_publish(&site, &publisher).is_err());
    assert_eq!(publisher.overlay_generation(), gen1);
    std::fs::write(tmp.path().join(".htaccess"), "Redirect 302 /old /newer\n").expect("write");
    compile_and_publish(&site, &publisher).expect("gen3");
    assert!(publisher.overlay_generation() > gen1);
    std::fs::remove_file(tmp.path().join(".htaccess")).expect("remove");
    compile_and_publish(&site, &publisher).expect("gen4");
    assert!(publisher.get("site-a").unwrap().entries.is_empty());
}

#[test]
fn compile_and_publish_roundtrip() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".htaccess"), "Redirect 301 /a /b\n").expect("write");
    let publisher = Arc::new(OverlayPublisher::new(1));
    let site = HtaccessSite {
        site_id: "site-a".into(),
        document_root: tmp.path().to_path_buf(),
    };
    compile_and_publish(&site, &publisher).expect("publish");
    assert!(publisher.get("site-a").is_some());
}
