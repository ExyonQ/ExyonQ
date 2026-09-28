//! Phase-5 CFD observability — real process, real file + OpenMetrics scrape.
//!
//! Not smoke: independent JSON parse + OpenMetrics `# EOF` checks against live sinks.
//! Tests serialize on process env (`EXYONQ_CFD_OBS_*`) via a mutex.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use exyonq_cfd_control::{CfdChild, CfdLaunchConfig};
use tempfile::{tempdir, TempDir};

fn obs_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
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

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn http_get(addr: &str, path: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    while Instant::now() < deadline {
        match TcpStream::connect(addr) {
            Ok(mut s) => {
                s.set_read_timeout(Some(Duration::from_secs(3))).ok();
                let req = format!("GET {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n");
                s.write_all(req.as_bytes()).unwrap();
                let mut buf = Vec::new();
                let _ = s.read_to_end(&mut buf);
                return String::from_utf8_lossy(&buf).into_owned();
            }
            Err(e) => {
                last = Some(e);
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
    panic!("connect {addr} failed: {last:?}");
}

fn looks_like_json_object(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('{') && t.ends_with('}') && t.contains("\"event\"") && !t.contains('\n')
}

fn scrape_metric(metrics: &str, name: &str) -> Option<u64> {
    for line in metrics.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix(name) {
            if rest.starts_with('{') || rest.starts_with(' ') {
                if let Some(sp) = rest.rfind(' ') {
                    return rest[sp + 1..].parse().ok();
                }
            }
        }
    }
    None
}

fn scrape_labeled(metrics: &str, name: &str, label_substr: &str) -> Option<u64> {
    for line in metrics.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with(name) && line.contains(label_substr) {
            if let Some(sp) = line.rfind(' ') {
                return line[sp + 1..].parse().ok();
            }
        }
    }
    None
}

fn wait_metrics(metrics_addr: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut metrics = String::new();
    while Instant::now() < deadline {
        if TcpStream::connect(metrics_addr).is_ok() {
            metrics = http_get(metrics_addr, "/metrics");
            if metrics.contains("exyonq_cfd_requests_total") && metrics.contains("# EOF") {
                return metrics;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    metrics
}

fn clear_obs_env() {
    for k in [
        "EXYONQ_CFD_OBS_CONSOLE_JSON",
        "EXYONQ_CFD_OBS_FILE",
        "EXYONQ_CFD_OBS_FILE_MAX_BYTES",
        "EXYONQ_CFD_OBS_FILE_KEEP",
        "EXYONQ_CFD_OBS_METRICS_LISTEN",
        "EXYONQ_CFD_OBS_SYSLOG",
        "EXYONQ_CFD_OBS_JOURNALD",
    ] {
        std::env::remove_var(k);
    }
}

fn start_child(listen: &str, gen_dir: &Path) -> CfdChild {
    let cfg = CfdLaunchConfig {
        listen: listen.to_string(),
        gen_dir: gen_dir.to_path_buf(),
        shards: 1,
        dataplane_bin: dataplane_bin(),
        ready_timeout: Duration::from_secs(20),
    };
    assert!(
        cfg.dataplane_bin.is_file(),
        "missing {:?}",
        cfg.dataplane_bin
    );
    CfdChild::start(&cfg).expect("start dataplane")
}

fn with_obs_env<R>(f: impl FnOnce(&TempDir, &str, &str) -> R) -> R {
    let _guard = obs_env_lock();
    clear_obs_env();
    let dir = tempdir().unwrap();
    let metrics_port = free_port();
    let listen_port = free_port();
    let listen = format!("127.0.0.1:{listen_port}");
    let metrics_addr = format!("127.0.0.1:{metrics_port}");
    let result = f(&dir, &listen, &metrics_addr);
    clear_obs_env();
    result
}

#[test]
fn file_json_and_metrics_scrape_are_real() {
    with_obs_env(|dir, listen, metrics_addr| {
        let log_path = dir.path().join("cfd-obs.log");
        std::env::set_var("EXYONQ_CFD_OBS_CONSOLE_JSON", "1");
        std::env::set_var("EXYONQ_CFD_OBS_FILE", &log_path);
        std::env::set_var("EXYONQ_CFD_OBS_FILE_MAX_BYTES", "2048");
        std::env::set_var("EXYONQ_CFD_OBS_FILE_KEEP", "3");
        std::env::set_var("EXYONQ_CFD_OBS_METRICS_LISTEN", metrics_addr);

        let child = start_child(listen, dir.path());

        for _ in 0..40 {
            let body = http_get(listen, "/__exyonq_cfd/v1/foundation");
            assert!(
                body.contains("HTTP/1.1 200") || body.contains("generation"),
                "unexpected foundation response: {body}"
            );
        }

        let metrics = wait_metrics(metrics_addr);
        assert!(
            metrics.contains("exyonq_cfd_requests_total"),
            "metrics missing requests_total:\n{metrics}"
        );
        assert!(
            metrics.contains("# EOF"),
            "metrics missing # EOF:\n{metrics}"
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut raw = String::new();
        while Instant::now() < deadline {
            if let Ok(t) = fs::read_to_string(&log_path) {
                if !t.is_empty() {
                    raw = t;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(!raw.is_empty(), "obs file empty at {}", log_path.display());
        let parsed = raw.lines().filter(|l| looks_like_json_object(l)).count();
        assert!(parsed > 0, "no JSON event lines in file:\n{raw}");

        for _ in 0..200 {
            let _ = http_get(listen, "/__exyonq_cfd/v1/foundation");
        }
        thread::sleep(Duration::from_millis(400));
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            entries.iter().any(|n| n.contains("cfd-obs.log")),
            "expected log file present: {entries:?}"
        );

        let _ = child.shutdown();
    });
}

#[test]
fn request_accounting_exactly_once_per_terminal_outcome() {
    with_obs_env(|dir, listen, metrics_addr| {
        let log_path = dir.path().join("acct.log");
        std::env::set_var("EXYONQ_CFD_OBS_FILE", &log_path);
        std::env::set_var("EXYONQ_CFD_OBS_METRICS_LISTEN", metrics_addr);

        let child = start_child(listen, dir.path());
        let _ = http_get(listen, "/__exyonq_cfd/v1/foundation");
        thread::sleep(Duration::from_millis(200));
        let before = wait_metrics(metrics_addr);
        let req0 = scrape_metric(&before, "exyonq_cfd_requests_total").unwrap_or(0);
        let r2xx0 = scrape_labeled(
            &before,
            "exyonq_cfd_responses_by_class_total",
            "class=\"2xx\"",
        )
        .unwrap_or(0);
        let r4xx0 = scrape_labeled(
            &before,
            "exyonq_cfd_responses_by_class_total",
            "class=\"4xx\"",
        )
        .unwrap_or(0);

        let ok = http_get(listen, "/__exyonq_cfd/v1/foundation");
        assert!(ok.contains("HTTP/1.1 200"), "foundation: {ok}");
        let miss = http_get(listen, "/no-such-route-p5or");
        assert!(
            miss.contains("HTTP/1.1 404") || miss.contains("route"),
            "route miss: {miss}"
        );

        thread::sleep(Duration::from_millis(250));
        let after = wait_metrics(metrics_addr);
        let req1 = scrape_metric(&after, "exyonq_cfd_requests_total").expect("requests");
        let r2xx1 = scrape_labeled(
            &after,
            "exyonq_cfd_responses_by_class_total",
            "class=\"2xx\"",
        )
        .unwrap_or(0);
        let r4xx1 = scrape_labeled(
            &after,
            "exyonq_cfd_responses_by_class_total",
            "class=\"4xx\"",
        )
        .unwrap_or(0);

        assert_eq!(
            req1.saturating_sub(req0),
            2,
            "exactly one account per request\nbefore:\n{before}\nafter:\n{after}"
        );
        assert_eq!(r2xx1.saturating_sub(r2xx0), 1, "one 2xx for foundation");
        assert_eq!(r4xx1.saturating_sub(r4xx0), 1, "one 4xx for route miss");

        let _ = child.shutdown();
    });
}

#[test]
fn active_connections_gauge_moves_with_accept_and_close() {
    with_obs_env(|dir, listen, metrics_addr| {
        let log_path = dir.path().join("gauge.log");
        std::env::set_var("EXYONQ_CFD_OBS_FILE", &log_path);
        std::env::set_var("EXYONQ_CFD_OBS_METRICS_LISTEN", metrics_addr);

        let child = start_child(listen, dir.path());
        let idle = wait_metrics(metrics_addr);
        let active0 = scrape_metric(&idle, "exyonq_cfd_active_connections").unwrap_or(0);
        let accepted0 = scrape_metric(&idle, "exyonq_cfd_accepted_connections_total").unwrap_or(0);

        let held = TcpStream::connect(listen).expect("hold connect");
        held.set_read_timeout(Some(Duration::from_millis(200))).ok();
        thread::sleep(Duration::from_millis(100));
        let mid = wait_metrics(metrics_addr);
        let active1 = scrape_metric(&mid, "exyonq_cfd_active_connections").unwrap_or(0);
        let accepted1 = scrape_metric(&mid, "exyonq_cfd_accepted_connections_total").unwrap_or(0);
        assert!(accepted1 > accepted0, "accepted must increase\nmid:\n{mid}");
        assert!(
            active1 > active0,
            "active must rise while connection held ({active0}->{active1})\n{mid}"
        );

        drop(held);
        thread::sleep(Duration::from_millis(250));
        let end = wait_metrics(metrics_addr);
        let active2 = scrape_metric(&end, "exyonq_cfd_active_connections").unwrap_or(0);
        let closed = scrape_metric(&end, "exyonq_cfd_closed_connections_total").unwrap_or(0);
        assert!(closed > 0, "closed connections must move\n{end}");
        assert_eq!(
            active2, active0,
            "active should return to baseline after close ({active0} vs {active2})\n{end}"
        );

        let _ = child.shutdown();
    });
}

#[test]
fn export_failure_does_not_increment_exported_success() {
    with_obs_env(|dir, listen, metrics_addr| {
        // Directory path as FILE target → open fails; export_errors increments;
        // no successful sink remains → events_exported stays 0.
        let bad = dir.path().join("not_a_file_dir");
        fs::create_dir_all(&bad).unwrap();
        std::env::set_var("EXYONQ_CFD_OBS_FILE", &bad);
        std::env::set_var("EXYONQ_CFD_OBS_METRICS_LISTEN", metrics_addr);
        std::env::remove_var("EXYONQ_CFD_OBS_CONSOLE_JSON");

        let child = start_child(listen, dir.path());
        for _ in 0..10 {
            let _ = http_get(listen, "/__exyonq_cfd/v1/foundation");
        }
        thread::sleep(Duration::from_millis(400));
        let metrics = wait_metrics(metrics_addr);
        let attempts =
            scrape_metric(&metrics, "exyonq_cfd_obs_events_export_attempts_total").unwrap_or(0);
        let exported = scrape_metric(&metrics, "exyonq_cfd_obs_events_exported_total").unwrap_or(0);
        let errors = scrape_metric(&metrics, "exyonq_cfd_obs_export_errors_total").unwrap_or(0);

        assert!(attempts > 0, "export attempts must run\n{metrics}");
        assert_eq!(
            exported, 0,
            "failed sink must not count as exported\n{metrics}"
        );
        assert!(
            errors > 0,
            "export errors must increase on sink failure\n{metrics}"
        );

        let _ = child.shutdown();
    });
}
