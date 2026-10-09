//! WC7D — real Redis process restart + data-loss safety (Docker).
//!
//! Ensures a durable named container + volume, then:
//! 1) kill process / restart with data preserved
//! 2) kill / restart / FLUSHALL → local generation never rolls back
//!
//! Soft-skips when Docker itself is unavailable, or when the Redis image
//! cannot be pulled (Docker Hub rate limit or registry timeout). A failed
//! `docker run` for any other reason still fails the test.
//!
//! KF-P16-009: container/volume/port are unique per process (PID + run stamp).
//! Never uses the historical fixed name `exyonq-wc7b2-redis`. Cleanup removes
//! only this process's resources.

use exyonq_cache_redis::{
    EventSigningKeys, RedisCoordConfig, RedisCoordSecrets, RedisCoordinationProvider, ReplayPolicy,
};
use exyonq_module_api::{GenerationStore, InvalidationSubscriber};
use std::net::TcpListener;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const REDIS_IMAGE: &str = "redis:7.2.7-alpine";

struct RedisNs {
    container: String,
    volume: String,
    port: u16,
    url: String,
}

fn docker_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Serialize WC7D tests in this binary so kill/start/cleanup cannot interleave
/// on the shared per-process Redis namespace.
fn suite_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn redis_ns() -> &'static RedisNs {
    static NS: OnceLock<RedisNs> = OnceLock::new();
    NS.get_or_init(|| {
        if let Ok(url) = std::env::var("EXYONQ_REDIS_COORD_URL") {
            if !url.is_empty() {
                // External Redis: do not manage Docker lifecycle for this process.
                return RedisNs {
                    container: String::new(),
                    volume: String::new(),
                    port: 0,
                    url,
                };
            }
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let id = format!("{}-{}", std::process::id(), stamp);
        let port = std::env::var("EXYONQ_WC7D_REDIS_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(ephemeral_port);
        RedisNs {
            container: std::env::var("EXYONQ_WC7D_REDIS_CONTAINER")
                .unwrap_or_else(|_| format!("exyonq-wc7d-{id}")),
            volume: std::env::var("EXYONQ_WC7D_REDIS_VOLUME")
                .unwrap_or_else(|_| format!("exyonq-wc7d-vol-{id}")),
            port,
            url: format!("redis://127.0.0.1:{port}/"),
        }
    })
}

fn ephemeral_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
        .unwrap_or(16379)
}

fn redis_url() -> String {
    redis_ns().url.clone()
}

fn container() -> String {
    redis_ns().container.clone()
}

fn volume() -> String {
    redis_ns().volume.clone()
}

fn manages_docker() -> bool {
    !redis_ns().container.is_empty()
}

/// Drops only this process's container/volume on scope exit (success or panic).
struct RedisResourceGuard;

impl Drop for RedisResourceGuard {
    fn drop(&mut self) {
        if !manages_docker() {
            return;
        }
        let _lock = docker_lock().lock().unwrap_or_else(|e| e.into_inner());
        let c = container();
        let v = volume();
        let _ = docker(&["rm", "-f", &c]);
        let _ = docker(&["volume", "rm", "-f", &v]);
    }
}

fn docker_available() -> bool {
    // Bound wait: a wedged Docker Desktop makes `docker info` hang forever and
    // stalls the whole `cargo test --workspace` (observed Darwin hang at WC7D).
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let ok = Command::new("docker")
            .args(["info"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let _ = tx.send(ok);
    });
    rx.recv_timeout(Duration::from_secs(8)).unwrap_or(false)
}

fn docker(args: &[&str]) -> std::process::Output {
    // `docker pull` against Docker Hub can sit forever on a stalled registry
    // connection. An unbounded wait stalls the whole nextest run.
    let mut child = match Command::new("docker")
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => return docker_failed(&format!("docker failed to start: {err}")),
    };
    let start = Instant::now();
    let limit = Duration::from_secs(25);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .unwrap_or_else(|err| docker_failed(&format!("docker output failed: {err}")));
            }
            Ok(None) if start.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return docker_failed("docker timed out");
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(err) => return docker_failed(&format!("docker wait failed: {err}")),
        }
    }
}

fn docker_failed(message: &str) -> std::process::Output {
    use std::os::unix::process::ExitStatusExt;
    std::process::Output {
        status: std::process::ExitStatus::from_raw(1),
        stdout: Vec::new(),
        stderr: message.as_bytes().to_vec(),
    }
}

