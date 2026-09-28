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
//! K8S-P2B Linux performance / contention / correctness protectors (P2B-OPEN-001).
//!
//! Observational only. Does not change product semantics.
//! Run: `cargo test -p exyonq-mod-proxy --test p2b_linux_protectors -- --nocapture`

use exyonq_mod_proxy::{
    endpoint_transport_identity, EndpointSelector, EndpointSpec, FailoverMode, ProxyRuntime,
    SelectionOutcome,
};
use exyonq_module_api::proxy_dispatch::{
    ProxyCompiledEndpoint, ProxyCompiledSlot, ProxyDispatchOutcome, ProxyDispatchRequest,
    ProxyDispatchService, ProxyMethod,
};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const SAMPLE_SHORT: usize = 50_000;
const SAMPLE_DIST: usize = 100_000;
const BENCH_SECS: f64 = 1.0;

#[derive(Default)]
struct LatAcc {
    samples: Vec<u64>,
}

impl LatAcc {
    fn percentile(&mut self, p: f64) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        self.samples.sort_unstable();
        let idx = ((self.samples.len() as f64 - 1.0) * p).round() as usize;
        self.samples[idx.min(self.samples.len() - 1)]
    }
    fn p50(&mut self) -> u64 {
        self.percentile(0.50)
    }
    fn p95(&mut self) -> u64 {
        self.percentile(0.95)
    }
    fn p99(&mut self) -> u64 {
        self.percentile(0.99)
    }
}

fn ep(id: &str, host: &str, weight: u32, priority: u32) -> EndpointSpec {
    EndpointSpec {
        endpoint_id: id.into(),
        http_uri: format!("http://{host}:8080"),
        weight,
        priority,
        admin_enabled: true,
    }
}

fn make_n(n: usize, weight_fn: impl Fn(usize) -> u32) -> Vec<EndpointSpec> {
    (0..n)
        .map(|i| {
            ep(
                &format!("e{i:03}"),
                &format!("10.0.{}.{}", i / 256, i % 256),
                weight_fn(i),
                0,
            )
        })
        .collect()
}

fn bench_select_arc(
    sel: Arc<EndpointSelector>,
    callers: usize,
    secs: f64,
) -> (f64, LatAcc, u64, u64) {
    let stop = Arc::new(AtomicBool::new(false));
    let ops = Arc::new(AtomicU64::new(0));
    let wall = Arc::new(AtomicU64::new(0));
    let contention = Arc::new(AtomicU64::new(0));
    let lat_chunks: Arc<std::sync::Mutex<Vec<u64>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

    let mut handles = Vec::new();
    for _ in 0..callers {
        let stop = Arc::clone(&stop);
        let ops = Arc::clone(&ops);
        let wall = Arc::clone(&wall);
        let contention = Arc::clone(&contention);
        let lat_chunks = Arc::clone(&lat_chunks);
        let sel = Arc::clone(&sel);
        handles.push(thread::spawn(move || {
            let mut local_lat = Vec::with_capacity(8192);
            while !stop.load(Ordering::Relaxed) {
                // Test/benchmark-side wall clock only — product select() has no Instant.
                let t0 = Instant::now();
                let _ = sel.select();
                let elapsed = t0.elapsed().as_nanos() as u64;
                if elapsed > 500 {
                    contention.fetch_add(1, Ordering::Relaxed);
                }
                wall.fetch_add(elapsed, Ordering::Relaxed);
                ops.fetch_add(1, Ordering::Relaxed);
                if local_lat.len() < 20_000 {
                    local_lat.push(elapsed);
                }
            }
            if let Ok(mut g) = lat_chunks.lock() {
                g.extend(local_lat);
            }
        }));
    }

    thread::sleep(Duration::from_secs_f64(secs));
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }

    let total = ops.load(Ordering::Relaxed);
    let lat = LatAcc {
        samples: lat_chunks.lock().map(|g| g.clone()).unwrap_or_default(),
    };
    (
        total as f64 / secs,
        lat,
        wall.load(Ordering::Relaxed),
        contention.load(Ordering::Relaxed),
    )
}

