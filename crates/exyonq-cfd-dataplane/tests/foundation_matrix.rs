//! Phase-1 Competitive Frontier dataplane foundation matrix (real process).
//!
//! Not a smoke harness: spawns the real `exyonq-dataplane` binary built by cargo.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use exyonq_cfd_control::{
    assert_no_listen_collision, CfdChild, CfdLaunchConfig, CfdLaunchError, DataplaneStatus,
};
use exyonq_cfd_gen::{GenDir, Generation, SCHEMA_VERSION};
use tempfile::tempdir;

fn foundation_process_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn dataplane_bin() -> PathBuf {
    // Prefer CARGO_BIN_EXE when running as integration test of this package.
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_exyonq-dataplane") {
        return PathBuf::from(p);
    }
    // Fallback: target/debug sibling
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push("debug");
    p.push("exyonq-dataplane");
    p
}

fn free_listen() -> String {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = l.local_addr().expect("addr");
    drop(l);
    addr.to_string()
}

fn http_get(addr: &str, path: &str) -> (u16, String, String) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).expect("write");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read");
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let gen = text
        .lines()
        .find(|l| {
            l.to_ascii_lowercase()
                .starts_with("x-exyonq-cfd-generation:")
        })
        .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
        .unwrap_or_default();
    (status, text, gen)
}

fn start_child(shards: usize) -> (tempfile::TempDir, CfdChild, String) {
    let dir = tempdir().unwrap();
    let listen = free_listen();
    let cfg = CfdLaunchConfig {
        listen: listen.clone(),
        gen_dir: dir.path().to_path_buf(),
        shards,
        dataplane_bin: dataplane_bin(),
        ready_timeout: Duration::from_secs(15),
    };
    assert!(
        cfg.dataplane_bin.is_file(),
        "missing binary {:?}",
        cfg.dataplane_bin
    );
    let child = CfdChild::start(&cfg).expect("start dataplane");
    (dir, child, listen)
}

#[test]
fn feature_gate_off_by_default() {
    let _guard = foundation_process_guard();
    // Unset in this process for the assertion — env may be polluted; check helper semantics.
    let was = std::env::var_os("EXYONQ_COMPETITIVE_H1_DATAPLANE");
    std::env::remove_var("EXYONQ_COMPETITIVE_H1_DATAPLANE");
    assert!(!exyonq_cfd_control::dataplane_enabled());
    if let Some(v) = was {
        std::env::set_var("EXYONQ_COMPETITIVE_H1_DATAPLANE", v);
    }
}

#[test]
fn binary_missing_fails_closed() {
    let _guard = foundation_process_guard();
    let dir = tempdir().unwrap();
    let cfg = CfdLaunchConfig {
        listen: "127.0.0.1:1".into(),
        gen_dir: dir.path().to_path_buf(),
        shards: 1,
        dataplane_bin: PathBuf::from("/nonexistent/exyonq-dataplane"),
        ready_timeout: Duration::from_secs(1),
    };
    let err = match CfdChild::start(&cfg) {
        Ok(_) => panic!("expected BinaryNotFound"),
        Err(e) => e,
    };
    assert!(matches!(err, CfdLaunchError::BinaryNotFound));
}

#[test]
fn start_ready_foundation_probe_and_route_miss_404() {
    let _guard = foundation_process_guard();
    let (_dir, child, listen) = start_child(2);
    let (st, body, gen) = http_get(&listen, "/__exyonq_cfd/v1/foundation");
    assert_eq!(st, 200, "body={body}");
    assert_eq!(gen, "1");
    assert!(body.contains("generation_id=1"));

    let (st2, body2, _) = http_get(&listen, "/api/anything");
    assert_eq!(st2, 404, "body={body2}");
    assert!(body2.contains("route miss") || body2.contains("404"));

    child.shutdown().unwrap();
}

