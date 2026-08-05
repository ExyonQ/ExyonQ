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

mod bench_trace;
mod fcgi_observability;
mod kernel_shell_metrics;
mod observability_runtime;

use async_trait::async_trait;
pub use bench_trace::{enabled as bench_trace_enabled, record_match_us, SampleGuard};
use exyonq_addon_sdk::{
    official_core_compat, static_descriptor, Addon, AddonDescriptor, Capability, CostClass,
};
use exyonq_module_api::{Body, HttpRequest, HttpResponse, Module, ModuleInfo};
pub use fcgi_observability::fcgi_responses_501_total;
use http::{header, Response, StatusCode};
pub use kernel_shell_metrics::{
    append_prometheus as append_kernel_shell_prometheus, fastcgi_http_501_total,
    proxy_http_501_total, static_http_501_total, KernelShellMetrics,
};
pub use observability_runtime::{register_observability_runtime, ObservabilityRegisterError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const MAX_LABELED_SERIES: usize = 64;

const HISTOGRAM_BUCKETS_MS: &[f64] = &[
    0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0,
];

static METRICS_DESCRIPTOR: std::sync::LazyLock<AddonDescriptor> = std::sync::LazyLock::new(|| {
    static_descriptor(
        "metrics",
        env!("CARGO_PKG_VERSION"),
        official_core_compat(),
        &[Capability::Telemetry],
        CostClass::ZeroCostWhenDisabled,
    )
});

/// Static manifest for handshake registration (addon-api 1.x).
pub fn descriptor() -> &'static AddonDescriptor {
    &METRICS_DESCRIPTOR
}