fn docker_ok(args: &[&str]) -> bool {
    docker(args).status.success()
}

fn redis_up() -> bool {
    let Ok(c) = redis::Client::open(redis_url().as_str()) else {
        return false;
    };
    c.get_connection_with_timeout(Duration::from_millis(400))
        .ok()
        .and_then(|mut conn| redis::cmd("PING").query::<String>(&mut conn).ok())
        .is_some()
}

fn wait_redis(timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if redis_up() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn container_exists() -> bool {
    docker_ok(&["inspect", &container()])
}

enum RedisEnsure {
    Ready,
    ImageUnavailable,
}

fn redis_image_pull_blocked(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    // A name conflict is our container, not a registry failure.
    if lower.contains("conflict") || lower.contains("already in use") {
        return false;
    }
    lower.contains("toomanyrequests")
        || lower.contains("pull rate limit")
        || lower.contains("unable to find image")
        || lower.contains("registry-1.docker.io")
        || lower.contains("client.timeout")
        || lower.contains("request canceled")
        || lower.contains("timed out")
}

fn ensure_redis_container() -> RedisEnsure {
    if !manages_docker() {
        assert!(
            wait_redis(Duration::from_secs(5)),
            "external EXYONQ_REDIS_COORD_URL not ready"
        );
        return RedisEnsure::Ready;
    }
    let _lock = docker_lock().lock().unwrap_or_else(|e| e.into_inner());
    let c = container();
    let v = volume();
    let port = redis_ns().port.to_string();
    let publish = format!("{port}:6379");
    let vol_mount = format!("{v}:/data");
    if !container_exists() {
        let _ = docker(&["volume", "create", &v]);
        let out = docker(&[
            "run",
            "-d",
            "--name",
            &c,
            "-p",
            &publish,
            "-v",
            &vol_mount,
            REDIS_IMAGE,
            "redis-server",
            "--save",
            "60",
            "1",
            "--appendonly",
            "yes",
        ]);
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            // Stale same-name residue from an earlier crash of *this* namespace: reclaim ours only.
            if err.contains("Conflict") || err.contains("already in use") {
                let _ = docker(&["rm", "-f", &c]);
                let out2 = docker(&[
                    "run",
                    "-d",
                    "--name",
                    &c,
                    "-p",
                    &publish,
                    "-v",
                    &vol_mount,
                    REDIS_IMAGE,
                    "redis-server",
                    "--save",
                    "60",
                    "1",
                    "--appendonly",
                    "yes",
                ]);
                let err2 = String::from_utf8_lossy(&out2.stderr);
                if !out2.status.success() {
                    if redis_image_pull_blocked(&err2) {
                        return RedisEnsure::ImageUnavailable;
                    }
                    panic!("docker run failed after reclaiming our container: {err2}");
                }
            } else if redis_image_pull_blocked(&err) {
                return RedisEnsure::ImageUnavailable;
            } else {
                panic!("docker run failed: {err}");
            }
        }
    } else if !docker_ok(&["inspect", "-f", "{{.State.Running}}", &c])
        || !String::from_utf8_lossy(&docker(&["inspect", "-f", "{{.State.Running}}", &c]).stdout)
            .contains("true")
    {
        assert!(docker_ok(&["start", &c]), "docker start existing container");
    }
    assert!(
        wait_redis(Duration::from_secs(30)),
        "Redis not ready after ensure"
    );
    RedisEnsure::Ready
}