#[test]
fn generation_g1_to_g2_observed() {
    let _guard = foundation_process_guard();
    let (_dir, child, listen) = start_child(1);
    let (_, _, gen1) = http_get(&listen, "/__exyonq_cfd/v1/foundation");
    assert_eq!(gen1, "1");

    let g2 = Generation::from_route_table(2, &exyonq_cfd_gen::RouteTable::default()).unwrap();
    child.publish_generation(&g2).unwrap();

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut seen = None;
    while Instant::now() < deadline {
        let (_, _, gen) = http_get(&listen, "/__exyonq_cfd/v1/foundation");
        if gen == "2" {
            seen = Some(gen);
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(seen.as_deref(), Some("2"));
    child.shutdown().unwrap();
}

#[test]
fn corrupt_generation_rejected_keeps_prior() {
    let _guard = foundation_process_guard();
    let (_dir, child, listen) = start_child(1);
    // Corrupt the blob in place (bypass GenDir::publish).
    let path = child.gen_dir.generation_path();
    std::fs::write(&path, b"EXYQCFD1\x00\x00\x00\x01garbage").unwrap();
    thread::sleep(Duration::from_millis(250));
    let (_, _, gen) = http_get(&listen, "/__exyonq_cfd/v1/foundation");
    assert_eq!(gen, "1", "corrupt must not replace live generation");
    child.shutdown().unwrap();
}

#[test]
fn incompatible_schema_handshake_rejected_at_start() {
    let _guard = foundation_process_guard();
    let dir = tempdir().unwrap();
    let listen = free_listen();
    let gd = GenDir::new(dir.path());
    gd.ensure().unwrap();
    let g = Generation::new(1, b"x".to_vec()).unwrap();
    let mut bytes = g.encode().unwrap();
    bytes[8..12].copy_from_slice(&99u32.to_le_bytes());
    let body_end = bytes.len() - 4;
    let c = exyonq_cfd_gen::crc32(&bytes[..body_end]);
    bytes[body_end..].copy_from_slice(&c.to_le_bytes());
    assert!(matches!(
        Generation::decode(&bytes),
        Err(exyonq_cfd_gen::GenError::IncompatibleSchema { got: 99 })
    ));
    std::fs::write(gd.generation_path(), &bytes).unwrap();

    let mut child = Command::new(dataplane_bin())
        .arg("serve")
        .arg("--listen")
        .arg(&listen)
        .arg("--gen-dir")
        .arg(dir.path())
        .arg("--shards")
        .arg("1")
        .arg("--schema-version")
        .arg(SCHEMA_VERSION.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut exited = None;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            exited = Some(status);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    let status = exited.expect("dataplane must exit on incompatible gen");
    assert!(!status.success());
}

#[test]
fn schema_version_cli_mismatch_exits() {
    let _guard = foundation_process_guard();
    let dir = tempdir().unwrap();
    let listen = free_listen();
    let gd = GenDir::new(dir.path());
    gd.ensure().unwrap();
    gd.publish(&Generation::new(1, b"ok".to_vec()).unwrap())
        .unwrap();
    let out = Command::new(dataplane_bin())
        .arg("serve")
        .arg("--listen")
        .arg(&listen)
        .arg("--gen-dir")
        .arg(dir.path())
        .arg("--shards")
        .arg("1")
        .arg("--schema-version")
        .arg("99")
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn second_non_reuseport_bind_fails_while_dataplane_owns_port() {
    let _guard = foundation_process_guard();
    let (_dir, child, listen) = start_child(1);
    let addr: SocketAddr = listen.parse().unwrap();
    // Plain TcpListener (no SO_REUSEPORT) must not steal competitive H1 accept domain.
    let bind_err = TcpListener::bind(addr);
    assert!(
        bind_err.is_err(),
        "dual-accept must be rejected; got {:?}",
        bind_err
    );
    child.shutdown().unwrap();
}

#[test]
fn listen_collision_helper() {
    assert!(assert_no_listen_collision("127.0.0.1:9", &["127.0.0.1:9".into()]).is_err());
}

#[test]
fn clean_shutdown_and_restart_cycle() {
    let _guard = foundation_process_guard();
    let dir = tempdir().unwrap();
    let listen = free_listen();
    let bin = dataplane_bin();
    for i in 0..3 {
        let cfg = CfdLaunchConfig {
            listen: listen.clone(),
            gen_dir: dir.path().to_path_buf(),
            shards: 1,
            dataplane_bin: bin.clone(),
            ready_timeout: Duration::from_secs(15),
        };
        let child = CfdChild::start(&cfg).unwrap_or_else(|e| panic!("cycle {i} start: {e}"));
        let st = exyonq_cfd_control::wait_status(dir.path()).unwrap();
        assert!(matches!(st, DataplaneStatus::Ready { .. }));
        child.shutdown().unwrap();
        // Ensure port released.
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn second_dataplane_process_rejected_by_exclusive_lock() {
    let _guard = foundation_process_guard();
    let dir = tempdir().unwrap();
    let listen = free_listen();
    let bin = dataplane_bin();
    let cfg = CfdLaunchConfig {
        listen: listen.clone(),
        gen_dir: dir.path().to_path_buf(),
        shards: 1,
        dataplane_bin: bin.clone(),
        ready_timeout: Duration::from_secs(15),
    };
    let child = CfdChild::start(&cfg).expect("first owner");
    // Second process same gen-dir must fail closed (even with REUSEPORT).
    let cfg2 = CfdLaunchConfig {
        listen: free_listen(), // different port — lock is on gen-dir
        gen_dir: dir.path().to_path_buf(),
        shards: 1,
        dataplane_bin: bin,
        ready_timeout: Duration::from_secs(5),
    };
    let err = match CfdChild::start(&cfg2) {
        Ok(_) => panic!("second dataplane must not become READY"),
        Err(e) => e,
    };
    match err {
        CfdLaunchError::ExitedBeforeReady(_)
        | CfdLaunchError::ReadyTimeout(_)
        | CfdLaunchError::InvalidStatus(_) => {}
        other => panic!("unexpected err {other}"),
    }
    child.shutdown().unwrap();
}

#[test]
fn ready_implies_accept_works() {
    let _guard = foundation_process_guard();
    let (_dir, child, listen) = start_child(2);
    // If READY raced ahead of bind, connect would fail — must succeed.
    let (st, _, _) = http_get(&listen, "/__exyonq_cfd/v1/foundation");
    assert_eq!(st, 200);
    child.shutdown().unwrap();
}

#[test]
fn generation_downgrade_rejected() {
    let _guard = foundation_process_guard();
    let (_dir, child, listen) = start_child(1);
    // Applied-generation header tracks shard-local projection id; payload must be
    // a valid CFDRT002/CFDCOM01 (opaque "g5" bytes never apply → header stays 1).
    let g5 = Generation::from_route_table(5, &exyonq_cfd_gen::RouteTable::default()).unwrap();
    child.publish_generation(&g5).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw5 = false;
    while Instant::now() < deadline {
        let (_, _, gen) = http_get(&listen, "/__exyonq_cfd/v1/foundation");
        if gen == "5" {
            saw5 = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(
        saw5,
        "must observe applied generation 5 before downgrade probe"
    );
    // Attempt downgrade via GenDir publish (bypass control monotonic checks).
    let low = Generation::from_route_table(2, &exyonq_cfd_gen::RouteTable::default()).unwrap();
    child.gen_dir.publish(&low).unwrap();
    thread::sleep(Duration::from_millis(300));
    let (_, _, gen) = http_get(&listen, "/__exyonq_cfd/v1/foundation");
    assert_eq!(gen, "5", "downgrade must be rejected");
    child.shutdown().unwrap();
}

#[cfg(unix)]
#[test]
fn gen_dir_permissions_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let gd = GenDir::new(dir.path());
    gd.ensure().unwrap();
    let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}
