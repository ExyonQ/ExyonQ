//! Repeated lifecycle resource stability for CFD foundations.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use exyonq_cfd_control::{CfdChild, CfdLaunchConfig};
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
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap().to_string();
    drop(l);
    a
}

fn count_fds(pid: u32) -> Option<usize> {
    let path = format!("/proc/{pid}/fd");
    if let Ok(rd) = fs::read_dir(&path) {
        return Some(rd.count());
    }
    let out = Command::new("lsof")
        .args(["-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).lines().skip(1).count())
}

#[test]
fn repeated_lifecycle_no_zombie_and_stable_fds() {
    let dir = tempdir().unwrap();
    let listen = free_listen();
    let bin = dataplane_bin();
    let mut fd_samples = Vec::new();
    for i in 0..5 {
        let cfg = CfdLaunchConfig {
            listen: listen.clone(),
            gen_dir: dir.path().to_path_buf(),
            shards: 2,
            dataplane_bin: bin.clone(),
            ready_timeout: Duration::from_secs(15),
        };
        let child = CfdChild::start(&cfg).unwrap_or_else(|e| panic!("cycle {i}: {e}"));
        let pid = child.child.id();
        let fds = count_fds(pid).expect("FD count must be available for stability oracle");
        fd_samples.push(fds);
        // Publish G bump each cycle.
        let g = exyonq_cfd_gen::Generation::new((i as u64) + 2, b"cycle".to_vec()).unwrap();
        child.publish_generation(&g).unwrap();
        thread::sleep(Duration::from_millis(150));
        child.shutdown().unwrap();
        thread::sleep(Duration::from_millis(100));
        // No zombie of that pid.
        let zombie = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "state="])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains('Z'))
            .unwrap_or(false);
        assert!(!zombie, "zombie pid {pid} after cycle {i}");
    }
    let min = *fd_samples.iter().min().unwrap_or(&0);
    let max = *fd_samples.iter().max().unwrap_or(&0);
    assert!(
        max.saturating_sub(min) <= 8,
        "FD growth across cycles samples={fd_samples:?}"
    );
}