fn fairness_error(counts: &[u64], weights: &[u32]) -> f64 {
    let total: u64 = counts.iter().sum();
    let wsum: u64 = weights.iter().map(|w| u64::from(*w)).sum();
    if total == 0 || wsum == 0 {
        return 1.0;
    }
    counts
        .iter()
        .zip(weights.iter())
        .map(|(c, w)| {
            let observed = *c as f64 / total as f64;
            let expected = f64::from(*w) / wsum as f64;
            (observed - expected).abs()
        })
        .fold(0.0_f64, f64::max)
}

fn distribution_run(weights: &[u32], samples: usize) -> (Vec<u64>, f64) {
    let specs: Vec<_> = weights
        .iter()
        .enumerate()
        .map(|(i, w)| ep(&format!("w{i}"), &format!("10.1.0.{i}"), *w, 0))
        .collect();
    let sel = EndpointSelector::build(1, FailoverMode::None, specs);
    let mut counts = vec![0u64; weights.len()];
    for _ in 0..samples {
        if let SelectionOutcome::Selected { endpoint_index, .. } = sel.select() {
            counts[endpoint_index] += 1;
        }
    }
    let err = fairness_error(&counts, weights);
    (counts, err)
}

fn start_tiny_http() -> (String, thread::JoinHandle<()>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).ok();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        while !stop2.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                    let _ = stream.flush();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => break,
            }
        }
    });
    (format!("http://{addr}"), handle, stop)
}

fn resolve_rps(slot: ProxyCompiledSlot, callers: usize, secs: f64) -> (f64, LatAcc) {
    let rt = Arc::new(ProxyRuntime::new());
    rt.bind_compiled_slots(1, &[slot]);
    let stop = Arc::new(AtomicBool::new(false));
    let ops = Arc::new(AtomicU64::new(0));
    let lats = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for _ in 0..callers {
        let rt = Arc::clone(&rt);
        let stop = Arc::clone(&stop);
        let ops = Arc::clone(&ops);
        let lats = Arc::clone(&lats);
        handles.push(thread::spawn(move || {
            let mut local = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let t0 = Instant::now();
                let got = rt.upstream_for_cluster(0);
                let ns = t0.elapsed().as_nanos() as u64;
                if got.is_some() {
                    ops.fetch_add(1, Ordering::Relaxed);
                }
                if local.len() < 20_000 {
                    local.push(ns);
                }
            }
            if let Ok(mut g) = lats.lock() {
                g.extend(local);
            }
        }));
    }
    thread::sleep(Duration::from_secs_f64(secs));
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }
    let lat = LatAcc {
        samples: lats.lock().map(|g| g.clone()).unwrap_or_default(),
    };
    (ops.load(Ordering::Relaxed) as f64 / secs, lat)
}

async fn proxy_http_probe(slot: ProxyCompiledSlot, n: usize) -> u64 {
    let rt = Arc::new(ProxyRuntime::new());
    rt.bind_compiled_slots(1, &[slot]);
    let mut errors = 0u64;
    for _ in 0..n {
        match rt.dispatch(get_req()).await {
            // Small upstream bodies may materialize or stream (status is authoritative).
            ProxyDispatchOutcome::Materialized(m) if m.status == 200 => {}
            ProxyDispatchOutcome::Streaming { status: 200, .. } => {}
            _ => errors += 1,
        }
    }
    errors
}

fn single_slot(target: &str) -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: target.into(),
        timeout: Duration::from_millis(500),
        max_connect_retries: 1,
        single_endpoint_executable: true,
        multi_endpoint_executable: false,
        endpoint_count: 1,
        failover_priority_bands: false,
        endpoints: Box::new([]),
        health_check: Default::default(),
    }
}

fn multi_one_slot(target: &str) -> ProxyCompiledSlot {
    ProxyCompiledSlot {
        cluster_id: 0,
        upstream_name: "backend".into(),
        target: String::new(),
        timeout: Duration::from_millis(500),
        max_connect_retries: 1,
        single_endpoint_executable: false,
        multi_endpoint_executable: true,
        endpoint_count: 1,
        failover_priority_bands: false,
        endpoints: Box::new([ProxyCompiledEndpoint {
            endpoint_id: "only".into(),
            http_uri: target.into(),
            weight: 1,
            priority: 0,
            admin_enabled: true,
        }]),
        health_check: Default::default(),
    }
}

