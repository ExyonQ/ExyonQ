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
//! Prometheus / OpenMetrics metrics module (Fase 2 / v0.2.0 P12).
//!
//! Cap054: HTTP counters / duration / labeled series are process-lifetime
//! (survive config reload). Duration is measured from module `on_request` until
//! `on_response`. For SSE, Cap032 keeps the event stream uncollected; Cap054
//! observes real status (and duration) via [`exyonq_module_api::ResponseBodyState::StreamingUnavailable`]
//! — never a fabricated empty product body.

mod bench_trace;
mod fcgi_observability;
mod kernel_shell_metrics;
mod observability_runtime;

use async_trait::async_trait;
pub use bench_trace::{enabled as bench_trace_enabled, record_match_us, SampleGuard};
use exyonq_addon_sdk::{
    official_core_compat, static_descriptor, Addon, AddonDescriptor, Capability, CostClass,
};
use exyonq_module_api::{Body, HttpRequest, HttpResponse, Module, ModuleInfo, ResponseObservation};
pub use fcgi_observability::fcgi_responses_501_total;
use http::{header, Response, StatusCode};
pub use kernel_shell_metrics::{
    append_prometheus as append_kernel_shell_prometheus, fastcgi_http_501_total,
    proxy_http_501_total, static_http_501_total, KernelShellMetrics,
};
pub use observability_runtime::{register_observability_runtime, ObservabilityRegisterError};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

const MAX_LABELED_SERIES: usize = 64;

const HISTOGRAM_BUCKETS_MS: &[f64] = &[
    0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0,
];

/// Cap054: wall-clock start stamped in `on_request`, consumed in `on_response`.
#[derive(Clone, Copy, Debug)]
struct RequestStart(Instant);

static METRICS_DESCRIPTOR: std::sync::LazyLock<AddonDescriptor> = std::sync::LazyLock::new(|| {
    static_descriptor(
        "metrics",
        env!("CARGO_PKG_VERSION"),
        official_core_compat(),
        &[Capability::Telemetry],
        CostClass::ZeroCostWhenDisabled,
    )
});

/// Process-lifetime HTTP metric state (survives MetricsModule rebuild on reload).
struct ProcessHttpMetrics {
    started_at: Instant,
    requests_total: AtomicU64,
    responses_2xx: AtomicU64,
    responses_4xx: AtomicU64,
    responses_5xx: AtomicU64,
    duration_buckets: Vec<AtomicU64>,
    /// Accumulated duration in microseconds (exported as fractional milliseconds).
    duration_sum_us: AtomicU64,
    duration_count: AtomicU64,
    addon_invocations: AtomicU64,
    labeled_dropped: AtomicU64,
    labeled: Mutex<Vec<LabeledSeries>>,
}

impl ProcessHttpMetrics {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            requests_total: AtomicU64::new(0),
            responses_2xx: AtomicU64::new(0),
            responses_4xx: AtomicU64::new(0),
            responses_5xx: AtomicU64::new(0),
            duration_buckets: HISTOGRAM_BUCKETS_MS
                .iter()
                .map(|_| AtomicU64::new(0))
                .collect(),
            duration_sum_us: AtomicU64::new(0),
            duration_count: AtomicU64::new(0),
            addon_invocations: AtomicU64::new(0),
            labeled_dropped: AtomicU64::new(0),
            labeled: Mutex::new(Vec::new()),
        }
    }
}

static PROCESS_HTTP: LazyLock<ProcessHttpMetrics> = LazyLock::new(ProcessHttpMetrics::new);
static PROCESS_WIRE_METRICS_ENABLED: AtomicBool = AtomicBool::new(false);

/// Eagerly stamp process-start for health uptime (call from composition root).
pub fn touch_process_http_metrics() {
    let _ = &*PROCESS_HTTP;
}

/// Static manifest for handshake registration (addon-api 1.x).
pub fn descriptor() -> &'static AddonDescriptor {
    &METRICS_DESCRIPTOR
}

pub struct MetricsModule {
    metrics_path: String,
    health_path: String,
    version: String,
    scrape_bearer_token: Option<String>,
}

