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
//! Process-wide CFD observation hub (atomics + bounded event ring).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Process-wide bound on queued structured events.
pub const EVENT_QUEUE_BOUND: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    S2xx,
    S3xx,
    S4xx,
    S5xx,
    Other,
}

impl StatusClass {
    pub fn from_status(code: u16) -> Self {
        match code {
            200..=299 => Self::S2xx,
            300..=399 => Self::S3xx,
            400..=499 => Self::S4xx,
            500..=599 => Self::S5xx,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::S2xx => "2xx",
            Self::S3xx => "3xx",
            Self::S4xx => "4xx",
            Self::S5xx => "5xx",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Full event vocabulary for Cap061 projection; not all paths emit yet.
pub enum ObsEvent {
    Access,
    WafDeny,
    WafAllow,
    RouteMiss,
    UpstreamError,
    ClientDisconnect,
    UpstreamDisconnect,
    Timeout,
    Backpressure,
    GenerationActivate,
    FramingReject,
    InternalError,
}

impl ObsEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::WafDeny => "waf_deny",
            Self::WafAllow => "waf_allow",
            Self::RouteMiss => "route_miss",
            Self::UpstreamError => "upstream_error",
            Self::ClientDisconnect => "client_disconnect",
            Self::UpstreamDisconnect => "upstream_disconnect",
            Self::Timeout => "timeout",
            Self::Backpressure => "backpressure",
            Self::GenerationActivate => "generation_activate",
            Self::FramingReject => "framing_reject",
            Self::InternalError => "internal_error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObsRecord {
    pub ts_unix_ms: u64,
    pub event: ObsEvent,
    pub method: &'static str,
    pub route_id: u32,
    pub status: u16,
    pub status_class: StatusClass,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub latency_us: u64,
    pub generation_id: u64,
    pub shard_id: u32,
    pub error_class: &'static str,
}

impl ObsRecord {
    pub fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    pub fn to_json_line(&self) -> String {
        format!(
            "{{\"timestamp_ms\":{},\"level\":\"INFO\",\"event\":\"{}\",\"method\":\"{}\",\"route_id\":{},\"status\":{},\"status_class\":\"{}\",\"bytes_in\":{},\"bytes_out\":{},\"latency_us\":{},\"generation_id\":{},\"shard_id\":{},\"error_class\":\"{}\"}}\n",
            self.ts_unix_ms,
            self.event.as_str(),
            self.method,
            self.route_id,
            self.status,
            self.status_class.as_str(),
            self.bytes_in,
            self.bytes_out,
            self.latency_us,
            self.generation_id,
            self.shard_id,
            self.error_class
        )
    }
}

/// Shared observation state. Request path: Relaxed atomics + try_push only.
#[derive(Debug, Default)]
pub struct ObsHub {
    pub requests_total: AtomicU64,
    pub responses_total: AtomicU64,
    pub status_2xx: AtomicU64,
    pub status_3xx: AtomicU64,
    pub status_4xx: AtomicU64,
    pub status_5xx: AtomicU64,
    pub status_other: AtomicU64,
    pub request_bytes: AtomicU64,
    pub response_bytes: AtomicU64,
    pub active_connections: AtomicU64,
    pub accepted_connections: AtomicU64,
    pub closed_connections: AtomicU64,
    pub client_disconnects: AtomicU64,
    pub upstream_disconnects: AtomicU64,
    pub timeouts: AtomicU64,
    pub waf_allows: AtomicU64,
    pub waf_denies: AtomicU64,
    pub route_hits: AtomicU64,
    pub route_misses: AtomicU64,
    pub upstream_connects: AtomicU64,
    pub upstream_reuse: AtomicU64,
    pub upstream_discard: AtomicU64,
    pub generation_current: AtomicU64,
    pub generation_publishes: AtomicU64,
    pub generation_activate: AtomicU64,
    pub generation_reject: AtomicU64,
    pub body_requests: AtomicU64,
    pub body_bytes: AtomicU64,
    pub backpressure_activations: AtomicU64,
    pub framing_rejects: AtomicU64,
    pub cl_te_rejects: AtomicU64,
    pub internal_errors: AtomicU64,
    pub fcgi_requests: AtomicU64,
    pub fcgi_success: AtomicU64,
    pub fcgi_connect_fail: AtomicU64,
    pub fcgi_timeout: AtomicU64,
    pub fcgi_protocol_fail: AtomicU64,
    pub fcgi_pool_hit: AtomicU64,
    pub fcgi_pool_miss: AtomicU64,
    pub fcgi_pool_discard: AtomicU64,
    pub latency_sum_us: AtomicU64,
    pub latency_count: AtomicU64,
    pub latency_bucket_1ms: AtomicU64,
    pub latency_bucket_5ms: AtomicU64,
    pub latency_bucket_25ms: AtomicU64,
    pub latency_bucket_100ms: AtomicU64,
    pub latency_bucket_inf: AtomicU64,
    pub events_dropped: AtomicU64,
    pub events_export_attempts: AtomicU64,
    pub events_exported: AtomicU64,
    pub queue_high_water: AtomicU64,
    pub export_errors: AtomicU64,
    events: Mutex<VecDeque<ObsRecord>>,
}

pub type ObsHubHandle = Arc<ObsHub>;

fn append_metric_line(s: &mut String, name: &str, help: &str, ty: &str, val: u64) {
    s.push_str("# HELP ");
    s.push_str(name);
    s.push(' ');
    s.push_str(help);
    s.push('\n');
    s.push_str("# TYPE ");
    s.push_str(name);
    s.push(' ');
    s.push_str(ty);
    s.push('\n');
    s.push_str(name);
    s.push(' ');
    s.push_str(&val.to_string());
    s.push('\n');
}

impl ObsHub {
    pub fn new() -> ObsHubHandle {
        Arc::new(Self::default())
    }

    #[inline]
    pub fn note_request_complete(
        &self,
        status: u16,
        bytes_in: u64,
        bytes_out: u64,
        latency_us: u64,
        had_body: bool,
    ) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        self.responses_total.fetch_add(1, Ordering::Relaxed);
        self.request_bytes.fetch_add(bytes_in, Ordering::Relaxed);
        self.response_bytes.fetch_add(bytes_out, Ordering::Relaxed);
        self.latency_sum_us.fetch_add(latency_us, Ordering::Relaxed);
        self.latency_count.fetch_add(1, Ordering::Relaxed);
        match StatusClass::from_status(status) {
            StatusClass::S2xx => self.status_2xx.fetch_add(1, Ordering::Relaxed),
            StatusClass::S3xx => self.status_3xx.fetch_add(1, Ordering::Relaxed),
            StatusClass::S4xx => self.status_4xx.fetch_add(1, Ordering::Relaxed),
            StatusClass::S5xx => self.status_5xx.fetch_add(1, Ordering::Relaxed),
            StatusClass::Other => self.status_other.fetch_add(1, Ordering::Relaxed),
        };
        if had_body {
            self.body_requests.fetch_add(1, Ordering::Relaxed);
            self.body_bytes.fetch_add(bytes_in, Ordering::Relaxed);
        }
        if latency_us <= 1_000 {
            self.latency_bucket_1ms.fetch_add(1, Ordering::Relaxed);
        } else if latency_us <= 5_000 {
            self.latency_bucket_5ms.fetch_add(1, Ordering::Relaxed);
        } else if latency_us <= 25_000 {
            self.latency_bucket_25ms.fetch_add(1, Ordering::Relaxed);
        } else if latency_us <= 100_000 {
            self.latency_bucket_100ms.fetch_add(1, Ordering::Relaxed);
        } else {
            self.latency_bucket_inf.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn try_push_event(&self, record: ObsRecord) {
        let Ok(mut q) = self.events.lock() else {
            self.events_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if q.len() >= EVENT_QUEUE_BOUND {
            self.events_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        q.push_back(record);
        let hw = q.len() as u64;
        let cur = self.queue_high_water.load(Ordering::Relaxed);
        if hw > cur {
            self.queue_high_water.store(hw, Ordering::Relaxed);
        }
    }

    pub fn drain_events(&self, max: usize) -> Vec<ObsRecord> {
        let Ok(mut q) = self.events.lock() else {
            return Vec::new();
        };
        let n = max.min(q.len());
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(r) = q.pop_front() {
                out.push(r);
            }
        }
        out
    }

    pub fn render_openmetrics(&self) -> String {
        let mut s = String::with_capacity(4096);
        append_metric_line(
            &mut s,
            "exyonq_cfd_requests_total",
            "CFD requests completed",
            "counter",
            self.requests_total.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_responses_total",
            "CFD responses completed",
            "counter",
            self.responses_total.load(Ordering::Relaxed),
        );
        s.push_str("# HELP exyonq_cfd_responses_by_class_total Responses by status class\n");
        s.push_str("# TYPE exyonq_cfd_responses_by_class_total counter\n");
        for (name, val) in [
            ("2xx", self.status_2xx.load(Ordering::Relaxed)),
            ("3xx", self.status_3xx.load(Ordering::Relaxed)),
            ("4xx", self.status_4xx.load(Ordering::Relaxed)),
            ("5xx", self.status_5xx.load(Ordering::Relaxed)),
            ("other", self.status_other.load(Ordering::Relaxed)),
        ] {
            s.push_str(&format!(
                "exyonq_cfd_responses_by_class_total{{class=\"{name}\"}} {val}\n"
            ));
        }
        append_metric_line(
            &mut s,
            "exyonq_cfd_request_bytes_total",
            "Request body bytes",
            "counter",
            self.request_bytes.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_response_bytes_total",
            "Response body bytes",
            "counter",
            self.response_bytes.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_active_connections",
            "Active connections",
            "gauge",
            self.active_connections.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_accepted_connections_total",
            "Accepted connections",
            "counter",
            self.accepted_connections.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_closed_connections_total",
            "Closed connections",
            "counter",
            self.closed_connections.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_client_disconnects_total",
            "Client disconnects",
            "counter",
            self.client_disconnects.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_upstream_disconnects_total",
            "Upstream disconnects",
            "counter",
            self.upstream_disconnects.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_timeouts_total",
            "Timeouts",
            "counter",
            self.timeouts.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_waf_allows_total",
            "WAF allows",
            "counter",
            self.waf_allows.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_waf_denies_total",
            "WAF denies",
            "counter",
            self.waf_denies.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_route_hits_total",
            "Route hits",
            "counter",
            self.route_hits.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_route_misses_total",
            "Route misses",
            "counter",
            self.route_misses.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_upstream_connects_total",
            "Upstream connects",
            "counter",
            self.upstream_connects.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_upstream_reuse_total",
            "Upstream reuse",
            "counter",
            self.upstream_reuse.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_upstream_discard_total",
            "Upstream discard",
            "counter",
            self.upstream_discard.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_generation_current",
            "Current generation id",
            "gauge",
            self.generation_current.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_generation_publishes_total",
            "Generation publish observations",
            "counter",
            self.generation_publishes.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_generation_activate_total",
            "Generation activations",
            "counter",
            self.generation_activate.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_generation_reject_total",
            "Generation rejects",
            "counter",
            self.generation_reject.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_body_requests_total",
            "Requests with body",
            "counter",
            self.body_requests.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_body_bytes_total",
            "Body bytes",
            "counter",
            self.body_bytes.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_backpressure_activations_total",
            "Backpressure activations",
            "counter",
            self.backpressure_activations.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_framing_rejects_total",
            "Framing rejects",
            "counter",
            self.framing_rejects.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_cl_te_rejects_total",
            "CL/TE rejects",
            "counter",
            self.cl_te_rejects.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_internal_errors_total",
            "Internal errors",
            "counter",
            self.internal_errors.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_fcgi_requests_total",
            "FastCGI requests",
            "counter",
            self.fcgi_requests.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_fcgi_success_total",
            "FastCGI successes",
            "counter",
            self.fcgi_success.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_fcgi_connect_fail_total",
            "FastCGI connect failures",
            "counter",
            self.fcgi_connect_fail.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_fcgi_timeout_total",
            "FastCGI timeouts",
            "counter",
            self.fcgi_timeout.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_fcgi_protocol_fail_total",
            "FastCGI protocol failures",
            "counter",
            self.fcgi_protocol_fail.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_fcgi_pool_hit_total",
            "FastCGI pool hits",
            "counter",
            self.fcgi_pool_hit.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_fcgi_pool_miss_total",
            "FastCGI pool misses",
            "counter",
            self.fcgi_pool_miss.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_fcgi_pool_discard_total",
            "FastCGI pool discards",
            "counter",
            self.fcgi_pool_discard.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_obs_events_dropped_total",
            "Dropped observability events",
            "counter",
            self.events_dropped.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_obs_events_export_attempts_total",
            "Observability export attempts",
            "counter",
            self.events_export_attempts.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_obs_events_exported_total",
            "Successfully exported observability events (all configured sinks)",
            "counter",
            self.events_exported.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_obs_queue_high_water",
            "Event queue high-water mark",
            "gauge",
            self.queue_high_water.load(Ordering::Relaxed),
        );
        append_metric_line(
            &mut s,
            "exyonq_cfd_obs_export_errors_total",
            "Export sink errors",
            "counter",
            self.export_errors.load(Ordering::Relaxed),
        );
        s.push_str("# HELP exyonq_cfd_request_latency_us Request latency histogram\n");
        s.push_str("# TYPE exyonq_cfd_request_latency_us histogram\n");
        let b1 = self.latency_bucket_1ms.load(Ordering::Relaxed);
        let b5 = b1 + self.latency_bucket_5ms.load(Ordering::Relaxed);
        let b25 = b5 + self.latency_bucket_25ms.load(Ordering::Relaxed);
        let b100 = b25 + self.latency_bucket_100ms.load(Ordering::Relaxed);
        let binf = b100 + self.latency_bucket_inf.load(Ordering::Relaxed);
        s.push_str(&format!(
            "exyonq_cfd_request_latency_us_bucket{{le=\"1000\"}} {b1}\n"
        ));
        s.push_str(&format!(
            "exyonq_cfd_request_latency_us_bucket{{le=\"5000\"}} {b5}\n"
        ));
        s.push_str(&format!(
            "exyonq_cfd_request_latency_us_bucket{{le=\"25000\"}} {b25}\n"
        ));
        s.push_str(&format!(
            "exyonq_cfd_request_latency_us_bucket{{le=\"100000\"}} {b100}\n"
        ));
        s.push_str(&format!(
            "exyonq_cfd_request_latency_us_bucket{{le=\"+Inf\"}} {binf}\n"
        ));
        s.push_str(&format!(
            "exyonq_cfd_request_latency_us_sum {}\n",
            self.latency_sum_us.load(Ordering::Relaxed)
        ));
        s.push_str(&format!(
            "exyonq_cfd_request_latency_us_count {}\n",
            self.latency_count.load(Ordering::Relaxed)
        ));
        s.push_str("# EOF\n");
        s
    }
}
