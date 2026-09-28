//! Phase-3 WAF + generation reload E2E through real exyonq-dataplane TCP.

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
use exyonq_cfd_gen::{GenDir, Generation};
use exyonq_waf::{MatchTarget, RuleInput};
use exyonq_waf_api::{WafAction, WafMode, WafPhase};
use tempfile::tempdir;

fn free_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn dataplane_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_exyonq-dataplane") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_BIN_EXE_exyonq-dataplane"))
}

fn upstream_echo(body: &'static [u8]) -> (SocketAddr, Arc<Mutex<u64>>) {
    let hits = Arc::new(Mutex::new(0u64));
    let hits2 = hits.clone();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    thread::spawn(move || {
        for stream in l.incoming().flatten() {
            let mut s = stream;
            let mut buf = [0u8; 8192];
            let _ = s.read(&mut buf);
            *hits2.lock().unwrap() += 1;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.write_all(body);
        }
    });
    (addr, hits)
}

fn leak_body(n: usize, b: u8) -> &'static [u8] {
    Box::leak(vec![b; n].into_boxed_slice())
}

fn http_get(addr: SocketAddr, host: &str, path: &str, extra_headers: &str) -> (u16, String) {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n",
        path = path,
        host = host,
        extra = extra_headers,
    );
    s.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|x| x.parse().ok())
        .unwrap_or(0);
    (status, text)
}

fn http_get_keepalive(s: &mut TcpStream, host: &str, path: &str) -> u16 {
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let (status, header_end, cl) = loop {
        match s.read(&mut tmp) {
            Ok(0) => return 0,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let end = end + 4;
                    let head = String::from_utf8_lossy(&buf[..end]);
                    let status = head
                        .split_whitespace()
                        .nth(1)
                        .and_then(|x| x.parse().ok())
                        .unwrap_or(0);
                    let cl = head
                        .lines()
                        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|line| line.split_once(':'))
                        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break (status, end, cl);
                }
                if buf.len() > 64 * 1024 {
                    return 0;
                }
            }
            Err(_) => return 0,
        }
    };
    while buf.len() < header_end + cl {
        match s.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }
    status
}

fn wait_ready(gen: &std::path::Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let gd = GenDir::new(gen);
    while Instant::now() < deadline {
        if let Ok(raw) = std::fs::read_to_string(gd.status_path()) {
            if raw.contains("READY") {
                return;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("dataplane not ready");
}

fn start_child(listen: SocketAddr, gen_dir: PathBuf) -> CfdChild {
    std::env::set_var("EXYONQ_COMPETITIVE_H1_DATAPLANE", "1");
    let cfg = CfdLaunchConfig {
        listen: listen.to_string(),
        gen_dir,
        shards: 1,
        dataplane_bin: dataplane_bin(),
        ready_timeout: Duration::from_secs(45),
    };
    CfdChild::start(&cfg).expect("start dataplane")
}

fn publish_waf(
    child: &CfdChild,
    gen_id: u64,
    table: &exyonq_cfd_gen::RouteTable,
    waf: &exyonq_waf::WafCompileInput,
) {
    let proj = projection_from_compile_input(waf).unwrap();
    let composite = CompositeProjection {
        routes: table.clone(),
        waf: Some(proj),
    };
    child.publish_composite(gen_id, &composite).unwrap();
    thread::sleep(Duration::from_millis(200));
}

#[test]
fn waf_allow_api_and_deny_path_before_upstream() {
    let (up, hits) = upstream_echo(leak_body(1024, b'a'));
    let dir = tempdir().unwrap();
    let listen = free_addr();
    let table = table_from_entries(&[(None, "/api", up, "upstream")]);
    GenDir::new(dir.path()).ensure().unwrap();

    let child = start_child(listen, dir.path().to_path_buf());
    wait_ready(dir.path(), Duration::from_secs(15));
    let waf = representative_phase3_waf_input();
    publish_waf(&child, 2, &table, &waf);

    let (st_ok, body_ok) = http_get(listen, "app.example", "/api/", "");
    assert_eq!(st_ok, 200, "allow /api/: {body_ok}");
    let before_deny = *hits.lock().unwrap();

    let (st_deny, body) = http_get(listen, "app.example", "/api/admin/secret", "");
    assert_eq!(st_deny, 403, "deny path: {body}");
    assert!(body.contains("waf denied"));
    assert_eq!(*hits.lock().unwrap(), before_deny, "DENY_BEFORE_UPSTREAM");

    let (st_hdr, _) = http_get(listen, "app.example", "/api/", "X-Attack: evil\r\n");
    assert_eq!(st_hdr, 403);

    let (st_host, _) = http_get(listen, "blocked.example", "/api/", "");
    assert_eq!(st_host, 403);

    drop(child);
}

#[test]
fn same_socket_waf_generation_g1_g2() {
    let (up, _) = upstream_echo(leak_body(1024, b'b'));
    let dir = tempdir().unwrap();
    let listen = free_addr();
    let table = table_from_entries(&[(None, "/api", up, "upstream")]);
    GenDir::new(dir.path()).ensure().unwrap();

    let child = start_child(listen, dir.path().to_path_buf());
    wait_ready(dir.path(), Duration::from_secs(15));

    let mut waf_off = representative_phase3_waf_input();
    waf_off.enabled = false;
    waf_off.mode = WafMode::Disabled;
    publish_waf(&child, 2, &table, &waf_off);

    let mut s = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert_eq!(http_get_keepalive(&mut s, "app.example", "/api/"), 200);

    let mut waf_on = representative_phase3_waf_input();
    waf_on.rulesets[0].rules.push(RuleInput {
        id: "CFD-RELOAD-DENY".into(),
        enabled: true,
        phases: vec![WafPhase::RequestHeaders],
        target: MatchTarget::Path,
        pattern: "reload-deny-marker".into(),
        action: WafAction::Block,
    });
    publish_waf(&child, 3, &table, &waf_on);

    assert_eq!(
        http_get_keepalive(&mut s, "app.example", "/api/reload-deny-marker"),
        403,
        "SAME_SOCKET_WAF_FRESHNESS"
    );

    publish_waf(&child, 4, &table, &waf_off);
    // Post-disable check does not require the same TCP socket: a generation
    // publish under load may drop idle keepalives. Reopen for the allow path.
    let mut s2 = TcpStream::connect_timeout(&listen, Duration::from_secs(2)).unwrap();
    s2.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert_eq!(http_get_keepalive(&mut s2, "app.example", "/api/"), 200);

    drop(child);
}

#[test]
fn corrupt_generation_keeps_prior() {
    let (up, _) = upstream_echo(leak_body(1024, b'c'));
    let dir = tempdir().unwrap();
    let listen = free_addr();
    let table = table_from_entries(&[(None, "/api", up, "upstream")]);
    GenDir::new(dir.path()).ensure().unwrap();

    let child = start_child(listen, dir.path().to_path_buf());
    wait_ready(dir.path(), Duration::from_secs(15));
    let waf = representative_phase3_waf_input();
    publish_waf(&child, 2, &table, &waf);
    assert_eq!(http_get(listen, "app.example", "/api/", "").0, 200);

    let nonsense = Generation::new(3, b"NOTAMAGIC_PAYLOAD_XXXX".to_vec()).unwrap();
    GenDir::new(dir.path()).publish(&nonsense).unwrap();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        http_get(listen, "app.example", "/api/", "").0,
        200,
        "corrupt gen must not wipe prior routes"
    );
    drop(child);
}