#[derive(Debug, Clone)]
struct LabeledSeries {
    method: String,
    status: String,
    route: String,
    count: u64,
}

/// Append optional runtime counters from the core data plane (registered once at startup).
pub fn register_runtime_prometheus_append(append: fn(&mut String)) {
    let _ = RUNTIME_PROMETHEUS_APPEND.set(append);
}

static RUNTIME_PROMETHEUS_APPEND: std::sync::OnceLock<fn(&mut String)> = std::sync::OnceLock::new();

impl MetricsModule {
    pub fn set_wire_enabled(enabled: bool) {
        PROCESS_WIRE_METRICS_ENABLED.store(enabled, Ordering::Release);
    }

    pub fn new(
        metrics_path: impl Into<String>,
        health_path: impl Into<String>,
        version: impl Into<String>,
        scrape_bearer_token: Option<String>,
    ) -> Self {
        touch_process_http_metrics();
        Self {
            metrics_path: metrics_path.into(),
            health_path: health_path.into(),
            version: version.into(),
            scrape_bearer_token,
        }
    }

    fn unauthorized_scrape() -> HttpResponse {
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(header::WWW_AUTHENTICATE, "Bearer realm=\"exyonq-metrics\"")
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("unauthorized"))
            .expect("401 response")
    }

    /// Constant-time-ish compare (length still leaks). Used for scrape bearer only.
    fn bearer_matches(provided: &str, expected: &str) -> bool {
        let a = provided.as_bytes();
        let b = expected.as_bytes();
        if a.len() != b.len() {
            return false;
        }
        let mut diff = 0u8;
        for (x, y) in a.iter().zip(b.iter()) {
            diff |= x ^ y;
        }
        diff == 0
    }

    // Module API returns the deny response as Err; boxing would change the hook contract.
    #[allow(clippy::result_large_err)]
    fn authorize_scrape(&self, req: &HttpRequest) -> Result<(), HttpResponse> {
        let Some(expected) = self.scrape_bearer_token.as_deref() else {
            return Ok(());
        };
        let Some(header_val) = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        else {
            return Err(Self::unauthorized_scrape());
        };
        // RFC 7235: auth-scheme is case-insensitive.
        let mut parts = header_val.splitn(2, char::is_whitespace);
        let scheme = parts.next().unwrap_or("");
        let provided = parts.next().map(str::trim).unwrap_or("");
        if !scheme.eq_ignore_ascii_case("Bearer") || provided.is_empty() {
            return Err(Self::unauthorized_scrape());
        }
        if Self::bearer_matches(provided, expected) {
            Ok(())
        } else {
            Err(Self::unauthorized_scrape())
        }
    }

    pub fn record_labeled(&self, method: &str, status: u16, route: &str) {
        Self::bump_labeled(method, status, route);
    }

    fn bump_labeled(method: &str, status: u16, route: &str) {
        let status = status_class(status);
        let route = sanitize_route_label(route);
        let mut labeled = match PROCESS_HTTP.labeled.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(series) = labeled
            .iter_mut()
            .find(|s| s.method == method && s.status == status && s.route == route)
        {
            series.count += 1;
            return;
        }
        if labeled.len() >= MAX_LABELED_SERIES {
            PROCESS_HTTP.labeled_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        labeled.push(LabeledSeries {
            method: method.to_string(),
            status,
            route,
            count: 1,
        });
    }

    /// Hot-path counter bump for completed module-observed responses.
    pub fn record_response(&self, status: u16) {
        Self::wire_record_response(status);
    }

    /// Cap067 / proxy-wire counter bump (same process-lifetime store as Hyper path).
    pub fn wire_record_response(status: u16) {
        if !PROCESS_WIRE_METRICS_ENABLED.load(Ordering::Acquire) {
            return;
        }
        PROCESS_HTTP.requests_total.fetch_add(1, Ordering::Relaxed);
        match status {
            200..=299 => {
                PROCESS_HTTP.responses_2xx.fetch_add(1, Ordering::Relaxed);
            }
            400..=499 => {
                PROCESS_HTTP.responses_4xx.fetch_add(1, Ordering::Relaxed);
            }
            500..=599 => {
                PROCESS_HTTP.responses_5xx.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    /// Record request latency (milliseconds) for histogram export.
    pub fn record_duration_ms(&self, elapsed_ms: f64) {
        Self::bump_duration_ms(elapsed_ms);
    }

    /// Wire/Cap067 observation: same counters as `on_response`, without a module instance.
    pub fn wire_record_exchange(method: &str, path: &str, status: u16, elapsed_ms: f64) {
        if !PROCESS_WIRE_METRICS_ENABLED.load(Ordering::Acquire) {
            return;
        }
        Self::wire_record_response(status);
        Self::bump_labeled(method, status, path);
        Self::bump_duration_ms(elapsed_ms);
    }

    fn bump_duration_ms(elapsed_ms: f64) {
        if !elapsed_ms.is_finite() || elapsed_ms < 0.0 {
            return;
        }
        PROCESS_HTTP.duration_count.fetch_add(1, Ordering::Relaxed);
        // Microsecond accumulator so sub-millisecond observations are not rounded to 0.
        let us = (elapsed_ms * 1000.0).round();
        if us.is_finite() && us >= 0.0 && us <= u64::MAX as f64 {
            PROCESS_HTTP
                .duration_sum_us
                .fetch_add(us as u64, Ordering::Relaxed);
        }
        for (idx, bound) in HISTOGRAM_BUCKETS_MS.iter().enumerate() {
            if elapsed_ms <= *bound {
                PROCESS_HTTP.duration_buckets[idx].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record one metrics-module hook invocation (exported as a real counter).
    pub fn record_addon_invocation(&self) {
        PROCESS_HTTP
            .addon_invocations
            .fetch_add(1, Ordering::Relaxed);
    }

    fn render_health_json(&self) -> String {
        let uptime_s = PROCESS_HTTP.started_at.elapsed().as_secs();
        format!(
            "{{\"version\":\"{}\",\"uptime_s\":{},\"status\":\"ok\"}}",
            escape_json_string(&self.version),
            uptime_s
        )
    }

    fn render_prometheus(&self) -> String {
        let total = PROCESS_HTTP.requests_total.load(Ordering::Relaxed);
        let ok = PROCESS_HTTP.responses_2xx.load(Ordering::Relaxed);
        let client_err = PROCESS_HTTP.responses_4xx.load(Ordering::Relaxed);
        let server_err = PROCESS_HTTP.responses_5xx.load(Ordering::Relaxed);
        let duration_sum_us = PROCESS_HTTP.duration_sum_us.load(Ordering::Relaxed);
        let duration_sum_ms = duration_sum_us as f64 / 1000.0;
        let duration_count = PROCESS_HTTP.duration_count.load(Ordering::Relaxed);
        let addon_invocations = PROCESS_HTTP.addon_invocations.load(Ordering::Relaxed);
        let labeled_dropped = PROCESS_HTTP.labeled_dropped.load(Ordering::Relaxed);

        let mut out = String::with_capacity(2048);
        out.push_str(
            "# HELP exyonq_http_requests_total HTTP requests observed by the metrics module (process lifetime; excludes core /health /live /ready probes; includes metrics scrapes).\n",
        );
        out.push_str("# TYPE exyonq_http_requests_total counter\n");
        out.push_str(&format!("exyonq_http_requests_total {total}\n"));
        out.push_str("# HELP exyonq_http_responses_2xx_total 2xx responses (process lifetime).\n");
        out.push_str("# TYPE exyonq_http_responses_2xx_total counter\n");
        out.push_str(&format!("exyonq_http_responses_2xx_total {ok}\n"));
        out.push_str("# HELP exyonq_http_responses_4xx_total 4xx responses (process lifetime).\n");
        out.push_str("# TYPE exyonq_http_responses_4xx_total counter\n");
        out.push_str(&format!("exyonq_http_responses_4xx_total {client_err}\n"));
        out.push_str("# HELP exyonq_http_responses_5xx_total 5xx responses (process lifetime).\n");
        out.push_str("# TYPE exyonq_http_responses_5xx_total counter\n");
        out.push_str(&format!("exyonq_http_responses_5xx_total {server_err}\n"));

        out.push_str(
            "# HELP exyonq_http_request_duration_milliseconds Request latency in milliseconds from module on_request until after response-body materialization in the module pipeline (SSE counted without collecting the event-stream body).\n",
        );
        out.push_str("# TYPE exyonq_http_request_duration_milliseconds histogram\n");
        for (idx, bound) in HISTOGRAM_BUCKETS_MS.iter().enumerate() {
            let count = PROCESS_HTTP.duration_buckets[idx].load(Ordering::Relaxed);
            out.push_str(&format!(
                "exyonq_http_request_duration_milliseconds_bucket{{le=\"{bound}\"}} {count}\n"
            ));
        }
        out.push_str(&format!(
            "exyonq_http_request_duration_milliseconds_bucket{{le=\"+Inf\"}} {duration_count}\n"
        ));
        out.push_str(&format!(
            "exyonq_http_request_duration_milliseconds_sum {duration_sum_ms}\n"
        ));
        out.push_str(&format!(
            "exyonq_http_request_duration_milliseconds_count {duration_count}\n"
        ));

        out.push_str(
            "# HELP exyonq_addon_invocations_total Metrics-module hook invocations observed (process lifetime).\n",
        );
        out.push_str("# TYPE exyonq_addon_invocations_total counter\n");
        out.push_str(&format!(
            "exyonq_addon_invocations_total{{addon=\"metrics\",hook=\"on_response\"}} {addon_invocations}\n"
        ));

        out.push_str(
            "# HELP exyonq_http_requests_labeled_dropped_total Labeled observations dropped after the 64-series cardinality cap (process lifetime).\n",
        );
        out.push_str("# TYPE exyonq_http_requests_labeled_dropped_total counter\n");
        out.push_str(&format!(
            "exyonq_http_requests_labeled_dropped_total {labeled_dropped}\n"
        ));

        if let Ok(labeled) = PROCESS_HTTP.labeled.lock() {
            out.push_str(
                "# HELP exyonq_http_requests_labeled_total HTTP requests by method/status/route (process lifetime; max 64 series).\n",
            );
            out.push_str("# TYPE exyonq_http_requests_labeled_total counter\n");
            for series in labeled.iter() {
                out.push_str(&format!(
                    "exyonq_http_requests_labeled_total{{method=\"{}\",status=\"{}\",route=\"{}\"}} {}\n",
                    escape_label(&series.method),
                    escape_label(&series.status),
                    escape_label(&series.route),
                    series.count
                ));
            }
        }

        if let Some(append) = RUNTIME_PROMETHEUS_APPEND.get() {
            append(&mut out);
        }

        out.push_str("# EOF\n");
        out
    }
}

fn status_class(status: u16) -> String {
    format!("{}xx", status / 100)
}

fn sanitize_route_label(route: &str) -> String {
    // Path only (no query) — callers must pass uri.path().
    let trimmed = route.trim();
    if let Some(q) = trimmed.find('?') {
        return sanitize_route_label(&trimmed[..q]);
    }
    if trimmed.is_empty() || trimmed == "/" {
        return "_root".to_string();
    }
    let mut out = trimmed.replace('/', "_");
    if !out.starts_with('_') {
        out.insert(0, '_');
    }
    if out.len() > 48 {
        let mut end = 48;
        while end > 0 && !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

fn escape_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

impl Addon for MetricsModule {
    fn descriptor(&self) -> &'static AddonDescriptor {
        descriptor()
    }
}

#[async_trait]
impl Module for MetricsModule {
    fn info(&self) -> ModuleInfo {
        ModuleInfo::from_descriptor(descriptor())
    }

    async fn on_request(&self, req: &mut HttpRequest) -> Result<(), exyonq_module_api::BoxError> {
        req.extensions_mut().insert(RequestStart(Instant::now()));
        Ok(())
    }

    async fn on_route(
        &self,
        req: &HttpRequest,
    ) -> Result<Option<HttpResponse>, exyonq_module_api::BoxError> {
        let path = req.uri().path();
        if path == self.health_path {
            if let Err(denied) = self.authorize_scrape(req) {
                return Ok(Some(denied));
            }
            let body = self.render_health_json();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store")
                .body(Body::from(body))
                .expect("health response");
            return Ok(Some(response));
        }
        if path != self.metrics_path {
            return Ok(None);
        }
        if let Err(denied) = self.authorize_scrape(req) {
            return Ok(Some(denied));
        }
        // Read-only scrape: only GET/HEAD produce the exposition.
        match *req.method() {
            http::Method::GET | http::Method::HEAD => {}
            _ => {
                let response = Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .header(header::ALLOW, "GET, HEAD")
                    .body(Body::from(bytes::Bytes::new()))
                    .expect("method not allowed");
                return Ok(Some(response));
            }
        }
        let body = self.render_prometheus();
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(body))
            .expect("metrics response");
        Ok(Some(response))
    }

    async fn on_response(
        &self,
        req: &HttpRequest,
        obs: &mut ResponseObservation,
    ) -> Result<(), exyonq_module_api::BoxError> {
        self.record_addon_invocation();
        let status = obs.status().as_u16();
        self.record_response(status);
        let method = req.method().as_str();
        let route = req.uri().path();
        self.record_labeled(method, status, route);
        if let Some(start) = req.extensions().get::<RequestStart>() {
            let elapsed_ms = start.0.elapsed().as_secs_f64() * 1000.0;
            self.record_duration_ms(elapsed_ms);
        }
        Ok(())
    }
}

#[cfg(test)]
mod observability_runtime_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use http::Request;

    #[tokio::test]
    async fn serves_prometheus_on_configured_path() {
        let module = MetricsModule::new("/metrics", "/exyonq-metrics-health", "0.2.0", None);
        let req = Request::builder()
            .uri("http://127.0.0.1/metrics")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = module.on_route(&req).await.unwrap().expect("metrics body");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body();
        let bytes = http_body_util::BodyExt::collect(body)
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("exyonq_http_requests_total"));
        assert!(text.contains("exyonq_http_request_duration_milliseconds_bucket"));
        assert!(text.contains("# EOF"));
        assert!(text.contains("after response-body materialization"));
        assert!(text.contains("exyonq_http_requests_labeled_dropped_total"));
    }

    #[test]
    fn runtime_append_includes_custom_counter_when_registered() {
        // Overlay-only registration: must not clear/replace production appenders
        // (workspace-parallel race root cause under former clear_for_tests).
        let _guard = exyonq_module_api::observability_runtime::begin_prometheus_appender_test();
        exyonq_module_api::observability_runtime::register_test_prometheus_appender(|out| {
            out.push_str("exyonq_test_custom_total 99\n");
        });
        let mut out = String::new();
        exyonq_module_api::observability_runtime::append_registered_prometheus(&mut out);
        assert!(out.contains("exyonq_test_custom_total"));
    }

    #[tokio::test]
    async fn serves_health_json() {
        let module = MetricsModule::new("/metrics", "/exyonq-metrics-health", "0.2.0", None);
        let req = Request::builder()
            .uri("http://127.0.0.1/exyonq-metrics-health")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = module.on_route(&req).await.unwrap().expect("health body");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body();
        let bytes = http_body_util::BodyExt::collect(body)
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("\"version\":\"0.2.0\""));
        assert!(text.contains("\"status\":\"ok\""));
    }

    #[tokio::test]
    async fn on_request_stamps_duration_recorded_in_on_response() {
        let module = MetricsModule::new("/metrics", "/exyonq-metrics-health", "0.4.4", None);
        let before = PROCESS_HTTP.duration_count.load(Ordering::Relaxed);
        let sum_before = PROCESS_HTTP.duration_sum_us.load(Ordering::Relaxed);
        let mut req = Request::builder()
            .uri("http://127.0.0.1/site/")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        module.on_request(&mut req).await.unwrap();
        assert!(req.extensions().get::<RequestStart>().is_some());
        std::thread::sleep(std::time::Duration::from_millis(2));
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let mut obs = ResponseObservation::from_http_response(resp);
        module.on_response(&req, &mut obs).await.unwrap();
        let after = PROCESS_HTTP.duration_count.load(Ordering::Relaxed);
        let sum_after = PROCESS_HTTP.duration_sum_us.load(Ordering::Relaxed);
        assert!(after > before, "duration_count must advance");
        assert!(sum_after > sum_before, "duration_sum_us must advance");
    }

    #[test]
    fn sub_millisecond_duration_still_increments_sum() {
        let module = MetricsModule::new("/metrics", "/exyonq-metrics-health", "0.4.4", None);
        let before = PROCESS_HTTP.duration_sum_us.load(Ordering::Relaxed);
        module.record_duration_ms(0.25);
        let after = PROCESS_HTTP.duration_sum_us.load(Ordering::Relaxed);
        assert!(after > before, "0.25ms must contribute to sum_us");
    }

    #[test]
    fn wire_terminal_statuses_are_accounted_once_by_real_class() {
        MetricsModule::set_wire_enabled(true);
        let total_before = PROCESS_HTTP.requests_total.load(Ordering::Relaxed);
        let responses_2xx_before = PROCESS_HTTP.responses_2xx.load(Ordering::Relaxed);
        let responses_4xx_before = PROCESS_HTTP.responses_4xx.load(Ordering::Relaxed);
        let responses_5xx_before = PROCESS_HTTP.responses_5xx.load(Ordering::Relaxed);

        for status in [200, 206, 304, 403, 404, 416, 429, 500] {
            MetricsModule::wire_record_response(status);
        }

        assert_eq!(
            PROCESS_HTTP.requests_total.load(Ordering::Relaxed) - total_before,
            8
        );
        assert_eq!(
            PROCESS_HTTP.responses_2xx.load(Ordering::Relaxed) - responses_2xx_before,
            2
        );
        assert_eq!(
            PROCESS_HTTP.responses_4xx.load(Ordering::Relaxed) - responses_4xx_before,
            4
        );
        assert_eq!(
            PROCESS_HTTP.responses_5xx.load(Ordering::Relaxed) - responses_5xx_before,
            1
        );
        MetricsModule::set_wire_enabled(false);
    }

    #[tokio::test]
    async fn metrics_rejects_post() {
        let module = MetricsModule::new("/metrics", "/exyonq-metrics-health", "0.4.4", None);
        let req = Request::builder()
            .method(http::Method::POST)
            .uri("http://127.0.0.1/metrics")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = module.on_route(&req).await.unwrap().expect("response");
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn scrape_bearer_required_when_configured() {
        let module = MetricsModule::new(
            "/metrics",
            "/exyonq-metrics-health",
            "0.4.4",
            Some("test-token-la008".into()),
        );
        let no_auth = Request::builder()
            .uri("http://127.0.0.1/metrics")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let denied = module.on_route(&no_auth).await.unwrap().expect("401");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let wrong = Request::builder()
            .uri("http://127.0.0.1/metrics")
            .header(header::AUTHORIZATION, "Bearer wrong-token")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let denied2 = module.on_route(&wrong).await.unwrap().expect("401");
        assert_eq!(denied2.status(), StatusCode::UNAUTHORIZED);

        let ok = Request::builder()
            .uri("http://127.0.0.1/metrics")
            .header(header::AUTHORIZATION, "Bearer test-token-la008")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let allowed = module.on_route(&ok).await.unwrap().expect("200");
        assert_eq!(allowed.status(), StatusCode::OK);

        let mixed = Request::builder()
            .uri("http://127.0.0.1/metrics")
            .header(header::AUTHORIZATION, "BEARER test-token-la008")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let mixed_ok = module.on_route(&mixed).await.unwrap().expect("200");
        assert_eq!(mixed_ok.status(), StatusCode::OK);
    }

    #[test]
    fn sanitize_route_strips_query() {
        assert_eq!(sanitize_route_label("/api/x?y=1"), "_api_x");
    }
}