fn kill_redis_process() {
    let _lock = docker_lock().lock().unwrap_or_else(|e| e.into_inner());
    let c = container();
    let out = docker(&["kill", "--signal=KILL", &c]);
    assert!(
        out.status.success(),
        "docker kill failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if !redis_up() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("Redis still up after kill");
}

fn start_redis_process() {
    let _lock = docker_lock().lock().unwrap_or_else(|e| e.into_inner());
    let c = container();
    if container_exists() {
        let out = docker(&["start", &c]);
        if !out.status.success() {
            let _ = docker(&["rm", "-f", &c]);
            drop(_lock);
            ensure_redis_container();
            return;
        }
    } else {
        drop(_lock);
        ensure_redis_container();
        return;
    }
    assert!(
        wait_redis(Duration::from_secs(30)),
        "Redis did not become ready after start"
    );
}

fn redis_fsync_persistence() {
    let client = redis::Client::open(redis_url().as_str()).unwrap();
    let mut conn = client
        .get_connection_with_timeout(Duration::from_secs(2))
        .unwrap();
    let _: String = redis::cmd("SAVE").query(&mut conn).unwrap();
}

fn flush_all_coordination() {
    let client = redis::Client::open(redis_url().as_str()).unwrap();
    let mut conn = client
        .get_connection_with_timeout(Duration::from_secs(2))
        .unwrap();
    let _: () = redis::cmd("FLUSHALL").query(&mut conn).unwrap();
}

fn keys() -> EventSigningKeys {
    EventSigningKeys::for_tests("k1", b"wc7d-real-restart-key-32bytes!!")
}

fn open(ns: &str, node: &str) -> RedisCoordinationProvider {
    RedisCoordinationProvider::connect(
        RedisCoordConfig {
            endpoint: redis_url(),
            deployment_id: "wc7d-real".into(),
            namespace: ns.into(),
            node_id: node.into(),
            connect_timeout: Duration::from_millis(800),
            command_timeout: Duration::from_millis(800),
            reconnect_min_backoff: Duration::from_millis(50),
            reconnect_max_backoff: Duration::from_millis(500),
            stream_maxlen: 1000,
            max_pending_events: 32,
            max_event_bytes: 8192,
            invalidation_enabled: true,
            generation_enabled: true,
            known_site_ids: vec![1],
            replay: ReplayPolicy {
                max_age_ms: 600_000,
                max_future_skew_ms: 60_000,
            },
        },
        RedisCoordSecrets::default(),
        keys(),
    )
    .unwrap()
}

#[test]
fn wc7d_real_redis_process_restart_preserve_data() {
    let _suite = suite_lock().lock().unwrap_or_else(|e| e.into_inner());
    if !docker_available() && manages_docker() {
        eprintln!("wc7d_real_redis_process_restart_preserve_data: soft-skip (no docker)");
        return;
    }
    let _cleanup = RedisResourceGuard;
    if matches!(ensure_redis_container(), RedisEnsure::ImageUnavailable) {
        eprintln!(
            "wc7d_real_redis_process_restart_preserve_data: soft-skip (redis image unavailable)"
        );
        return;
    }
    let ns = format!("exyonq:fpc:v1:wc7d:rst:{}", std::process::id());
    let a = open(&ns, "a");
    let b = open(&ns, "b");
    b.start_subscriber().unwrap();
    assert_eq!(a.advance_generation(1, 42).unwrap(), 42);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let _ = b.reconcile_known_sites();
        if b.get_generation(1).unwrap() >= 42 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(b.get_generation(1).unwrap() >= 42);

    redis_fsync_persistence();
    kill_redis_process();
    assert!(!redis_up(), "Redis must be down after kill");
    assert_eq!(
        a.get_generation(1).unwrap(),
        42,
        "local gen survives Redis process death"
    );

    start_redis_process();
    let _ = a.reconcile_known_sites();
    let a2 = open(&ns, "a2");
    assert!(a2.reconcile_known_sites().is_ok());
    assert!(
        a2.get_generation(1).unwrap() >= 42,
        "preserved Redis gen must remain after process restart (AOF/volume)"
    );
    b.stop();
}

#[test]
fn wc7d_redis_data_loss_local_generation_never_rolls_back() {
    let _suite = suite_lock().lock().unwrap_or_else(|e| e.into_inner());
    if !docker_available() && manages_docker() {
        eprintln!("wc7d_redis_data_loss: soft-skip (no docker)");
        return;
    }
    let _cleanup = RedisResourceGuard;
    if matches!(ensure_redis_container(), RedisEnsure::ImageUnavailable) {
        eprintln!("wc7d_redis_data_loss: soft-skip (redis image unavailable)");
        return;
    }
    let ns = format!("exyonq:fpc:v1:wc7d:dl:{}", std::process::id());
    let a = open(&ns, "a");
    let b = open(&ns, "b");
    b.start_subscriber().unwrap();
    assert_eq!(a.advance_generation(1, 77).unwrap(), 77);
    let local_a = a.get_generation(1).unwrap();
    assert!(local_a >= 77);

    kill_redis_process();
    assert_eq!(a.get_generation(1).unwrap(), local_a);
    start_redis_process();
    flush_all_coordination();

    let _ = a.reconcile_known_sites();
    assert!(
        a.get_generation(1).unwrap() >= local_a,
        "local generation must never roll back after Redis data loss"
    );
    assert!(a.advance_generation(1, local_a + 1).unwrap() > local_a);
    assert!(a.get_generation(1).unwrap() > local_a);
    b.stop();
}
