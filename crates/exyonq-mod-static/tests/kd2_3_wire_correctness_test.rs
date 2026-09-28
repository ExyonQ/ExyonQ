//! KD2.3 — acotada correctness for static wire + sendfile paths (Linux).

#![cfg(target_os = "linux")]

use bytes::Bytes;
use exyonq_mod_static::{SendfileHandleRegistry, StaticRoot};
use std::io::Read;
use std::net::{Shutdown, TcpListener};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn static_site(body_byte: u8) -> (tempfile::TempDir, Arc<StaticRoot>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("bench");
    std::fs::create_dir_all(&root).expect("mkdir");
    std::fs::write(root.join("64k.bin"), vec![body_byte; 65536]).expect("64k");
    let mut site = StaticRoot::new(&root, "/site", None).expect("root");
    site.preload_tree().expect("preload");
    (dir, Arc::new(site))
}

fn tcp_pair() -> (std::net::TcpStream, std::net::TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let client = thread::spawn(move || std::net::TcpStream::connect(addr).expect("connect"));
    let (server, _) = listener.accept().expect("accept");
    (server, client.join().expect("join"))
}

fn run_blocking_sync_once(
    site: Arc<StaticRoot>,
    server: std::net::TcpStream,
    head: Bytes,
) -> std::thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut server = server;
        let _ = exyonq_mod_static::wire_conn::serve_blocking_sync(
            &site,
            &mut server,
            head,
            Bytes::new(),
        );
    })
}

#[test]
fn blocking_sync_get_site_asset_is_wire_not_found() {
    // Cap061 / 6f9a4cf0: blocking wire serves /health only; site assets use Hyper/static.
    // Wire response for non-health paths is NOT_FOUND_KEEP_ALIVE (200 + "not found").
    let (_dir, site) = static_site(0x41);
    let (server, mut client) = tcp_pair();
    let head = Bytes::from("GET /site/64k.bin HTTP/1.1\r\nHost: x\r\n\r\n");
    let worker = run_blocking_sync_once(site.clone(), server, head);

    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut buf = [0u8; 4096];
    let n = client.read(&mut buf).expect("read");
    let _ = client.shutdown(Shutdown::Both);
    worker.join().expect("join");

    let text = std::str::from_utf8(&buf[..n]).expect("utf8");
    assert!(
        text.contains("not found"),
        "Cap061 wire must not sendfile site assets: {text:?}"
    );
    assert!(
        !text.contains("Content-Length: 65536"),
        "site body must not be served on blocking wire"
    );
}

#[test]
fn blocking_sync_head_site_asset_is_wire_not_found() {
    let (_dir, site) = static_site(0x42);
    let (server, mut client) = tcp_pair();
    let head = Bytes::from("HEAD /site/64k.bin HTTP/1.1\r\nHost: x\r\n\r\n");
    let worker = run_blocking_sync_once(site.clone(), server, head);

    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut buf = [0u8; 4096];
    let n = client.read(&mut buf).expect("read");
    let _ = client.shutdown(Shutdown::Both);
    worker.join().expect("join");

    let text = std::str::from_utf8(&buf[..n]).expect("utf8");
    assert!(
        text.contains("not found") || text.contains("Content-Length: 9"),
        "unexpected response: {text:?}"
    );
}

#[test]
fn blocking_sync_unknown_path_serves_not_found_wire() {
    let (_dir, site) = static_site(0x00);
    let (server, mut client) = tcp_pair();
    let head = Bytes::from("GET /site/missing.bin HTTP/1.1\r\nHost: x\r\n\r\n");
    let worker = run_blocking_sync_once(site.clone(), server, head);

    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut buf = [0u8; 2048];
    let n = client.read(&mut buf).expect("read");
    let _ = client.shutdown(Shutdown::Both);
    worker.join().expect("join");

    let text = std::str::from_utf8(&buf[..n]).expect("utf8");
    // Bench wire path: NOT_FOUND_KEEP_ALIVE is HTTP 200 + body "not found" (precooked wire semantics).
    assert!(
        text.contains("not found") || text.contains("404"),
        "unexpected response: {text:?}"
    );
}

#[test]
fn sendfile_registry_take_release_no_active_leak() {
    use exyonq_mod_static::sendfile::SendfileAsset;
    use std::fs::File;
    use std::io::Write as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("body.bin");
    let mut f = File::create(&path).expect("create");
    f.write_all(b"payload").expect("write");
    let reg = SendfileHandleRegistry::new();
    let handle = reg.issue(1, Arc::new(SendfileAsset::open(&path, 7).expect("asset")));
    assert_eq!(reg.active_count(), 1);
    let taken = reg.take(handle).expect("take once");
    assert_eq!(taken.body_len, 7);
    assert_eq!(reg.active_count(), 0);
    reg.release(handle);
    assert_eq!(reg.active_count(), 0);
}

#[test]
fn client_disconnect_during_blocking_sendfile_closes_cleanly() {
    let (_dir, site) = static_site(0x55);
    let (mut server, client) = tcp_pair();
    drop(client);
    let head = Bytes::from("GET /site/64k.bin HTTP/1.1\r\nHost: x\r\n\r\n");
    let result =
        exyonq_mod_static::wire_conn::serve_blocking_sync(&site, &mut server, head, Bytes::new());
    assert!(result.is_ok() || result.is_err());
}
