//! Linux-only pread fallback oracle — real dataplane + controlled eligible sendfile errno.
#![cfg(all(target_os = "linux", feature = "test-utils"))]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use exyonq_cfd_control::{CfdChild, CfdLaunchConfig};
use exyonq_cfd_gen::{BackendKind, CompiledRoute, CompiledStaticPolicy, RouteTable};
use tempfile::tempdir;

const ORACLE_PAYLOAD: &[u8] = b"pread-oracle-payload-v2-deterministic-4096-bytes!!";

fn dataplane_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_exyonq-dataplane") {
        return PathBuf::from(p);
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target/debug/exyonq-dataplane");
    p
}

fn free_listen() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    a.to_string()
}

fn static_table(root: &std::path::Path) -> RouteTable {
    RouteTable {
        upstreams: Vec::new(),
        fcgi_pools: Vec::new(),
        static_policies: vec![CompiledStaticPolicy {
            id: 3,
            document_root: root.to_string_lossy().into_owned(),
            flags: 0,
            index: vec!["index.html".into()],
        }],
        routes: vec![CompiledRoute {
            route_id: 30,
            host: Some("h".into()),
            path: "/static".into(),
            backend_kind: BackendKind::Static,
            backend_id: 3,
        }],
    }
}

fn body_from_response(raw: &[u8]) -> Vec<u8> {
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(raw.len());
    raw[head_end..].to_vec()
}

fn http_get_body(addr: &str, path: &str) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let req = format!("GET {path} HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).unwrap();
    let status = String::from_utf8_lossy(&buf)
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, body_from_response(&buf))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut cmd = Command::new("sha256sum");
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = cmd.spawn().expect("sha256sum");
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(bytes).unwrap();
    }
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(child.wait().unwrap().success());
    out.split_whitespace().next().unwrap_or("").to_string()
}

fn start_with_table(table: &RouteTable) -> (tempfile::TempDir, CfdChild, String) {
    let dir = tempdir().unwrap();
    let listen = free_listen();
    let cfg = CfdLaunchConfig {
        listen: listen.clone(),
        gen_dir: dir.path().to_path_buf(),
        shards: 1,
        dataplane_bin: dataplane_bin(),
        ready_timeout: Duration::from_secs(20),
    };
    assert!(
        cfg.dataplane_bin.is_file(),
        "missing {:?}",
        cfg.dataplane_bin
    );
    let child = CfdChild::start(&cfg).expect("start");
    child.publish_routes(2, table).expect("publish");
    thread::sleep(Duration::from_millis(250));
    (dir, child, listen)
}

#[test]
fn native_static_pread_fallback_on_eligible_sendfile_errno() {
    let docroot = tempdir().unwrap();
    std::fs::write(docroot.path().join("oracle.bin"), ORACLE_PAYLOAD).unwrap();
    let expected_sha = sha256_hex(ORACLE_PAYLOAD);
    let table = static_table(docroot.path());

    // Inherited by dataplane child (Command default env); compiled only with test-utils.
    std::env::set_var("EXYONQ_TEST_SENDFILE_ERRNO", "22");
    let (_dir, child, listen) = start_with_table(&table);

    let (status, body) = http_get_body(&listen, "/static/oracle.bin");
    assert_eq!(status, 200, "body len={}", body.len());
    assert_eq!(body.as_slice(), ORACLE_PAYLOAD);
    assert_eq!(sha256_hex(&body), expected_sha);

    child.shutdown().unwrap();
    std::env::remove_var("EXYONQ_TEST_SENDFILE_ERRNO");
}

#[test]
fn native_static_normal_sendfile_without_errno_override() {
    std::env::remove_var("EXYONQ_TEST_SENDFILE_ERRNO");
    let docroot = tempdir().unwrap();
    std::fs::write(docroot.path().join("plain.bin"), ORACLE_PAYLOAD).unwrap();
    let table = static_table(docroot.path());
    let (_dir, child, listen) = start_with_table(&table);

    let (status, body) = http_get_body(&listen, "/static/plain.bin");
    assert_eq!(status, 200);
    assert_eq!(body.as_slice(), ORACLE_PAYLOAD);

    child.shutdown().unwrap();
}

#[test]
fn native_static_ineligible_errno_does_not_serve_full_payload_via_pread() {
    let docroot = tempdir().unwrap();
    std::fs::write(docroot.path().join("fail.bin"), ORACLE_PAYLOAD).unwrap();
    let table = static_table(docroot.path());

    std::env::set_var("EXYONQ_TEST_SENDFILE_ERRNO", "5"); // EIO — not eligible for pread fallback
    let (_dir, child, listen) = start_with_table(&table);

    let (status, body) = http_get_body(&listen, "/static/fail.bin");
    // Headers may already be 200 before body pump; ineligible errno must not deliver full payload.
    assert_ne!(body.as_slice(), ORACLE_PAYLOAD, "status={status}");
    assert!(
        body.len() < ORACLE_PAYLOAD.len(),
        "status={status} len={}",
        body.len()
    );

    child.shutdown().unwrap();
    std::env::remove_var("EXYONQ_TEST_SENDFILE_ERRNO");
}
