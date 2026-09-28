//! Phase-4 real TCP request/response body streaming through exyonq-dataplane.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use exyonq_cfd_control::{
    projection_from_compile_input, representative_phase3_waf_input, table_from_entries, CfdChild,
    CfdLaunchConfig, CompositeProjection,
};
use exyonq_cfd_gen::RouteTable;
use tempfile::tempdir;

#[derive(Default)]
struct UpstreamStats {
    hits: u64,
    body_bytes: u64,
    last_hash: u64,
}

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

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn spawn_echo_upstream(name: &'static str) -> (SocketAddr, Arc<Mutex<UpstreamStats>>) {
    let stats = Arc::new(Mutex::new(UpstreamStats::default()));
    let stats2 = Arc::clone(&stats);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let stats = Arc::clone(&stats2);
            thread::spawn(move || {
                let mut s = stream;
                s.set_read_timeout(Some(Duration::from_secs(15))).ok();
                while let Some((method, body)) = read_request(&mut s) {
                    if let Ok(mut g) = stats.lock() {
                        g.hits = g.hits.saturating_add(1);
                        g.body_bytes = g
                            .body_bytes
                            .saturating_add(u64::try_from(body.len()).unwrap_or(u64::MAX));
                        g.last_hash = fnv1a64(&body);
                    }
                    let response_body = if method == "GET" {
                        name.as_bytes().to_vec()
                    } else {
                        body
                    };
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\nContent-Type: application/octet-stream\r\n\r\n",
                        response_body.len()
                    );
                    if s.write_all(head.as_bytes()).is_err() {
                        break;
                    }
                    if s.write_all(&response_body).is_err() {
                        break;
                    }
                }
            });
        }
    });
    (addr, stats)
}

fn read_request(s: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        match s.read(&mut tmp) {
            Ok(0) => return None,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
                if buf.len() > 64 * 1024 {
                    return None;
                }
            }
            Err(_) => return None,
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]);
    let method = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or("")
        .to_string();
    let cl = head
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|line| line.split_once(':'))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[header_end..].to_vec();
    while body.len() < cl {
        match s.read(&mut tmp) {
            Ok(0) => return None,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
            Err(_) => return None,
        }
    }
    body.truncate(cl);
    Some((method, body))
}

fn read_response(s: &mut TcpStream) -> (u16, Vec<u8>) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    s.set_read_timeout(Some(Duration::from_secs(20))).ok();
    loop {
        match s.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..end + 4]);
                    let cl = head
                        .lines()
                        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|line| line.split_once(':'))
                        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + cl {
                        let status = head
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().nth(1))
                            .and_then(|value| value.parse::<u16>().ok())
                            .unwrap_or(0);
                        return (status, buf[end + 4..end + 4 + cl].to_vec());
                    }
                }
            }
            Err(_) => break,
        }
    }
    (0, buf)
}

fn start_with_table(table: &RouteTable) -> (tempfile::TempDir, CfdChild, SocketAddr) {
    let dir = tempdir().unwrap();
    let listen = free_addr();
    let cfg = CfdLaunchConfig {
        listen: listen.to_string(),
        gen_dir: dir.path().to_path_buf(),
        shards: 1,
        dataplane_bin: dataplane_bin(),
        ready_timeout: Duration::from_secs(20),
    };
    let child = CfdChild::start(&cfg).expect("start dataplane");
    child.publish_routes(2, table).expect("publish routes");
    thread::sleep(Duration::from_millis(250));
    (dir, child, listen)
}

fn publish_waf(
    child: &CfdChild,
    gen_id: u64,
    table: &RouteTable,
    waf: &exyonq_waf::WafCompileInput,
) {
    let proj = projection_from_compile_input(waf).unwrap();
    let composite = CompositeProjection {
        routes: table.clone(),
        waf: Some(proj),
    };
    child.publish_composite(gen_id, &composite).unwrap();
    thread::sleep(Duration::from_millis(250));
}

fn body(size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| u8::try_from(i % 251).unwrap_or(0))
        .collect()
}

