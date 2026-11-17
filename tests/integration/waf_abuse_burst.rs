/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! `[waf.abuse]` quota accounting over real HTTP.
//!
//! One client request must consume exactly one quota unit. The wire
//! fast-path pre-checks (which exist so the raw request-target is inspected
//! before Hyper drops a `#fragment`) ran the header gate and then handed the
//! request to the Hyper path, which ran the same gate again. With `burst = 1`
//! that challenged the *first* request, which is what failed 6 of the 13 OL-04
//! E2E checks in CI.
//!
//! These tests install the gate the server must consult (a test double for the
//! pipeline contract, labelled as such) and assert the HTTP result plus the
//! number of judgements per request. The production token bucket
//! (`modules/ratelimit`) is exercised by `scripts/e2e/ol04-challenge-e2e.sh`
//! with the real binary.

mod bootstrap;

use bytes::Bytes;
use exyonq_core::AppConfig;
use exyonq_core::{WafAbuseGate, WafDecision, WafPhase};
use http_body_util::{BodyExt, Full};
use hyper::{Request, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::net::{IpAddr, SocketAddr, TcpListener as StdTcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

/// The abuse booking is process-global; the tests below share one gate state.
static GLOBAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct CountingGate {
    calls: AtomicUsize,
    /// Deny from the second judgement of the same client onwards (a `burst = 1`
    /// bucket for the pipeline contract).
    burst: usize,
}

impl CountingGate {
    fn new(burst: usize) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            burst,
        }
    }

    /// Hand the burst back to the next test. The server and its gate are
    /// process-wide (see `harness`), so each test starts from a full bucket.
    fn reset(&self) {
        self.calls.store(0, Ordering::SeqCst);
    }
}

impl WafAbuseGate for CountingGate {
    fn check(&self, _client_ip: IpAddr, _route_id: Option<&str>) -> Option<WafDecision> {
        let seen = self.calls.fetch_add(1, Ordering::SeqCst);
        if seen < self.burst {
            return None;
        }
        Some(WafDecision::abuse_rate_limit(WafPhase::RequestHeaders))
    }
}

/// Process-lifetime static root.
///
/// The static dispatch service is a process-global singleton: it binds the
/// configured roots once (`bootstrap::ensure_integration_modules` is
/// `call_once`) and a second server in the same process keeps resolving
/// against the first registration. Two tests therefore cannot own two
/// different roots, so they share this one and it outlives both: the earlier
/// version returned a per-test `TempDir` that was dropped before the server
/// answered, and the resulting `404 not found` was the test deleting its own
/// fixture, not the server losing `/health`.
fn shared_root() -> &'static std::path::Path {
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.html"), "burst-ok").expect("index");
        dir
    })
    .path()
}

/// Wait until the listener accepts, without sleeping and without issuing any
/// request the quota assertions could count.
async fn wait_listening(listen: SocketAddr) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match tokio::net::TcpStream::connect(listen).await {
            Ok(_) => return,
            Err(err) if tokio::time::Instant::now() < deadline => {
                let _ = err;
                tokio::task::yield_now().await;
            }
            Err(err) => panic!("server never listened on {listen}: {err}"),
        }
    }
}

fn ephemeral_addr() -> SocketAddr {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

fn abuse_config(root: &std::path::Path) -> AppConfig {
    let toml = format!(
        r#"
config_version = 1

[waf]
enabled = true
mode = "block"

[waf.abuse]
mode = "rate_limit"
requests_per_second = 1
burst = 1

[[server]]
listen = "127.0.0.1:0"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = {root:?}
index = "index.html"
"#,
        root = root.display().to_string()
    );
    AppConfig::parse_str(&toml).expect("abuse config parses")
}

/// The server, its gate and its static root are process-wide.
///
/// The abuse gate binding and the static dispatch service are process-global
/// (`call_once`), and `std::thread` keeps the server alive across the
/// per-`#[tokio::test]` runtimes, so one server serves both tests and the gate's
/// counter is reset between them. Each test therefore starts from a full
/// `burst = 1` bucket whatever the order, and both contracts stay strict.
struct Harness {
    listen: SocketAddr,
    gate: Arc<CountingGate>,
}

async fn harness() -> Harness {
    static HARNESS: tokio::sync::OnceCell<Harness> = tokio::sync::OnceCell::const_new();
    HARNESS
        .get_or_init(|| async {
            bootstrap::ensure_integration_modules();
            exyonq_core::set_waf_enforce(true);
            let gate = Arc::new(CountingGate::new(1));
            exyonq_core::install_waf_runtime_binding(exyonq_core::WafRuntimeBinding {
                engine: exyonq_core::current_waf_engine(),
                abuse: Some(gate.clone()),
                enforce: true,
                wire_inspection_active: true,
            });

            let listen = ephemeral_addr();
            let mut config = abuse_config(shared_root());
            config.servers[0].listen = listen.to_string();
            // The server outlives the per-test runtime: `#[tokio::test]` builds
            // and drops one runtime per test, so a task spawned there dies with
            // it. A dedicated thread with its own runtime keeps the single
            // in-process server alive for the whole binary.
            std::thread::Builder::new()
                .name("waf-abuse-server".into())
                .spawn(move || {
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                        .expect("server runtime");
                    rt.block_on(async move {
                        exyonq_core::server::run_on(listen, config)
                            .await
                            .expect("server runs");
                    });
                })
                .expect("server thread");
            wait_listening(listen).await;
            Harness { listen, gate }
        })
        .await;
    let harness = HARNESS.get().expect("harness initialised");
    harness.gate.reset();
    Harness {
        listen: harness.listen,
        gate: Arc::clone(&harness.gate),
    }
}

fn client() -> Client<HttpConnector, Full<Bytes>> {
    Client::builder(TokioExecutor::new()).build_http()
}

async fn get(
    client: &Client<HttpConnector, Full<Bytes>>,
    listen: SocketAddr,
    path: &str,
) -> (StatusCode, Bytes) {
    let uri: Uri = format!("http://{listen}{path}").parse().unwrap();
    // Keep-alive is part of the contract this exercises: the client pools the
    // connection, so a probe and the request after it travel on the same socket.
    // The wire used to answer the second head with a terminal `404 not found`
    // instead of handing the connection back to Hyper.
    let req = Request::get(uri).body(Full::new(Bytes::new())).unwrap();
    let response = timeout(Duration::from_secs(5), client.request(req))
        .await
        .expect("timed out")
        .expect("request failed");
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body)
}