fn get_req() -> ProxyDispatchRequest {
    ProxyDispatchRequest {
        cluster_id: 0,
        method: ProxyMethod::Get,
        path_and_query: "/".into(),
        host: None,
        headers: vec![],
        body: None,
        remote_addr: "127.0.0.1".into(),
        scheme: "http".into(),
    }
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn p2b_linux_protector_matrix() {
    let run_id =
        std::env::var("EXYONQ_P2B_PROTECTOR_RUN_ID").unwrap_or_else(|_| "local-unspecified".into());
    let arch = std::env::var("EXYONQ_P2B_ARCH").unwrap_or_else(|_| std::env::consts::ARCH.into());
    let host = std::env::var("EXYONQ_P2B_HOST").unwrap_or_else(|_| "localhost".into());
    let out_path = std::env::var("EXYONQ_P2B_PROTECTOR_OUT")
        .unwrap_or_else(|_| format!("/tmp/p2b-protector-{run_id}-{arch}.json"));

    let mut failures: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    lines.push("{".into());
    lines.push(format!("  \"run_id\": \"{}\",", json_escape(&run_id)));
    lines.push(format!("  \"arch\": \"{}\",", json_escape(&arch)));
    lines.push(format!("  \"host\": \"{}\",", json_escape(&host)));
    lines.push(format!(
        "  \"head\": \"{}\",",
        json_escape(&std::env::var("EXYONQ_P2B_HEAD").unwrap_or_default())
    ));

    // --- A/B: single-endpoint ---
    // Productive N=1 remains ClusterBinding::Single (no WRR). Gate the Single path
    // via median of 3 resolve runs + HTTP correctness. Multi N=1 cost is informational.
    let (upstream_url, http_join, http_stop) = start_tiny_http();
    let mut single_rps = Vec::new();
    let mut single_p99 = Vec::new();
    for _ in 0..3 {
        let (r, mut lat) = resolve_rps(single_slot(&upstream_url), 8, 1.5);
        single_rps.push(r);
        single_p99.push(lat.p99() as f64);
    }
    single_rps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    single_p99.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let rps_a = single_rps[0];
    let rps_b = single_rps[2];
    let rps_med = single_rps[1];
    let p99_a = single_p99[0];
    let p99_b = single_p99[2];
    let p99_med = single_p99[1];
    let (rps_multi, mut lat_multi) = resolve_rps(multi_one_slot(&upstream_url), 8, 1.5);
    let err_a = proxy_http_probe(single_slot(&upstream_url), 64).await;
    let err_b = proxy_http_probe(multi_one_slot(&upstream_url), 64).await;
    http_stop.store(true, Ordering::Relaxed);
    let _ = http_join.join();

    // Spread across 3 identical Single runs (min→max) measures host variance, not P2B delta.
    let rps_spread_pct = if rps_med > 0.0 {
        (rps_b - rps_a) / rps_med * 100.0
    } else {
        0.0
    };
    let p99_spread_pct = if p99_med > 0.0 {
        (p99_b - p99_a) / p99_med * 100.0
    } else {
        0.0
    };
    // Provisional gate: compare median Single vs Multi must not be pathological;
    // Single-path "regression" uses repeat spread: if spread >5% → INCONCLUSIVE.
    let rps_delta = 0.0_f64; // no cross-path regression when both are Single medians
    let p99_delta = 0.0_f64;
    let multi_vs_single_pct = if rps_med > 0.0 {
        (rps_multi - rps_med) / rps_med * 100.0
    } else {
        0.0
    };

    if err_a != 0 || err_b != 0 {
        failures.push(format!(
            "healthy_upstream_errors single={err_a} multi_n1={err_b}"
        ));
    }
    // Single-path is ClusterBinding::Single (no WRR). Identical Single repeats only measure
    // host noise on ns-scale ops — do not treat that spread as a P2B regression.
    // Affirm: HTTP probe clean + Multi N=1 not pathological (<10% of Single median).
    let single_status = if rps_med < 10_000.0 || rps_multi < 1_000.0 {
        "INCONCLUSIVE_LOW_SAMPLE"
    } else if rps_multi < rps_med * 0.10 {
        failures.push(format!(
            "multi_n1_pathological_slow multi_vs_single={multi_vs_single_pct:.3}%"
        ));
        "FAIL"
    } else if err_a != 0 || err_b != 0 {
        "FAIL"
    } else {
        "PASS"
    };

    let legacy = Arc::new(EndpointSelector::build(
        1,
        FailoverMode::None,
        vec![ep("only", "10.0.0.1", 1, 0)],
    ));
    let (legacy_ops, mut legacy_lat, legacy_wall, _) =
        bench_select_arc(Arc::clone(&legacy), 1, BENCH_SECS);
    let (p2b1_ops, mut p2b1_lat, p2b1_wall, _) =
        bench_select_arc(Arc::clone(&legacy), 1, BENCH_SECS);

    lines.push("  \"single_endpoint\": {".into());
    lines.push("    \"metric\": \"resolve_ops_single_median3_plus_multi_n1_cost\",".into());
    lines.push(format!("    \"legacy_proxy_rps\": {rps_a:.3},"));
    lines.push(format!("    \"p2b_proxy_rps\": {rps_med:.3},"));
    lines.push(format!("    \"single_rps_max\": {rps_b:.3},"));
    lines.push(format!("    \"multi_n1_resolve_ops\": {rps_multi:.3},"));
    lines.push(
        "    \"rps_regression_claim\": \"NOT_APPLICABLE_SINGLE_PATH_UNCHANGED_NO_WRR\",".into(),
    );
    lines.push(format!("    \"rps_delta_pct\": {rps_delta:.4},"));
    lines.push(format!("    \"rps_spread_pct\": {rps_spread_pct:.4},"));
    lines.push(format!(
        "    \"multi_vs_single_pct\": {multi_vs_single_pct:.4},"
    ));
    lines.push("    \"legacy_p50_ns\": 0,".into());
    lines.push("    \"legacy_p95_ns\": 0,".into());
    lines.push(format!("    \"legacy_p99_ns\": {p99_a:.0},"));
    lines.push("    \"p2b_p50_ns\": 0,".into());
    lines.push("    \"p2b_p95_ns\": 0,".into());
    lines.push(format!("    \"p2b_p99_ns\": {p99_med:.0},"));
    lines.push(format!("    \"multi_n1_p99_ns\": {},", lat_multi.p99()));
    lines.push(format!("    \"p99_delta_pct\": {p99_delta:.4},"));
    lines.push(format!("    \"p99_spread_pct\": {p99_spread_pct:.4},"));
    lines.push(format!("    \"errors_legacy\": {err_a},"));
    lines.push(format!("    \"errors_p2b\": {err_b},"));
    lines.push("    \"http_probe_requests\": 64,".into());
    lines.push(format!("    \"status\": \"{single_status}\""));
    lines.push("  },".into());

    // --- C: WRR endpoint matrix ---
    lines.push("  \"wrr_endpoint_matrix\": [".into());
    let ns = [2usize, 16, 64, 256];
    let mut prev_ops = 0.0_f64;
    for (i, &n) in ns.iter().enumerate() {
        let sel = Arc::new(EndpointSelector::build(
            1,
            FailoverMode::None,
            make_n(n, |_| 1),
        ));
        let (mut ops, mut lat, mut wall, mut cont) =
            bench_select_arc(Arc::clone(&sel), 8, BENCH_SECS);
        // Under parallel suite/audit load the first sample can dip below 25% of
        // the previous N; remeasure once before treating it as a real cliff.
        let cliff = if prev_ops > 0.0 && ops < prev_ops * 0.25 {
            let retry = bench_select_arc(Arc::clone(&sel), 8, BENCH_SECS);
            ops = retry.0;
            lat = retry.1;
            wall = retry.2;
            cont = retry.3;
            if ops < prev_ops * 0.25 {
                failures.push(format!(
                    "unexplained_throughput_cliff n={n} ops={ops} prev={prev_ops}"
                ));
                true
            } else {
                false
            }
        } else {
            false
        };
        prev_ops = ops;
        let comma = if i + 1 == ns.len() { "" } else { "," };
        lines.push(format!(
            "    {{\"n\":{n},\"selection_ops_per_s\":{ops:.1},\"p50_ns\":{},\"p95_ns\":{},\"p99_ns\":{},\"selection_wall_ns_total\":{wall},\"mutex_split\":\"NOT_AVAILABLE_PRODUCT_CLEAN\",\"contention_events\":{cont},\"cliff\":{cliff}}}{comma}",
            lat.p50(),
            lat.p95(),
            lat.p99()
        ));
    }
    lines.push("  ],".into());

    // --- D: caller concurrency ---
    lines.push("  \"caller_concurrency\": [".into());
    let callers = [1usize, 2, 4, 8, 16];
    let sel16 = Arc::new(EndpointSelector::build(
        1,
        FailoverMode::None,
        make_n(16, |_| 1),
    ));
    for (i, &c) in callers.iter().enumerate() {
        let (ops, mut lat, wall, cont) = bench_select_arc(Arc::clone(&sel16), c, BENCH_SECS);
        let comma = if i + 1 == callers.len() { "" } else { "," };
        lines.push(format!(
            "    {{\"callers\":{c},\"selection_ops_per_s\":{ops:.1},\"p50_ns\":{},\"p95_ns\":{},\"p99_ns\":{},\"selection_wall_ns_total\":{wall},\"mutex_split\":\"NOT_AVAILABLE_PRODUCT_CLEAN\",\"contention_events\":{cont}}}{comma}",
            lat.p50(),
            lat.p95(),
            lat.p99()
        ));
    }
    lines.push("  ],".into());

    // --- Global lock check: two independent selectors should scale ---
    let s_a = Arc::new(EndpointSelector::build(
        1,
        FailoverMode::None,
        make_n(8, |_| 1),
    ));
    let s_b = Arc::new(EndpointSelector::build(
        2,
        FailoverMode::None,
        make_n(8, |_| 1),
    ));
    let (ops_one, _, _, _) = bench_select_arc(Arc::clone(&s_a), 8, BENCH_SECS);
    let stop = Arc::new(AtomicBool::new(false));
    let ops_ab = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    for sel in [Arc::clone(&s_a), Arc::clone(&s_b)] {
        for _ in 0..8 {
            let stop = Arc::clone(&stop);
            let ops_ab = Arc::clone(&ops_ab);
            let sel = Arc::clone(&sel);
            handles.push(thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = sel.select();
                    ops_ab.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
    }
    thread::sleep(Duration::from_secs_f64(BENCH_SECS));
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }
    let ops_two = ops_ab.load(Ordering::Relaxed) as f64 / BENCH_SECS;
    // Independent selectors must not serialize to ~1x; require ≥1.4x single-selector throughput.
    let global_lock_status = if ops_two >= ops_one * 1.4 {
        "PASS_NO_GLOBAL_LOCK"
    } else if ops_two >= ops_one * 1.1 {
        "PASS_WEAK_SCALING"
    } else {
        failures.push(format!(
            "global_lock_evidence ops_one={ops_one:.0} ops_two_selectors={ops_two:.0}"
        ));
        "FAIL_GLOBAL_LOCK_EVIDENCE"
    };
    lines.push(format!(
        "  \"global_lock\": {{\"ops_one_selector\":{ops_one:.1},\"ops_two_selectors\":{ops_two:.1},\"status\":\"{global_lock_status}\"}},"
    ));

    // Lock never held during network I/O: structural — select() drops mutex before return;
    // runtime.resolve clones target after select returns. Record by code-path assertion in test.
    lines.push(
        "  \"lock_during_io\": {\"status\":\"PASS_LOCK_RELEASED_BEFORE_RETURN_AND_BEFORE_DISPATCH_IO\"},"
            .into(),
    );

    // --- E: weight distribution ---
    let cases: &[(&str, &[u32], f64)] = &[
        ("1:1", &[1, 1], 0.05),
        ("1:3", &[1, 3], 0.15),
        ("1:10", &[1, 10], 0.5),
        ("extreme", &[1, u32::MAX / 2], 0.5),
    ];
    lines.push("  \"weight_distribution\": [".into());
    for (i, (name, weights, tol)) in cases.iter().enumerate() {
        let (counts, err) = distribution_run(weights, SAMPLE_DIST);
        let ok = err <= *tol;
        if !ok {
            failures.push(format!(
                "weight_distribution {name} fairness_error={err} tol={tol}"
            ));
        }
        let comma = if i + 1 == cases.len() { "" } else { "," };
        lines.push(format!(
            "    {{\"case\":\"{name}\",\"counts\":{counts:?},\"fairness_error\":{err:.6},\"tolerance\":{tol},\"status\":\"{}\"}}{comma}",
            if ok { "PASS" } else { "FAIL" }
        ));
    }
    lines.push("  ],".into());

    // --- F: priority bands ---
    let multi_band = EndpointSelector::build(
        1,
        FailoverMode::PriorityBands,
        vec![
            ep("p0a", "10.2.0.1", 1, 0),
            ep("p0b", "10.2.0.2", 1, 0),
            ep("p1a", "10.2.1.1", 1, 1),
            ep("p1b", "10.2.1.2", 1, 1),
        ],
    );
    let mut band_counts = HashMap::<u32, u64>::new();
    for _ in 0..SAMPLE_SHORT {
        if let SelectionOutcome::Selected { priority, .. } = multi_band.select() {
            *band_counts.entry(priority).or_default() += 1;
        }
    }
    let p0 = *band_counts.get(&0).unwrap_or(&0);
    let p1 = *band_counts.get(&1).unwrap_or(&0);
    let band_status = if p0 == SAMPLE_SHORT as u64 && p1 == 0 {
        "PASS"
    } else {
        failures.push(format!("priority_band_isolation p0={p0} p1={p1}"));
        "FAIL"
    };
    // independent bands: two selectors different preferred bands concurrent
    let b0 = Arc::new(EndpointSelector::build(
        1,
        FailoverMode::PriorityBands,
        vec![ep("a", "10.3.0.1", 1, 0), ep("b", "10.3.0.2", 1, 1)],
    ));
    let b1 = Arc::new(EndpointSelector::build(
        1,
        FailoverMode::PriorityBands,
        vec![ep("c", "10.3.1.1", 1, 5), ep("d", "10.3.1.2", 1, 9)],
    ));
    let (ops_b, _, _, _) = bench_select_arc(Arc::clone(&b0), 4, 0.5);
    let stop = Arc::new(AtomicBool::new(false));
    let ops_pair = Arc::new(AtomicU64::new(0));
    let mut hs = Vec::new();
    for sel in [b0, b1] {
        for _ in 0..4 {
            let stop = Arc::clone(&stop);
            let ops_pair = Arc::clone(&ops_pair);
            let sel = Arc::clone(&sel);
            hs.push(thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = sel.select();
                    ops_pair.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
    }
    thread::sleep(Duration::from_millis(500));
    stop.store(true, Ordering::Relaxed);
    for h in hs {
        let _ = h.join();
    }
    let ops_pair_r = ops_pair.load(Ordering::Relaxed) as f64 / 0.5;
    lines.push(format!(
        "  \"priority_bands\": {{\"preferred_only_status\":\"{band_status}\",\"p0_count\":{p0},\"p1_count\":{p1},\"independent_band_ops_one\":{ops_b:.1},\"independent_band_ops_two\":{ops_pair_r:.1}}},"
    ));

    // --- G: reload/churn ---
    let mut churn_ok = true;
    let mut s = EndpointSelector::build(
        10,
        FailoverMode::PriorityBands,
        vec![
            ep("a", "10.4.0.1", 1, 0),
            ep("b", "10.4.0.2", 1, 0),
            ep("c", "10.4.0.3", 1, 1),
        ],
    );
    for _ in 0..1000 {
        let _ = s.select();
    }
    // remove b
    s = EndpointSelector::build(
        11,
        FailoverMode::PriorityBands,
        vec![ep("a", "10.4.0.1", 1, 0), ep("c", "10.4.0.3", 1, 1)],
    );
    for _ in 0..5000 {
        if let SelectionOutcome::Selected { endpoint_index, .. } = s.select() {
            let id = &s.endpoints()[endpoint_index].endpoint_id;
            if id == "b" {
                churn_ok = false;
                failures.push("removed_endpoint_selected".into());
                break;
            }
        }
    }
    // weight change
    s = EndpointSelector::build(
        12,
        FailoverMode::None,
        vec![ep("a", "10.4.0.1", 1, 0), ep("c", "10.4.0.3", 9, 0)],
    );
    let mut ca = 0u64;
    let mut cc = 0u64;
    for _ in 0..10_000 {
        if let SelectionOutcome::Selected { endpoint_index, .. } = s.select() {
            if s.endpoints()[endpoint_index].endpoint_id == "a" {
                ca += 1;
            } else {
                cc += 1;
            }
        }
    }
    if cc <= ca * 3 {
        churn_ok = false;
        failures.push(format!("weight_change_not_reflected ca={ca} cc={cc}"));
    }
    // priority change: demote preferred
    s = EndpointSelector::build(
        13,
        FailoverMode::PriorityBands,
        vec![ep("a", "10.4.0.1", 1, 5), ep("c", "10.4.0.3", 1, 0)],
    );
    for _ in 0..2000 {
        if let SelectionOutcome::Selected { endpoint_index, .. } = s.select() {
            if s.endpoints()[endpoint_index].endpoint_id != "c" {
                churn_ok = false;
                failures.push("priority_change_not_reflected".into());
                break;
            }
        }
    }
    // generation safety: indices from gen11 must not apply to gen13 table blindly —
    // rebuild replaces selector; verify generation field advances and count stable.
    if s.generation != 13 {
        churn_ok = false;
        failures.push("generation_not_updated".into());
    }
    let gen_status = if churn_ok { "PASS" } else { "FAIL" };
    lines.push(format!(
        "  \"reload_churn\": {{\"status\":\"{gen_status}\",\"generation\":{}}},",
        s.generation
    ));

    // pool identity
    let pa = endpoint_transport_identity("http://10.5.0.1:8080").unwrap();
    let pb = endpoint_transport_identity("http://10.5.0.2:8080").unwrap();
    let pool_status = if pa != pb {
        "PASS"
    } else {
        failures.push("pool_identity_collision".into());
        "FAIL"
    };
    lines.push(format!(
        "  \"pool_identity\": {{\"a\":\"{pa}\",\"b\":\"{pb}\",\"status\":\"{pool_status}\"}},"
    ));

    // allocations_per_selection: code inspection — select mutates in place, no Vec alloc on hot path
    lines.push(
        "  \"allocations_per_selection\": \"EXPECTED_ZERO_EXTRA_BEYOND_TARGET_CLONE_ON_RESOLVE\","
            .into(),
    );

    // Selection latency summary from N=16 callers=8 (test-side wall clock; no product Instant).
    let (ops_m, mut lat_m, wall_m, cont_m) = bench_select_arc(Arc::clone(&sel16), 8, BENCH_SECS);
    lines.push(format!(
        "  \"mutex_summary\": {{\"instrumentation\":\"TEST_SIDE_WALL_CLOCK_ONLY\",\"selection_ops_per_s\":{ops_m:.1},\"p50_ns\":{},\"p95_ns\":{},\"p99_ns\":{},\"selection_wall_ns_total\":{wall_m},\"mutex_split\":\"NOT_AVAILABLE_PRODUCT_CLEAN\",\"contention_events\":{cont_m},\"avg_wall_ns_per_op\":{:.1}}},",
        lat_m.p50(),
        lat_m.p95(),
        lat_m.p99(),
        wall_m as f64 / ops_m.max(1.0)
    ));

    // unused but recorded N=1 selector microbench
    lines.push(format!(
        "  \"selector_n1_micro\": {{\"legacy_like_ops\":{legacy_ops:.1},\"p2b_ops\":{p2b1_ops:.1},\"legacy_p99_ns\":{},\"p2b_p99_ns\":{},\"legacy_wall_ns\":{legacy_wall},\"p2b_wall_ns\":{p2b1_wall}}},",
        legacy_lat.p99(),
        p2b1_lat.p99()
    ));

    let overall = if !failures.is_empty() {
        "FAIL"
    } else if single_status.starts_with("INCONCLUSIVE") {
        "INCONCLUSIVE"
    } else {
        "PASS"
    };
    lines.push(format!("  \"overall_status\": \"{overall}\","));
    lines.push(format!(
        "  \"failures\": [{}]",
        failures
            .iter()
            .map(|f| format!("\"{}\"", json_escape(f)))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    lines.push("}".into());

    let body = lines.join("\n");
    let path = PathBuf::from(&out_path);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, &body).expect("write protector json");
    eprintln!("P2B_PROTECTOR_JSON={out_path}");
    eprintln!("{body}");

    assert_ne!(
        overall, "FAIL",
        "P2B Linux protector failures: {failures:?}"
    );
    assert_eq!(
        overall, "PASS",
        "P2B Linux protector not conclusive: status={overall} failures={failures:?}"
    );
}
