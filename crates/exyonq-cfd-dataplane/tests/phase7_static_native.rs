//! Phase-7 native CFD Static tests — real dataplane process + real filesystem.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use exyonq_cfd_control::{CfdChild, CfdLaunchConfig};
use exyonq_cfd_gen::{BackendKind, CompiledRoute, CompiledStaticPolicy, RouteTable};
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
    let child = CfdChild::start(&cfg).expect("start");
    child.publish_routes(2, table).expect("publish g2 routes");
    thread::sleep(Duration::from_millis(250));
    (dir, child, listen)
}

fn http_exchange(addr: &str, req: &str, expect_body: bool) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    stream.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    if !expect_body {
                        break;
                    }
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
    (status, text)
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

#[test]
fn native_static_get_head_index_and_rejects() {
    let docroot = tempdir().unwrap();
    std::fs::write(docroot.path().join("app.css"), b"body{}").unwrap();
    std::fs::write(docroot.path().join("index.html"), b"home").unwrap();
    std::fs::write(docroot.path().join(".env"), b"secret").unwrap();
    std::fs::write(docroot.path().join("index.php"), b"<?php echo 1;").unwrap();
    let table = static_table(docroot.path());
    let (_gen_dir, child, listen) = start_with_table(&table);

    let (st, body) = http_exchange(
        &listen,
        "GET /static/app.css HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n",
        true,
    );
    assert_eq!(st, 200, "{body}");
    assert!(body.contains("Content-Length: 6"), "{body}");
    assert!(
        body.contains("Content-Type: text/css; charset=utf-8"),
        "{body}"
    );
    assert!(body.ends_with("body{}"), "{body}");

    let (st, head) = http_exchange(
        &listen,
        "HEAD /static/app.css HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n",
        false,
    );
    assert_eq!(st, 200, "{head}");
    assert!(head.contains("Content-Length: 6"), "{head}");
    assert!(!head.ends_with("body{}"), "{head}");

    let (st, body) = http_exchange(
        &listen,
        "GET /static/app.css HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\n\r\nDATA",
        true,
    );
    assert_eq!(st, 200, "{body}");
    assert!(body.contains("Connection: close"), "{body}");
    assert!(body.ends_with("body{}"), "{body}");

    let (st, body) = http_exchange(
        &listen,
        "GET /static/ HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n",
        true,
    );
    assert_eq!(st, 200, "{body}");
    assert!(body.ends_with("home"), "{body}");

    for (path, expected) in [
        ("/static/.env", 403),
        ("/static/index.php", 403),
        ("/static/%2e%2e/secret.txt", 403),
        ("/static/missing.txt", 404),
    ] {
        let req = format!("GET {path} HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n");
        let (st, body) = http_exchange(&listen, &req, true);
        assert_eq!(st, expected, "path={path} body={body}");
    }

    let (st, body) = http_exchange(
        &listen,
        "POST /static/app.css HTTP/1.1\r\nHost: h\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        true,
    );
    assert_eq!(st, 405, "{body}");

    // Sequential keepalive on one TCP connection (not pipelining).
    let mut stream = TcpStream::connect(&listen).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    stream
        .write_all(b"GET /static/app.css HTTP/1.1\r\nHost: h\r\nConnection: keep-alive\r\n\r\n")
        .unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.ends_with(b"body{}") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let first = String::from_utf8_lossy(&buf);
    assert!(first.contains("HTTP/1.1 200 "), "{first}");
    assert!(first.contains("body{}"), "{first}");
    stream
        .write_all(b"GET /static/app.css HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n")
        .unwrap();
    buf.clear();
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.ends_with(b"body{}") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let second = String::from_utf8_lossy(&buf);
    assert!(second.contains("HTTP/1.1 200 "), "{second}");
    assert!(second.contains("body{}"), "{second}");

    child.shutdown().unwrap();
}

#[test]
fn static_proxy_isolation_and_generation_reload() {
    let docroot_a = tempdir().unwrap();
    let docroot_b = tempdir().unwrap();
    std::fs::write(docroot_a.path().join("a.txt"), b"AAA").unwrap();
    std::fs::write(docroot_b.path().join("b.txt"), b"BBB").unwrap();

    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let up_addr = upstream.local_addr().unwrap();
    let proxy_hits = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let hits = proxy_hits.clone();
    thread::spawn(move || {
        while let Ok((mut s, _)) = upstream.accept() {
            hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ =
                s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });

    let table_a = RouteTable {
        upstreams: vec![exyonq_cfd_gen::CompiledUpstream {
            id: 1,
            connect: up_addr,
            authority_host: "h".into(),
        }],
        fcgi_pools: Vec::new(),
        static_policies: vec![CompiledStaticPolicy {
            id: 3,
            document_root: docroot_a.path().to_string_lossy().into_owned(),
            flags: 0,
            index: vec![],
        }],
        routes: vec![
            CompiledRoute {
                route_id: 1,
                host: Some("h".into()),
                path: "/static".into(),
                backend_kind: BackendKind::Static,
                backend_id: 3,
            },
            CompiledRoute {
                route_id: 2,
                host: Some("h".into()),
                path: "/api".into(),
                backend_kind: BackendKind::Proxy,
                backend_id: 1,
            },
        ],
    };
    let (_gen_dir, child, listen) = start_with_table(&table_a);

    let (st, body) = http_exchange(
        &listen,
        "GET /static/a.txt HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n",
        true,
    );
    assert_eq!(st, 200, "{body}");
    assert!(body.ends_with("AAA"), "{body}");

    let (st, body) = http_exchange(
        &listen,
        "GET /static/missing.txt HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n",
        true,
    );
    assert_eq!(st, 404, "{body}");
    assert_eq!(
        proxy_hits.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "static miss must not fall through to proxy"
    );

    let (st, body) = http_exchange(
        &listen,
        "GET /api/x HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n",
        true,
    );
    assert_eq!(st, 200, "{body}");
    assert!(body.ends_with("ok"), "{body}");
    assert!(proxy_hits.load(std::sync::atomic::Ordering::Relaxed) >= 1);

    let table_b = RouteTable {
        upstreams: table_a.upstreams.clone(),
        fcgi_pools: Vec::new(),
        static_policies: vec![CompiledStaticPolicy {
            id: 3,
            document_root: docroot_b.path().to_string_lossy().into_owned(),
            flags: 0,
            index: vec![],
        }],
        routes: table_a.routes.clone(),
    };
    child.publish_routes(3, &table_b).expect("reload");
    thread::sleep(Duration::from_millis(300));

    let (st, body) = http_exchange(
        &listen,
        "GET /static/b.txt HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n",
        true,
    );
    assert_eq!(st, 200, "{body}");
    assert!(body.ends_with("BBB"), "{body}");

    // Invalid publish (relative docroot) must fail closed; previous gen stays active.
    let bad = RouteTable {
        upstreams: table_b.upstreams.clone(),
        fcgi_pools: Vec::new(),
        static_policies: vec![CompiledStaticPolicy {
            id: 3,
            document_root: "relative-root".into(),
            flags: 0,
            index: vec![],
        }],
        routes: table_b.routes.clone(),
    };
    assert!(child.publish_routes(4, &bad).is_err());
    let (st, body) = http_exchange(
        &listen,
        "GET /static/b.txt HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n",
        true,
    );
    assert_eq!(st, 200, "{body}");
    assert!(body.ends_with("BBB"), "{body}");

    child.shutdown().unwrap();
}