#[test]
fn content_length_body_hash_matrix() {
    let (up, stats) = spawn_echo_upstream("up-a");
    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);

    for size in [0usize, 1, 1024, 16 * 1024, 64 * 1024, 1024 * 1024] {
        let payload = body(size);
        let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
        let head = format!(
            "POST /api/echo HTTP/1.1\r\nHost: h\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        s.write_all(head.as_bytes()).unwrap();
        s.write_all(&payload).unwrap();
        let (status, response_body) = read_response(&mut s);
        assert_eq!(status, 200, "size={size}");
        assert_eq!(fnv1a64(&response_body), fnv1a64(&payload), "size={size}");
        assert_eq!(response_body.len(), payload.len(), "size={size}");
    }

    let g = stats.lock().unwrap();
    assert_eq!(g.hits, 6);
    child.shutdown().unwrap();
}

#[test]
fn keepalive_post_then_get() {
    let (up, _) = spawn_echo_upstream("get-ok");
    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    let payload = b"abc123";
    let head = format!(
        "POST /api/echo HTTP/1.1\r\nHost: h\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(payload).unwrap();
    let (st1, b1) = read_response(&mut s);
    assert_eq!(st1, 200);
    assert_eq!(b1, payload);

    s.write_all(b"GET /api/name HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n")
        .unwrap();
    let (st2, b2) = read_response(&mut s);
    assert_eq!(st2, 200);
    assert_eq!(b2, b"get-ok");
    child.shutdown().unwrap();
}

#[test]
fn request_framing_rejects_te_cl_te_and_duplicate_cl() {
    let (up, stats) = spawn_echo_upstream("up");
    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);

    for raw in [
        "POST /api HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        "POST /api HTTP/1.1\r\nHost: h\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nx",
        "POST /api HTTP/1.1\r\nHost: h\r\nContent-Length: 1\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx",
    ] {
        let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
        let _ = s.write_all(raw.as_bytes());
        let (status, _) = read_response(&mut s);
        assert_eq!(status, 400, "raw={raw:?}");
    }

    assert_eq!(stats.lock().unwrap().hits, 0);
    child.shutdown().unwrap();
}

