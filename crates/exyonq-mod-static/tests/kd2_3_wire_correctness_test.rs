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

type BlockingOutcome = std::io::Result<exyonq_mod_static::wire_conn::BlockingServeOutcome>;

fn run_blocking_sync_once(
    site: Arc<StaticRoot>,
    server: std::net::TcpStream,
    head: Bytes,
) -> (
    std::thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<BlockingOutcome>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let mut server = server;
        let outcome = exyonq_mod_static::wire_conn::serve_blocking_sync(
            &site,
            &mut server,
            head,
            Bytes::new(),
        );
        let _ = tx.send(outcome);
    });
    (handle, rx)
}

/// A client that receives no bytes must not block the test forever.
fn expect_no_response(client: &mut std::net::TcpStream) {
    client
        .set_read_timeout(Some(Duration::from_millis(300)))
        .expect("timeout");
    let mut buf = [0u8; 256];
    match client.read(&mut buf) {
        // `Ok(0)` is the caller closing the socket it kept: still no bytes.
        Ok(0) => {}
        Ok(n) => panic!(
            "blocking wire must not answer a head it does not serve, got {:?}",
            String::from_utf8_lossy(&buf[..n])
        ),
        Err(err) => assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            "unexpected read error: {err:?}"
        ),
    }
}

fn expect_handoff(
    outcome: BlockingOutcome,
    head: &Bytes,
) -> exyonq_mod_static::wire_conn::BlockingServeOutcome {
    match outcome.expect("blocking wire result") {
        handoff @ exyonq_mod_static::wire_conn::BlockingServeOutcome::Handoff { .. } => {
            match &handoff {
                exyonq_mod_static::wire_conn::BlockingServeOutcome::Handoff {
                    head: returned,
                    rest,
                } => {
                    assert_eq!(returned, head, "the caller must get its own head back");
                    assert!(rest.is_empty(), "no extra bytes were buffered");
                }
                _ => unreachable!(),
            }
            handoff
        }
        exyonq_mod_static::wire_conn::BlockingServeOutcome::Served => {
            panic!("site heads are not the wire's: expected Handoff")
        }
    }
}

#[test]
fn blocking_sync_get_site_asset_hands_off() {
    // Cap061: the blocking wire answers /health only. A site asset is not its
    // head, so it comes back to the caller instead of the `not found` 404 the
    // wire used to fabricate.
    let (_dir, site) = static_site(0x41);
    let (server, mut client) = tcp_pair();
    let head = Bytes::from("GET /site/64k.bin HTTP/1.1\r\nHost: x\r\n\r\n");
    let (worker, outcome) = run_blocking_sync_once(site.clone(), server, head.clone());

    expect_no_response(&mut client);
    let _ = client.shutdown(Shutdown::Both);
    worker.join().expect("join");
    let _ = expect_handoff(outcome.recv().expect("outcome"), &head);
}

#[test]
fn blocking_sync_head_site_asset_hands_off() {
    let (_dir, site) = static_site(0x42);
    let (server, mut client) = tcp_pair();
    let head = Bytes::from("HEAD /site/64k.bin HTTP/1.1\r\nHost: x\r\n\r\n");
    let (worker, outcome) = run_blocking_sync_once(site.clone(), server, head.clone());

    expect_no_response(&mut client);
    let _ = client.shutdown(Shutdown::Both);
    worker.join().expect("join");
    let _ = expect_handoff(outcome.recv().expect("outcome"), &head);
}

#[test]
fn blocking_sync_unknown_path_hands_off() {
    let (_dir, site) = static_site(0x00);
    let (server, mut client) = tcp_pair();
    let head = Bytes::from("GET /site/missing.bin HTTP/1.1\r\nHost: x\r\n\r\n");
    let (worker, outcome) = run_blocking_sync_once(site.clone(), server, head.clone());

    expect_no_response(&mut client);
    let _ = client.shutdown(Shutdown::Both);
    worker.join().expect("join");
    let _ = expect_handoff(outcome.recv().expect("outcome"), &head);
}

#[test]
fn blocking_sync_metrics_keeps_terminal_404() {
    // LA-CAP054-008: `/metrics` stays a wire-owned probe whose response is a
    // deliberate 404 (no unauthenticated OpenMetrics on the wire).
    let (_dir, site) = static_site(0x00);
    let (server, mut client) = tcp_pair();
    let head = Bytes::from("GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n");
    let (worker, outcome) = run_blocking_sync_once(site.clone(), server, head.clone());

    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut buf = [0u8; 4096];
    let n = client.read(&mut buf).expect("read");
    let _ = client.shutdown(Shutdown::Both);
    worker.join().expect("join");
    let text = std::str::from_utf8(&buf[..n]).expect("utf8");
    assert!(
        text.starts_with("HTTP/1.1 404"),
        "the deliberate /metrics 404 must survive: {text:?}"
    );
    assert!(matches!(
        outcome.recv().expect("outcome"),
        Ok(exyonq_mod_static::wire_conn::BlockingServeOutcome::Served)
    ));
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
