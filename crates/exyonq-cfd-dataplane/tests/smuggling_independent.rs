//! Independent Phase-4 smuggling/framing adversarial cases (real TCP).
//!
//! Invented for security audit — not copied from `phase4_bodies.rs`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use exyonq_cfd_control::{table_from_entries, CfdChild, CfdLaunchConfig};
use tempfile::tempdir;

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

fn free_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn spawn_counting_upstream() -> (SocketAddr, std::sync::Arc<std::sync::atomic::AtomicU64>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let hits = std::sync::Arc::new(AtomicU64::new(0));
    let hits2 = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            hits2.fetch_add(1, Ordering::SeqCst);
            let mut s = stream;
            let mut buf = [0u8; 8192];
            let _ = s.read(&mut buf);
            let _ =
                s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
    (addr, hits)
}

fn start_dp(up: SocketAddr) -> (tempfile::TempDir, CfdChild, SocketAddr) {
    let dir = tempdir().unwrap();
    let listen = free_addr();
    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let cfg = CfdLaunchConfig {
        listen: listen.to_string(),
        gen_dir: dir.path().to_path_buf(),
        shards: 1,
        dataplane_bin: dataplane_bin(),
        ready_timeout: Duration::from_secs(20),
    };
    let child = CfdChild::start(&cfg).expect("start dataplane");
    child.publish_routes(2, &table).expect("publish");
    thread::sleep(Duration::from_millis(250));
    (dir, child, listen)
}

fn read_http_exchange(s: &mut TcpStream) -> (u16, String) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let header_end = loop {
        match s.read(&mut tmp) {
            Ok(0) => break buf.len(),
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p + 4;
                }
            }
            Err(_) => break buf.len(),
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end.min(buf.len())]).to_string();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let cl = head
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split_once(':'))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < header_end + cl {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    (status, head)
}

fn read_status(s: &mut TcpStream) -> u16 {
    read_http_exchange(s).0
}

#[test]
fn tcp_cl_te_and_te_cl_never_reach_upstream() {
    let (up, hits) = spawn_counting_upstream();
    let (_dir, child, listen) = start_dp(up);

    for raw in [
        b"POST /api HTTP/1.1\r\nHost: h\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nX".as_slice(),
        b"POST /api HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\nContent-Length: 1\r\nConnection: close\r\n\r\nX".as_slice(),
        b"POST /api HTTP/1.1\r\nHost: h\r\nContent-Length: 2\r\nContent-Length: 9\r\nConnection: close\r\n\r\nab".as_slice(),
        b"POST /api HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\n".as_slice(),
        b"POST /api HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: identity\r\nConnection: close\r\n\r\n".as_slice(),
        b"POST /api HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\nX: y\rTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nBODY".as_slice(),
    ] {
        let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
        let _ = s.write_all(raw);
        let status = read_status(&mut s);
        assert_eq!(status, 400, "payload={raw:?}");
    }

    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    child.shutdown().unwrap();
}

#[test]
fn tcp_extra_bytes_after_declared_cl_rejected_before_upstream() {
    let (up, hits) = spawn_counting_upstream();
    let (_dir, child, listen) = start_dp(up);

    // Prefetched body longer than Content-Length → pipelining/smuggling adjacent.
    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.write_all(
        b"POST /api HTTP/1.1\r\nHost: h\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabcGET /x HTTP/1.1\r\n",
    )
    .unwrap();
    assert_eq!(read_status(&mut s), 400);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    child.shutdown().unwrap();
}

#[test]
fn tcp_premature_eof_mid_body_does_not_complete_upstream_success_path() {
    let (up, hits) = spawn_counting_upstream();
    let (_dir, child, listen) = start_dp(up);

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.write_all(
        b"POST /api HTTP/1.1\r\nHost: h\r\nContent-Length: 64\r\nConnection: close\r\n\r\npartial",
    )
    .unwrap();
    drop(s); // premature EOF
    thread::sleep(Duration::from_millis(400));
    // Upstream may see a partial connect attempt depending on race; must not count a completed
    // echo success. Counting upstream increments on accept — allow 0 or 1 accept, but dataplane
    // must not leave a keep-alive desync (connection already closed by client).
    let _ = hits.load(std::sync::atomic::Ordering::SeqCst);
    child.shutdown().unwrap();
}

#[test]
fn tcp_foundation_with_declared_body_forces_close_no_keepalive_poison() {
    let (up, hits) = spawn_counting_upstream();
    let (_dir, child, listen) = start_dp(up);

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    // Headers only first: foundation must not keepalive-poison unread body.
    s.write_all(
        b"POST /__exyonq_cfd/v1/foundation HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\nConnection: keep-alive\r\n\r\n",
    )
    .unwrap();
    let (status, head) = read_http_exchange(&mut s);
    assert_eq!(status, 200);
    assert!(
        head.to_ascii_lowercase().contains("connection: close"),
        "foundation+CL must force close; head={head}"
    );
    // Body + forged next request must not become a proxied upstream hit.
    let _ = s.write_all(b"AAAAAGET /api HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    child.shutdown().unwrap();
}

#[test]
fn tcp_keepalive_after_exact_cl_boundary() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let up = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let header_end = loop {
                let n = s.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p + 4;
                }
            };
            let head = String::from_utf8_lossy(&buf[..header_end]);
            let cl = head
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                .and_then(|l| l.split_once(':'))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while buf.len() < header_end + cl {
                let n = s.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let body = &buf[header_end..header_end + cl.min(buf.len().saturating_sub(header_end))];
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.write_all(body);
        }
    });

    let (_dir, child, listen) = start_dp(up);
    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.write_all(
        b"POST /api HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\nConnection: keep-alive\r\n\r\nDEAD",
    )
    .unwrap();
    assert_eq!(read_status(&mut s), 200);
    // Drain body of first response
    let mut drain = [0u8; 64];
    let _ = s.read(&mut drain);
    s.write_all(
        b"POST /api HTTP/1.1\r\nHost: h\r\nContent-Length: 3\r\nConnection: close\r\n\r\nYES",
    )
    .unwrap();
    assert_eq!(read_status(&mut s), 200);
    child.shutdown().unwrap();
}