#[test]
fn waf_header_deny_before_any_upstream_body_bytes() {
    let (up, stats) = spawn_echo_upstream("up");
    let table = table_from_entries(&[(None, "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);
    let waf = representative_phase3_waf_input();
    publish_waf(&child, 3, &table, &waf);

    let payload = body(64 * 1024);
    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    let head = format!(
        "POST /api/echo HTTP/1.1\r\nHost: app.example\r\nX-Attack: evil\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(&payload);
    let (status, response_body) = read_response(&mut s);
    assert_eq!(status, 403);
    assert!(String::from_utf8_lossy(&response_body).contains("waf denied"));
    let g = stats.lock().unwrap();
    assert_eq!(g.hits, 0);
    assert_eq!(g.body_bytes, 0);
    child.shutdown().unwrap();
}

#[test]
fn generation_refresh_after_long_body_uses_fresh_next_request() {
    let (up_a, _) = spawn_echo_upstream("gen-a");
    let (up_b, _) = spawn_echo_upstream("gen-b");
    let table_a = table_from_entries(&[(Some("h"), "/api", up_a, "up-a")]);
    let table_b = table_from_entries(&[(Some("h"), "/api", up_b, "up-b")]);
    let (_dir, child, listen) = start_with_table(&table_a);

    let payload = body(128 * 1024);
    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    let head = format!(
        "POST /api/echo HTTP/1.1\r\nHost: h\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(&payload[..16 * 1024]).unwrap();
    child.publish_routes(3, &table_b).unwrap();
    thread::sleep(Duration::from_millis(250));
    s.write_all(&payload[16 * 1024..]).unwrap();
    let (st1, b1) = read_response(&mut s);
    assert_eq!(st1, 200);
    assert_eq!(fnv1a64(&b1), fnv1a64(&payload));

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_fresh = false;
    while Instant::now() < deadline {
        s.write_all(b"GET /api/name HTTP/1.1\r\nHost: h\r\nConnection: keep-alive\r\n\r\n")
            .unwrap();
        let (st2, b2) = read_response(&mut s);
        assert_eq!(st2, 200);
        if b2 == b"gen-b" {
            saw_fresh = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(
        saw_fresh,
        "next same-socket request did not observe generation 3"
    );
    child.shutdown().unwrap();
}

#[test]
fn slow_client_body_relay_completes_with_bounded_buffers() {
    let (up, _) = spawn_echo_upstream("slow");
    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);

    let payload = body(64 * 1024);
    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    let head = format!(
        "POST /api/slow HTTP/1.1\r\nHost: h\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes()).unwrap();
    for chunk in payload.chunks(1024) {
        s.write_all(chunk).unwrap();
        thread::sleep(Duration::from_millis(2));
    }
    let (status, response_body) = read_response(&mut s);
    assert_eq!(status, 200);
    assert_eq!(fnv1a64(&response_body), fnv1a64(&payload));
    child.shutdown().unwrap();
}

#[test]
fn large_request_headers_still_stream_body() {
    // Regression: request head must not share the 16KiB body relay buffer.
    let (up, stats) = spawn_echo_upstream("hdr");
    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);

    let payload = body(8 * 1024);
    let pad = "x".repeat(20 * 1024);
    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    let head = format!(
        "POST /api/echo HTTP/1.1\r\nHost: h\r\nX-Pad: {pad}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(&payload).unwrap();
    let (status, response_body) = read_response(&mut s);
    assert_eq!(status, 200);
    assert_eq!(fnv1a64(&response_body), fnv1a64(&payload));
    assert_eq!(stats.lock().unwrap().hits, 1);
    child.shutdown().unwrap();
}

#[test]
fn upstream_trailing_bytes_do_not_corrupt_next_keepalive_request() {
    // Upstream responds with CL body + trailing garbage. Phase-4 fail-closed
    // poison must not reuse that upstream socket for the next client request.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let up = listener.local_addr().unwrap();
    thread::spawn(move || {
        let mut n = 0u64;
        for stream in listener.incoming().flatten() {
            n += 1;
            let mut s = stream;
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let Some((_method, body)) = read_request(&mut s) else {
                continue;
            };
            let response_body = if n == 1 { b"first".to_vec() } else { body };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                response_body.len()
            );
            if s.write_all(head.as_bytes()).is_err() {
                continue;
            }
            if n == 1 {
                let mut wire = response_body;
                wire.push(b'Z');
                let _ = s.write_all(&wire);
            } else {
                let _ = s.write_all(&response_body);
            }
        }
    });

    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.write_all(
        b"POST /api/a HTTP/1.1\r\nHost: h\r\nContent-Length: 1\r\nConnection: keep-alive\r\n\r\nx",
    )
    .unwrap();
    let (st1, b1) = read_response(&mut s);
    assert_eq!(st1, 200);
    assert_eq!(b1, b"first");

    let payload = b"second-body";
    let head = format!(
        "POST /api/b HTTP/1.1\r\nHost: h\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(payload).unwrap();
    let (st2, b2) = read_response(&mut s);
    assert_eq!(st2, 200);
    assert_eq!(b2, payload);
    child.shutdown().unwrap();
}

#[test]
fn clean_upstream_keepalive_reuses_single_accept() {
    // Causal reuse proof: N client requests ⇒ 1 upstream accept when responses are clean CL.
    let accepts = Arc::new(Mutex::new(0u64));
    let requests = Arc::new(Mutex::new(0u64));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let up = listener.local_addr().unwrap();
    let accepts2 = Arc::clone(&accepts);
    let requests2 = Arc::clone(&requests);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            *accepts2.lock().unwrap() += 1;
            let mut s = stream;
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut seq = 0u64;
            while let Some((_method, _body)) = read_request(&mut s) {
                *requests2.lock().unwrap() += 1;
                seq += 1;
                let body = format!("nonce-{seq}");
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                    body.len()
                );
                if s.write_all(head.as_bytes()).is_err() {
                    break;
                }
                if s.write_all(body.as_bytes()).is_err() {
                    break;
                }
            }
        }
    });

    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);

    let n = 20u64;
    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    for i in 1..=n {
        let close = if i == n { "close" } else { "keep-alive" };
        let req = format!("GET /api/r{i} HTTP/1.1\r\nHost: h\r\nConnection: {close}\r\n\r\n");
        s.write_all(req.as_bytes()).unwrap();
        let (st, body) = read_response(&mut s);
        assert_eq!(st, 200, "req {i}");
        assert_eq!(body, format!("nonce-{i}").as_bytes(), "req {i} nonce");
    }

    // Allow idle pool bookkeeping to settle.
    thread::sleep(Duration::from_millis(50));
    let accepts_n = *accepts.lock().unwrap();
    let requests_n = *requests.lock().unwrap();
    assert_eq!(requests_n, n, "upstream must see every request");
    assert_eq!(
        accepts_n, 1,
        "clean CL keepalive must reuse one upstream accept; got {accepts_n}"
    );
    child.shutdown().unwrap();
}

#[test]
fn delayed_trailing_byte_forces_fresh_accept() {
    // Trailing byte after a clean response must poison idle/reuse (ADR-021 idle READABLE).
    let accepts = Arc::new(Mutex::new(0u64));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let up = listener.local_addr().unwrap();
    let accepts2 = Arc::clone(&accepts);
    thread::spawn(move || {
        let mut n = 0u64;
        for stream in listener.incoming().flatten() {
            *accepts2.lock().unwrap() += 1;
            n += 1;
            let mut s = stream;
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let Some((_method, body)) = read_request(&mut s) else {
                continue;
            };
            let response_body = if n == 1 { b"clean".to_vec() } else { body };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                response_body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&response_body);
            if n == 1 {
                // Delayed trailing byte after response complete (idle-window adversary).
                thread::sleep(Duration::from_millis(30));
                let _ = s.write_all(b"X");
            }
        }
    });

    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.write_all(
        b"POST /api/a HTTP/1.1\r\nHost: h\r\nContent-Length: 1\r\nConnection: keep-alive\r\n\r\nx",
    )
    .unwrap();
    let (st1, b1) = read_response(&mut s);
    assert_eq!(st1, 200);
    assert_eq!(b1, b"clean");

    // Wait for idle READABLE discard of the poisoned pooled conn.
    thread::sleep(Duration::from_millis(80));

    s.write_all(
        b"POST /api/b HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnext",
    )
    .unwrap();
    let (st2, b2) = read_response(&mut s);
    assert_eq!(st2, 200);
    assert_eq!(b2, b"next");

    let accepts_n = *accepts.lock().unwrap();
    assert!(
        accepts_n >= 2,
        "delayed trailing must prevent reuse; accepts={accepts_n}"
    );
    child.shutdown().unwrap();
}

#[test]
fn checkout_window_trailing_never_parsed_as_response() {
    // After a clean pooled response, spray trailing bytes while the next client
    // request is in flight. Poison must not be returned as the second response body.
    let accepts = Arc::new(Mutex::new(0u64));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let up = listener.local_addr().unwrap();
    let accepts2 = Arc::clone(&accepts);
    thread::spawn(move || {
        let mut n = 0u64;
        for stream in listener.incoming().flatten() {
            *accepts2.lock().unwrap() += 1;
            n += 1;
            let mut s = stream;
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let Some((_method, body)) = read_request(&mut s) else {
                continue;
            };
            if n == 1 {
                let response_body = b"ok1";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                    response_body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(response_body);
                for _ in 0..40 {
                    thread::sleep(Duration::from_millis(2));
                    if s.write_all(b"HTTP/1.1 500 X\r\nContent-Length: 4\r\n\r\nNOPE")
                        .is_err()
                    {
                        break;
                    }
                }
            } else {
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        }
    });

    let table = table_from_entries(&[(Some("h"), "/api", up, "upstream")]);
    let (_dir, child, listen) = start_with_table(&table);

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.write_all(b"GET /api/1 HTTP/1.1\r\nHost: h\r\nConnection: keep-alive\r\n\r\n")
        .unwrap();
    let (st1, b1) = read_response(&mut s);
    assert_eq!(st1, 200);
    assert_eq!(b1, b"ok1");

    thread::sleep(Duration::from_millis(15));
    s.write_all(
        b"POST /api/2 HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\nConnection: close\r\n\r\ngood",
    )
    .unwrap();
    let (st2, b2) = read_response(&mut s);
    assert_eq!(st2, 200, "second response must not be poison status");
    assert_eq!(b2, b"good", "second body must not be poison payload");
    assert_ne!(b2, b"NOPE");
    child.shutdown().unwrap();
}