pub struct MetricsModule {
    metrics_path: String,
    health_path: String,
    version: String,
    started_at: Instant,
    requests_total: AtomicU64,
    responses_2xx: AtomicU64,
    responses_4xx: AtomicU64,
    responses_5xx: AtomicU64,
    duration_buckets: Vec<AtomicU64>,
    duration_sum_ms: AtomicU64,
    duration_count: AtomicU64,
    /// Placeholder hook for future addon registry telemetry.
    addon_invocations: AtomicU64,
    /// v0.3 labeled counters: method, status class, route (bounded cardinality).
    labeled: Mutex<Vec<LabeledSeries>>,
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
    pub fn new(
        metrics_path: impl Into<String>,
        health_path: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            metrics_path: metrics_path.into(),
            health_path: health_path.into(),
            version: version.into(),
            started_at: Instant::now(),
            requests_total: AtomicU64::new(0),
            responses_2xx: AtomicU64::new(0),
            responses_4xx: AtomicU64::new(0),
            responses_5xx: AtomicU64::new(0),
            duration_buckets: HISTOGRAM_BUCKETS_MS
                .iter()
                .map(|_| AtomicU64::new(0))
                .collect(),
            duration_sum_ms: AtomicU64::new(0),
            duration_count: AtomicU64::new(0),
            addon_invocations: AtomicU64::new(0),
            labeled: Mutex::new(Vec::new()),
        }
    }

    pub fn record_labeled(&self, method: &str, status: u16, route: &str) {
        let status = status_class(status);
        let route = sanitize_route_label(route);
        let mut labeled = self.labeled.lock().expect("metrics labeled lock");
        if let Some(series) = labeled
            .iter_mut()
            .find(|s| s.method == method && s.status == status && s.route == route)
        {
            series.count += 1;
            return;
        }
        if labeled.len() >= MAX_LABELED_SERIES {
            return;
        }
        labeled.push(LabeledSeries {
            method: method.to_string(),
            status,
            route,
            count: 1,
        });
    }

    /// Hot-path counter bump for the modules wire loop (P10 bench).
    pub fn record_response(&self, status: u16) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        match status {
            200..=299 => {
                self.responses_2xx.fetch_add(1, Ordering::Relaxed);
            }
            400..=499 => {
                self.responses_4xx.fetch_add(1, Ordering::Relaxed);
            }
            500..=599 => {
                self.responses_5xx.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    /// Record end-to-end request duration (milliseconds) for histogram export.
    pub fn record_duration_ms(&self, elapsed_ms: f64) {
        self.duration_count.fetch_add(1, Ordering::Relaxed);
        self.duration_sum_ms
            .fetch_add(elapsed_ms.round() as u64, Ordering::Relaxed);
        for (idx, bound) in HISTOGRAM_BUCKETS_MS.iter().enumerate() {
            if elapsed_ms <= *bound {
                self.duration_buckets[idx].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Placeholder for future `exyonq_addon_*` registry hooks.
    pub fn record_addon_invocation(&self) {
        self.addon_invocations.fetch_add(1, Ordering::Relaxed);
    }

    fn render_health_json(&self) -> String {
        let uptime_s = self.started_at.elapsed().as_secs();
        format!(
            "{{\"version\":\"{}\",\"uptime_s\":{},\"status\":\"ok\"}}",
            self.version, uptime_s
        )
    }

    fn render_prometheus(&self) -> String {
        let total = self.requests_total.load(Ordering::Relaxed);
        let ok = self.responses_2xx.load(Ordering::Relaxed);
        let client_err = self.responses_4xx.load(Ordering::Relaxed);
        let server_err = self.responses_5xx.load(Ordering::Relaxed);
        let duration_sum = self.duration_sum_ms.load(Ordering::Relaxed);
        let duration_count = self.duration_count.load(Ordering::Relaxed);
        let addon_invocations = self.addon_invocations.load(Ordering::Relaxed);

        let mut out = String::with_capacity(2048);
        out.push_str("# HELP exyonq_http_requests_total Total HTTP requests observed.\n");
        out.push_str("# TYPE exyonq_http_requests_total counter\n");
        out.push_str(&format!("exyonq_http_requests_total {total}\n"));
        out.push_str("# HELP exyonq_http_responses_2xx_total 2xx responses.\n");
        out.push_str("# TYPE exyonq_http_responses_2xx_total counter\n");
        out.push_str(&format!("exyonq_http_responses_2xx_total {ok}\n"));
        out.push_str("# HELP exyonq_http_responses_4xx_total 4xx responses.\n");
        out.push_str("# TYPE exyonq_http_responses_4xx_total counter\n");
        out.push_str(&format!("exyonq_http_responses_4xx_total {client_err}\n"));
        out.push_str("# HELP exyonq_http_responses_5xx_total 5xx responses.\n");
        out.push_str("# TYPE exyonq_http_responses_5xx_total counter\n");
        out.push_str(&format!("exyonq_http_responses_5xx_total {server_err}\n"));

        out.push_str("# HELP exyonq_http_request_duration_milliseconds Request duration.\n");
        out.push_str("# TYPE exyonq_http_request_duration_milliseconds histogram\n");
        for (idx, bound) in HISTOGRAM_BUCKETS_MS.iter().enumerate() {
            let count = self.duration_buckets[idx].load(Ordering::Relaxed);
            out.push_str(&format!(
                "exyonq_http_request_duration_milliseconds_bucket{{le=\"{bound}\"}} {count}\n"
            ));
        }
        out.push_str(&format!(
            "exyonq_http_request_duration_milliseconds_bucket{{le=\"+Inf\"}} {duration_count}\n"
        ));
        out.push_str(&format!(
            "exyonq_http_request_duration_milliseconds_sum {duration_sum}\n"
        ));
        out.push_str(&format!(
            "exyonq_http_request_duration_milliseconds_count {duration_count}\n"
        ));

        out.push_str(
            "# HELP exyonq_addon_invocations_total Addon hook invocations (placeholder).\n",
        );
        out.push_str("# TYPE exyonq_addon_invocations_total counter\n");
        out.push_str(&format!(
            "exyonq_addon_invocations_total{{addon=\"metrics\",hook=\"on_response\"}} {addon_invocations}\n"
        ));
        out.push_str(
            "exyonq_addon_invocations_total{addon=\"compression\",hook=\"on_response\"} 0\n",
        );
        out.push_str("exyonq_addon_invocations_total{addon=\"ratelimit\",hook=\"on_request\"} 0\n");
        out.push_str(
            "# HELP exyonq_addon_hook_duration_microseconds Addon hook latency (placeholder).\n",
        );
        out.push_str("# TYPE exyonq_addon_hook_duration_microseconds histogram\n");
        out.push_str("exyonq_addon_hook_duration_microseconds_bucket{addon=\"metrics\",hook=\"on_route\",le=\"10\"} 0\n");
        out.push_str("exyonq_addon_hook_duration_microseconds_bucket{addon=\"metrics\",hook=\"on_route\",le=\"+Inf\"} 0\n");
        out.push_str(
            "exyonq_addon_hook_duration_microseconds_sum{addon=\"metrics\",hook=\"on_route\"} 0\n",
        );
        out.push_str("exyonq_addon_hook_duration_microseconds_count{addon=\"metrics\",hook=\"on_route\"} 0\n");

        if let Ok(labeled) = self.labeled.lock() {
            out.push_str(
                "# HELP exyonq_http_requests_labeled_total HTTP requests by method/status/route.\n",
            );
            out.push_str("# TYPE exyonq_http_requests_labeled_total counter\n");
            for series in labeled.iter() {
                out.push_str(&format!(
                    "exyonq_http_requests_labeled_total{{method=\"{}\",status=\"{}\",route=\"{}\"}} {}\n",
                    series.method, series.status, series.route, series.count
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
    let trimmed = route.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "_root".to_string();
    }
    let mut out = trimmed.replace('/', "_");
    if !out.starts_with('_') {
        out.insert(0, '_');
    }
    if out.len() > 48 {
        out.truncate(48);
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

    async fn on_route(
        &self,
        req: &HttpRequest,
    ) -> Result<Option<HttpResponse>, exyonq_module_api::BoxError> {
        let path = req.uri().path();
        if path == self.health_path {
            let body = self.render_health_json();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
                .body(Body::from(body))
                .expect("health response");
            return Ok(Some(response));
        }
        if path != self.metrics_path {
            return Ok(None);
        }
        let body = self.render_prometheus();
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )
            .body(Body::from(body))
            .expect("metrics response");
        Ok(Some(response))
    }

    async fn on_response(
        &self,
        req: &HttpRequest,
        resp: &mut HttpResponse,
    ) -> Result<(), exyonq_module_api::BoxError> {
        self.record_addon_invocation();
        let status = resp.status().as_u16();
        self.record_response(status);
        let method = req.method().as_str();
        let route = req.uri().path();
        self.record_labeled(method, status, route);
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
        let module = MetricsModule::new("/metrics", "/health", "0.2.0");
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
    }

    #[test]
    fn runtime_append_includes_custom_counter_when_registered() {
        exyonq_module_api::observability_runtime::clear_prometheus_appenders_for_tests();
        exyonq_module_api::observability_runtime::register_prometheus_appender(|out| {
            out.push_str("exyonq_test_custom_total 99\n");
        });
        let mut out = String::new();
        exyonq_module_api::observability_runtime::append_registered_prometheus(&mut out);
        assert!(out.contains("exyonq_test_custom_total"));
    }

    #[tokio::test]
    async fn serves_health_json() {
        let module = MetricsModule::new("/metrics", "/health", "0.2.0");
        let req = Request::builder()
            .uri("http://127.0.0.1/health")
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
}
