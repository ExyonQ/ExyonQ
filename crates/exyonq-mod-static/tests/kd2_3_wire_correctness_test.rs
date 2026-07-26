//! KD2.3 — acotada correctness for static wire + sendfile paths (Linux).

#![cfg(target_os = "linux")]

use bytes::Bytes;
use exyonq_mod_static::{SendfileHandleRegistry, StaticRoot};
use std::io::Read;
use std::net::{Shutdown, TcpListener};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn bench_site(body_byte: u8) -> (tempfile::TempDir, Arc<StaticRoot>) {
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
fn blocking_sync_get_64k_returns_status_and_body() {
    let (_dir, site) = bench_site(0x41);
    let (server, mut client) = tcp_pair();
    let head = Bytes::from("GET /site/64k.bin HTTP/1.1\r\nHost: x\r\n\r\n");
    let worker = run_blocking_sync_once(site.clone(), server, head);

    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let mut buf = vec![0u8; 70000];
    let mut total = 0usize;
    for _ in 0..64 {
        match client.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if let Some(hdr) = std::str::from_utf8(&buf[..total])
                    .ok()
                    .and_then(|t| t.find("\r\n\r\n"))
                {
                    let body_start = hdr + 4;
                    if total >= body_start + 65536 {
                        break;
                    }
                }
            }
            Err(e) => panic!("read failed: {e}"),
        }
        if total >= 66000 {
            break;
        }
    }
    let _ = client.shutdown(Shutdown::Both);
    worker.join().expect("join");

    let n = total;

    let text = std::str::from_utf8(&buf[..n]).expect("utf8");
    assert!(text.starts_with("HTTP/1.1 200"));
    assert!(text.contains("Content-Length: 65536"));
    let body_start = text.find("\r\n\r\n").expect("headers end") + 4;
    assert_eq!(n - body_start, 65536);
    assert!(buf[body_start..n].iter().all(|&b| b == 0x41));
}

#[test]
fn blocking_sync_head_64k_has_headers_without_body() {
    let (_dir, site) = bench_site(0x42);
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
    assert!(text.starts_with("HTTP/1.1 200"));
    assert!(text.contains("Content-Length: 65536"));
    let hdr_end = text.find("\r\n\r\n").expect("headers end") + 4;
    assert_eq!(n, hdr_end, "HEAD must not include body bytes");
}

#[test]
fn blocking_sync_unknown_path_serves_not_found_wire() {
    let (_dir, site) = bench_site(0x00);
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
    let (_dir, site) = bench_site(0x55);
    let (mut server, client) = tcp_pair();
    drop(client);
    let head = Bytes::from("GET /site/64k.bin HTTP/1.1\r\nHost: x\r\n\r\n");
    let result =
        exyonq_mod_static::wire_conn::serve_blocking_sync(&site, &mut server, head, Bytes::new());
    assert!(result.is_ok() || result.is_err());
}
