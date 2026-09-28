//! Phase-2 real H1 GET + routing + headers matrix (real TCP upstream + real dataplane).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use exyonq_cfd_control::{table_from_entries, CfdChild, CfdLaunchConfig};
use exyonq_cfd_gen::{CompiledRoute, CompiledUpstream, RouteTable};
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

fn free_listen() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    a.to_string()
}

/// Tiny real upstream: keepalive-capable; records Host; rejects leaked hop-by-hop `Foo`.
fn spawn_upstream(seen_hosts: Arc<Mutex<Vec<String>>>) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let h = thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            s.set_read_timeout(Some(Duration::from_secs(2))).ok();
            loop {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let header_end = loop {
                    match s.read(&mut tmp) {
                        Ok(0) => break None,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break Some(end + 4);
                            }
                            if buf.len() > 64 * 1024 {
                                break None;
                            }
                        }
                        Err(_) => break None,
                    }
                };
                let Some(end) = header_end else { break };
                let req = String::from_utf8_lossy(&buf[..end]);
                let host = req
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("host:"))
                    .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
                    .unwrap_or_default();
                if let Ok(mut g) = seen_hosts.lock() {
                    g.push(host.clone());
                }
                assert!(
                    !req.lines()
                        .any(|l| l.to_ascii_lowercase().starts_with("foo:")),
                    "hop-by-hop Foo leaked upstream: {req}"
                );
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = format!("ok host={host} path={path}");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\nContent-Type: text/plain\r\n\r\n{body}",
                    body.len()
                );
                if s.write_all(resp.as_bytes()).is_err() {
                    break;
                }
            }
        }
    });
    (addr, h)
}

fn read_one_http_response(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    loop {
        let n = stream.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]);
            let cl = head
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + cl {
                return String::from_utf8_lossy(&buf[..end + 4 + cl]).into_owned();
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn start_with_table(table: &RouteTable) -> (tempfile::TempDir, CfdChild, String) {
    let dir = tempdir().unwrap();
    let listen = free_listen();
    let cfg = CfdLaunchConfig {
        listen: listen.clone(),
        gen_dir: dir.path().to_path_buf(),
        shards: 1,
        dataplane_bin: dataplane_bin(),
        ready_timeout: Duration::from_secs(15),
    };
    assert!(
        cfg.dataplane_bin.is_file(),
        "missing {:?}",
        cfg.dataplane_bin
    );
    // Start publishes empty/default from env; overwrite with our table as G1 then restart path:
    // CfdChild::start already published G1 — republish as gen 1 replace then bump via publish_routes.
    let child = CfdChild::start(&cfg).expect("start");
    // G1 from start may be empty; publish G2 so shards observe id change + table.
    child.publish_routes(2, table).expect("publish g2 routes");
    thread::sleep(Duration::from_millis(250));
    (dir, child, listen)
}

fn http_exchange(addr: &str, req: &str, keepalive: bool) -> (u16, String, Option<TcpStream>) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    stream.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    // Read until Content-Length body complete or close.
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = &buf[..end + 4];
                    let head_s = String::from_utf8_lossy(head);
                    let cl = head_s
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + cl {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if keepalive {
        (status, text, Some(stream))
    } else {
        (status, text, None)
    }
}

#[test]
fn single_get_and_host_rewrite() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (up, _jh) = spawn_upstream(Arc::clone(&seen));
    let table = table_from_entries(&[(Some("app.example"), "/api", up, "127.0.0.1")]);
    let (_d, child, listen) = start_with_table(&table);
    let req = "GET /api/x HTTP/1.1\r\nHost: app.example\r\nConnection: close\r\n\r\n";
    let (st, body, _) = http_exchange(&listen, req, false);
    assert_eq!(st, 200, "{body}");
    assert!(body.contains("ok host=127.0.0.1"), "{body}");
    let hosts = seen.lock().unwrap();
    assert_eq!(hosts.last().map(String::as_str), Some("127.0.0.1"));
    child.shutdown().unwrap();
}

