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
//! Worker/pool memory gauges on the `/metrics` cold path.
//!
//! ExyonQ accept workers are OS threads in one process, not independent
//! processes. Linux exposes one resident set per address space, so a thread
//! has no private RSS to report. The per-worker series therefore carry the
//! containing process's real `VmRSS` and the explicit `scope="shared"` label;
//! the pool series carries the same aggregate. This avoids inventing a
//! divided or otherwise synthetic per-thread value.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Mirrors the server worker ceiling; keeps label cardinality bounded even if
/// a caller ever passes an unexpected count.
const MAX_METRIC_WORKERS: usize = 16;

static CONFIGURED_WORKERS: AtomicUsize = AtomicUsize::new(0);

/// Set the actual accept-worker count after the server resolves its geometry.
pub(crate) fn set_server_worker_count(workers: usize) {
    CONFIGURED_WORKERS.store(workers.min(MAX_METRIC_WORKERS), Ordering::Release);
}

/// Append the worker/pool memory series to the Prometheus exposition.
///
/// No series is emitted before the server has configured workers or when the
/// kernel does not expose `/proc/self/status` (for example on macOS).
pub fn append_worker_memory_prometheus(out: &mut String) {
    let workers = CONFIGURED_WORKERS.load(Ordering::Acquire);
    if workers == 0 {
        return;
    }
    let Some(rss_bytes) = process_rss_bytes() else {
        return;
    };
    out.push_str(&render_worker_memory_prometheus(workers, rss_bytes));
}

fn render_worker_memory_prometheus(workers: usize, rss_bytes: u64) -> String {
    let workers = workers.min(MAX_METRIC_WORKERS);
    let mut out = String::with_capacity(512 + workers * 160);
    out.push_str(
        "# HELP exyonq_worker_count Configured accept workers in the primary listener pool.\n",
    );
    out.push_str("# TYPE exyonq_worker_count gauge\n");
    out.push_str(&format!(
        "exyonq_worker_count{{pool=\"accept\"}} {workers}\n"
    ));
    out.push_str(
        "# HELP exyonq_worker_pool_resident_memory_bytes Resident set size of the accept-worker pool process. Linux workers are threads sharing one address space; this is the process VmRSS, not a sum of per-thread values.\n",
    );
    out.push_str("# TYPE exyonq_worker_pool_resident_memory_bytes gauge\n");
    out.push_str(&format!(
        "exyonq_worker_pool_resident_memory_bytes{{pool=\"accept\",workers=\"{workers}\"}} {rss_bytes}\n"
    ));
    out.push_str(
        "# HELP exyonq_worker_resident_memory_bytes Per-worker view of the containing process VmRSS. Linux threads share one resident set, so values repeat and scope=\"shared\"; they are not independent per-thread RSS.\n",
    );
    out.push_str("# TYPE exyonq_worker_resident_memory_bytes gauge\n");
    for worker in 0..workers {
        out.push_str(&format!(
            "exyonq_worker_resident_memory_bytes{{pool=\"accept\",worker=\"{worker}\",scope=\"shared\"}} {rss_bytes}\n"
        ));
    }
    out
}

/// Read the current process RSS from Linux `/proc`. Returns `None` on
/// platforms/filesystems without that interface instead of fabricating a value.
fn process_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_vm_rss_bytes(&status)
}

fn parse_vm_rss_bytes(status: &str) -> Option<u64> {
    for line in status.lines() {
        let Some(rest) = line.strip_prefix("VmRSS:") else {
            continue;
        };
        let kib = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        return kib.checked_mul(1024);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vmrss_kib_as_bytes() {
        assert_eq!(
            parse_vm_rss_bytes("Name:\tex\nVmRSS:\t  1234 kB\n"),
            Some(1_263_616)
        );
        assert_eq!(parse_vm_rss_bytes("VmRSS:\tnot-a-number kB\n"), None);
        assert_eq!(parse_vm_rss_bytes("Name:\tex\n"), None);
    }

    #[test]
    fn worker_series_count_tracks_configured_workers() {
        let body = render_worker_memory_prometheus(4, 8 * 1024 * 1024);
        assert_eq!(
            body.matches("exyonq_worker_resident_memory_bytes{").count(),
            4,
            "{body}"
        );
        assert!(body.contains("exyonq_worker_count{pool=\"accept\"} 4\n"));
        assert!(body.contains(
            "exyonq_worker_pool_resident_memory_bytes{pool=\"accept\",workers=\"4\"} 8388608\n"
        ));
        assert!(body.contains(
            "exyonq_worker_resident_memory_bytes{pool=\"accept\",worker=\"3\",scope=\"shared\"} 8388608\n"
        ));

        let capped = render_worker_memory_prometheus(MAX_METRIC_WORKERS + 8, 4096);
        assert_eq!(
            capped
                .matches("exyonq_worker_resident_memory_bytes{")
                .count(),
            MAX_METRIC_WORKERS,
            "per-worker series must stay bounded"
        );
        assert!(capped.contains(&format!(
            "exyonq_worker_pool_resident_memory_bytes{{pool=\"accept\",workers=\"{MAX_METRIC_WORKERS}\"}} 4096\n"
        )));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_process_rss_is_readable_and_rendered() {
        let rss = process_rss_bytes().expect("Linux /proc/self/status VmRSS");
        assert!(rss > 0, "VmRSS must be positive");
        let body = render_worker_memory_prometheus(2, rss);
        assert!(body.contains(&format!(
            "exyonq_worker_resident_memory_bytes{{pool=\"accept\",worker=\"0\",scope=\"shared\"}} {rss}\n"
        )));
        assert!(body.contains(&format!(
            "exyonq_worker_resident_memory_bytes{{pool=\"accept\",worker=\"1\",scope=\"shared\"}} {rss}\n"
        )));
    }
}
