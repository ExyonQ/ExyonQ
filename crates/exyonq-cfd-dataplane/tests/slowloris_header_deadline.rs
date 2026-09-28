//! L4-P2-001 — absolute client-header deadline under real TCP drip (slowloris).
//!
//! Idle-style refresh on every readable byte must NOT extend header acquisition.
//! Concurrent healthy requests must still complete while attackers drip.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

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

fn spawn_ok_upstream() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let _ =
                s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
    addr
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
    // G1 is published at start; bump to G2 with the test route table.
    child.publish_routes(2, &table).expect("publish");
    thread::sleep(Duration::from_millis(250));
    (dir, child, listen)
}

fn read_status(s: &mut TcpStream) -> Option<u16> {
    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match s.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&buf);
    text.split_whitespace().nth(1)?.parse().ok()
}

/// Drip incomplete headers forever at an interval that would refresh an idle timer.
fn drip_slowloris(listen: SocketAddr, interval: Duration, max: Duration) -> Duration {
    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.set_nodelay(true).ok();
    s.set_write_timeout(Some(Duration::from_secs(2))).ok();
    s.set_read_timeout(Some(Duration::from_millis(200))).ok();
    let started = Instant::now();
    let _ = s.write_all(b"GET /api HTTP/1.1\r\n");
    let mut n = 0u32;
    while started.elapsed() < max {
        thread::sleep(interval);
        let line = format!("X-Drip-{n}: a\r\n");
        if s.write_all(line.as_bytes()).is_err() {
            break;
        }
        n += 1;
        let mut b = [0u8; 64];
        match s.read(&mut b) {
            Ok(0) => break,
            Ok(_) => break, // e.g. 408 timeout response
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
    }
    let lived = started.elapsed();
    let _ = s.shutdown(Shutdown::Both);
    lived
}

#[test]
fn absolute_header_deadline_closes_single_drip() {
    let up = spawn_ok_upstream();
    let (_dir, child, listen) = start_dp(up);

    // Drip every 400ms — faster than a naïve 5s idle refresh, so a refresh bug
    // would keep the conn alive past ~5s. Absolute deadline must kill by ~6–8s.
    let lived = drip_slowloris(listen, Duration::from_millis(400), Duration::from_secs(12));
    assert!(
        lived < Duration::from_secs(8),
        "slowloris conn lived {lived:?}; absolute header deadline should close ≤~5s+poll slack"
    );
    assert!(
        lived > Duration::from_secs(2),
        "closed too fast ({lived:?}); deadline should allow brief incomplete headers"
    );

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.write_all(b"GET /api HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n")
        .unwrap();
    assert_eq!(read_status(&mut s), Some(200));

    child.shutdown().unwrap();
}

#[test]
fn absolute_header_deadline_ten_drips_healthy_progress() {
    let up = spawn_ok_upstream();
    let (_dir, child, listen) = start_dp(up);
    let healthy_ok = Arc::new(AtomicUsize::new(0));

    let mut attackers = Vec::new();
    for _ in 0..10 {
        let addr = listen;
        attackers.push(thread::spawn(move || {
            drip_slowloris(addr, Duration::from_millis(300), Duration::from_secs(12))
        }));
    }

    let healthy = Arc::clone(&healthy_ok);
    let healthy_thread = thread::spawn(move || {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if let Ok(mut s) = TcpStream::connect_timeout(&listen, Duration::from_secs(1)) {
                if s.write_all(b"GET /api HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n")
                    .is_ok()
                    && read_status(&mut s) == Some(200)
                {
                    healthy.fetch_add(1, Ordering::Relaxed);
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    });

    let mut max_life = Duration::ZERO;
    for t in attackers {
        let lived = t.join().expect("attacker join");
        max_life = max_life.max(lived);
    }
    healthy_thread.join().expect("healthy join");

    assert!(
        max_life < Duration::from_secs(8),
        "attacker max lifetime {max_life:?} exceeds absolute header budget"
    );
    assert!(
        healthy_ok.load(Ordering::Relaxed) >= 5,
        "healthy traffic starved under slowloris; ok={}",
        healthy_ok.load(Ordering::Relaxed)
    );

    child.shutdown().unwrap();
}

#[test]
fn normal_and_fragmented_headers_still_succeed() {
    let up = spawn_ok_upstream();
    let (_dir, child, listen) = start_dp(up);

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.set_nodelay(true).ok();
    s.write_all(b"GET /api HTTP/1.1\r\n").unwrap();
    thread::sleep(Duration::from_millis(50));
    s.write_all(b"Host: h\r\n").unwrap();
    thread::sleep(Duration::from_millis(50));
    s.write_all(b"Connection: close\r\n\r\n").unwrap();
    assert_eq!(read_status(&mut s), Some(200));

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    let pad = "x".repeat(2048);
    let req = format!("GET /api HTTP/1.1\r\nHost: h\r\nX-Pad: {pad}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).unwrap();
    assert_eq!(read_status(&mut s), Some(200));

    child.shutdown().unwrap();
}