#[test]
fn keepalive_100_and_hop_by_hop() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (up, _jh) = spawn_upstream(Arc::clone(&seen));
    let table = table_from_entries(&[(Some("h"), "/p", up, "127.0.0.1")]);
    let (_d, child, listen) = start_with_table(&table);
    let mut stream = TcpStream::connect(&listen).unwrap();
    for i in 0..100 {
        let req = format!(
            "GET /p/{i} HTTP/1.1\r\nHost: h\r\nConnection: keep-alive, foo\r\nFoo: smuggle\r\nX-Req: {i}\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).unwrap();
        let text = read_one_http_response(&mut stream);
        assert!(
            text.contains("HTTP/1.1 200") && text.contains("ok host="),
            "i={i} body={text}"
        );
    }
    child.shutdown().unwrap();
}

#[test]
fn multi_host_path_matrix_and_miss() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (up_a, _) = spawn_upstream(Arc::clone(&seen));
    let (up_b, _) = spawn_upstream(Arc::clone(&seen));
    let (up_c, _) = spawn_upstream(Arc::clone(&seen));
    let table = RouteTable {
        upstreams: vec![
            CompiledUpstream {
                id: 1,
                connect: up_a,
                authority_host: "ua".into(),
            },
            CompiledUpstream {
                id: 2,
                connect: up_b,
                authority_host: "ub".into(),
            },
            CompiledUpstream {
                id: 3,
                connect: up_c,
                authority_host: "uc".into(),
            },
        ],
        fcgi_pools: Vec::new(),
        static_policies: Vec::new(),
        routes: vec![
            CompiledRoute {
                route_id: 10,
                host: Some("host-a.example".into()),
                path: "/a".into(),
                backend_kind: exyonq_cfd_gen::BackendKind::Proxy,
                backend_id: 1,
            },
            CompiledRoute {
                route_id: 11,
                host: Some("host-a.example".into()),
                path: "/b".into(),
                backend_kind: exyonq_cfd_gen::BackendKind::Proxy,
                backend_id: 2,
            },
            CompiledRoute {
                route_id: 12,
                host: Some("host-b.example".into()),
                path: "/a".into(),
                backend_kind: exyonq_cfd_gen::BackendKind::Proxy,
                backend_id: 3,
            },
        ],
    };
    let (_d, child, listen) = start_with_table(&table);

    let cases = [
        ("host-a.example", "/a/x", "ua"),
        ("host-a.example", "/b/y", "ub"),
        ("host-b.example", "/a/z", "uc"),
    ];
    for (host, path, expect) in cases {
        let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        let (st, body, _) = http_exchange(&listen, &req, false);
        assert_eq!(st, 200, "{body}");
        assert!(body.contains(&format!("host={expect}")), "{body}");
    }
    let req = "GET /a HTTP/1.1\r\nHost: unknown.example\r\nConnection: close\r\n\r\n";
    let (st, _, _) = http_exchange(&listen, req, false);
    assert_eq!(st, 404);
    let req = "GET /missing HTTP/1.1\r\nHost: host-a.example\r\nConnection: close\r\n\r\n";
    let (st, _, _) = http_exchange(&listen, req, false);
    assert_eq!(st, 404);
    child.shutdown().unwrap();
}