/// Send one request on an existing connection and read one full response.
///
/// The probes test drives its own socket so "same connection" is a property of
/// the test, not of a client pool's reuse heuristics.
async fn request_on(stream: &mut tokio::net::TcpStream, target: &str) -> (u16, Vec<u8>) {
    let request = format!("GET {target} HTTP/1.1\r\nHost: exyonq.local\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut buf: Vec<u8> = Vec::new();
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_string();
            let status: u16 = head
                .split_whitespace()
                .nth(1)
                .expect("status code")
                .parse()
                .expect("numeric status");
            let body_start = end + 4;
            let mut body_len = 0usize;
            for line in head.lines() {
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    body_len = value.trim().parse().expect("content-length");
                }
            }
            while buf.len() < body_start + body_len {
                let mut chunk = [0u8; 1024];
                let n = stream.read(&mut chunk).await.expect("read body");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let end_body = (body_start + body_len).min(buf.len());
            return (status, buf[body_start..end_body].to_vec());
        }
        let mut chunk = [0u8; 1024];
        let n = stream.read(&mut chunk).await.expect("read response head");
        if n == 0 {
            panic!(
                "connection closed before a response: {:?}",
                String::from_utf8_lossy(&buf)
            );
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// A client with `burst = 1` gets its first request served and the second one
/// rejected. Judging the first request twice rejects it, which is the defect.
#[tokio::test]
async fn one_request_consumes_exactly_one_quota_unit() {
    let _guard = GLOBAL.lock().await;
    let Harness { listen, gate } = harness().await;
    let client = client();

    let (status, body) = get(&client, listen, "/index.html").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the first request owns the burst; body={:?}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(body.as_ref(), b"burst-ok");
    assert_eq!(
        gate.calls.load(Ordering::SeqCst),
        1,
        "one request must be judged once"
    );

    let (status, _) = get(&client, listen, "/index.html").await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "the second is over budget"
    );
    assert_eq!(gate.calls.load(Ordering::SeqCst), 2);
}

/// Liveness probes are exempt from rate limits, so they must not spend the
/// client's quota either: a monitored deployment would otherwise lock out its
/// own users.
///
/// The `404` this test used to report was the test deleting its own fixture: a
/// per-test `TempDir` was returned from the spawn helper and dropped before the
/// server answered, so the static root no longer existed. With a
/// process-lifetime root the probe is answered `200` and the burst survives it,
/// on Darwin and on native Linux alike.
#[tokio::test]
async fn probes_do_not_spend_the_quota() {
    let _guard = GLOBAL.lock().await;
    let Harness { listen, gate } = harness().await;

    // One socket for the whole sequence: the wire answers the probe and hands
    // the same connection to Hyper for the file.
    let mut stream = tokio::net::TcpStream::connect(listen)
        .await
        .expect("connect keep-alive socket");

    for _ in 0..3 {
        let (status, body) = request_on(&mut stream, "/health").await;
        assert_eq!(
            status,
            StatusCode::OK.as_u16(),
            "a probe must be answered by the handler, body={:?}",
            String::from_utf8_lossy(&body)
        );
    }
    assert_eq!(
        gate.calls.load(Ordering::SeqCst),
        0,
        "probes must not reach the abuse gate"
    );

    let (status, body) = request_on(&mut stream, "/index.html").await;
    assert_eq!(
        status,
        StatusCode::OK.as_u16(),
        "the burst is still intact on the same connection, body={:?}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(body.as_slice(), b"burst-ok");
}