#[test]
fn same_socket_generation_refresh() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (up_a, _) = spawn_upstream(Arc::clone(&seen));
    let (up_b, _) = spawn_upstream(Arc::clone(&seen));
    let g1 = table_from_entries(&[(Some("h"), "/x", up_a, "auth-a")]);
    let g2tab = table_from_entries(&[(Some("h"), "/x", up_b, "auth-b")]);
    let (_d, child, listen) = start_with_table(&g1);

    let mut stream = TcpStream::connect(&listen).unwrap();
    let req1 = "GET /x HTTP/1.1\r\nHost: h\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(req1.as_bytes()).unwrap();
    let t1 = read_one_http_response(&mut stream);
    assert!(t1.contains("host=auth-a"), "{t1}");

    child.publish_routes(3, &g2tab).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ok = false;
    while Instant::now() < deadline {
        stream.write_all(req1.as_bytes()).unwrap();
        let t = read_one_http_response(&mut stream);
        if t.contains("host=auth-b") {
            ok = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(ok, "same-socket did not observe G3 upstream");
    child.shutdown().unwrap();
}

#[test]
fn unsupported_method_and_transfer_encoding_fail_closed() {
    let (up, _) = spawn_upstream(Arc::new(Mutex::new(Vec::new())));
    let table = table_from_entries(&[(Some("h"), "/", up, "h")]);
    let (_d, child, listen) = start_with_table(&table);
    let req = "POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n";
    let (st, _, _) = http_exchange(&listen, req, false);
    assert_eq!(st, 400);
    let req = "OPTIONS / HTTP/1.1\r\nHost: h\r\nContent-Length: 0\r\n\r\n";
    let (st, _, _) = http_exchange(&listen, req, false);
    assert_eq!(st, 405);
    child.shutdown().unwrap();
}

#[test]
fn upstream_connect_failure_502() {
    let dead: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let table = table_from_entries(&[(Some("h"), "/", dead, "h")]);
    let (_d, child, listen) = start_with_table(&table);
    let req = "GET / HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n";
    let (st, body, _) = http_exchange(&listen, req, false);
    assert_eq!(st, 502, "{body}");
    child.shutdown().unwrap();
}

#[test]
fn bodyless_204_nonzero_cl_keepalive_no_hang() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let up = listener.local_addr().unwrap();
    thread::spawn(move || {
        for s in listener.incoming().take(4) {
            let Ok(mut s) = s else { continue };
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            // Malicious/nonconformant upstream: 204 with phantom Content-Length.
            let _ = s.write_all(
                b"HTTP/1.1 204 No Content\r\nContent-Length: 5\r\nConnection: keep-alive\r\n\r\n",
            );
        }
    });
    let table = table_from_entries(&[(Some("h"), "/", up, "auth")]);
    let (_d, child, listen) = start_with_table(&table);
    let mut stream = TcpStream::connect(&listen).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: h\r\nConnection: keep-alive\r\n\r\n")
        .unwrap();
    let text = read_one_http_response(&mut stream);
    assert!(text.starts_with("HTTP/1.1 204"), "{text}");
    assert!(
        text.to_ascii_lowercase().contains("content-length: 0"),
        "must advertise CL=0 matching empty body; got {text}"
    );
    // Second request on same socket must not hang waiting for phantom 5 bytes.
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n")
        .unwrap();
    let text2 = read_one_http_response(&mut stream);
    assert!(text2.starts_with("HTTP/1.1 204"), "{text2}");
    child.shutdown().unwrap();
}

#[test]
fn keepalive_300_zero_gap() {
    let (up, _) = spawn_upstream(Arc::new(Mutex::new(Vec::new())));
    let table = table_from_entries(&[(Some("h"), "/p", up, "127.0.0.1")]);
    let (_d, child, listen) = start_with_table(&table);
    let mut stream = TcpStream::connect(&listen).unwrap();
    for i in 0..300 {
        let req = format!("GET /p/{i} HTTP/1.1\r\nHost: h\r\nConnection: keep-alive\r\n\r\n");
        stream.write_all(req.as_bytes()).unwrap();
        let text = read_one_http_response(&mut stream);
        assert!(
            text.contains("HTTP/1.1 200") && text.contains("ok host="),
            "i={i} body={text}"
        );
    }
    child.shutdown().unwrap();
}

#[test]
fn query_path_routes_and_large_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let up = listener.local_addr().unwrap();
    thread::spawn(move || {
        for s in listener.incoming().take(20) {
            let Ok(mut s) = s else { continue };
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let body = vec![b'Z'; 200_000];
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
        }
    });
    let table = table_from_entries(&[(Some("h"), "/api", up, "auth")]);
    let (_d, child, listen) = start_with_table(&table);
    let req = "GET /api?x=1 HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n";
    let mut stream = TcpStream::connect(&listen).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.write_all(req.as_bytes()).unwrap();
    let text = read_one_http_response(&mut stream);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    let idx = text.find("\r\n\r\n").expect("headers") + 4;
    let payload = text.as_bytes()[idx..].to_vec();
    let cl = text
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut full = payload;
    let mut tmp = [0u8; 65536];
    while full.len() < cl {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => full.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }
    assert_eq!(full.len(), 200_000, "truncated body len={}", full.len());
    assert!(full.iter().all(|&b| b == b'Z'));
    child.shutdown().unwrap();
}
