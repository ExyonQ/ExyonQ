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
//! Cross-cutting module pipeline — compression, rate limiting, metrics, addon hooks (KD4.4).

mod convert;

pub use convert::{inject_client_ip, module_request_from_parts};

use convert::{
    boxed_from_module_response, module_request_from_incoming, module_response_from_boxed,
};
use exyonq_compression::CompressionModule;
use exyonq_config_ir::ModulesConfig;
use exyonq_metrics::MetricsModule;
use exyonq_module_api::{HttpRequest, ModuleRegistry, ResponseObservation, RuntimeProfile};
use exyonq_ratelimit::RateLimitModule;
use hyper::body::Incoming;
use hyper::header::CONTENT_TYPE;
use hyper::{Request, Response};
use semver::Version;
use std::future::Future;
use std::sync::Arc;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

const MODULE_BODY_LIMIT: usize = 4 * 1024 * 1024;

/// Cap032: SSE responses must stay streaming — never enter module body collect.
fn response_is_event_stream(response: &Response<BoxBody>) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| {
            ct.split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
        })
}

/// Per-snapshot pipeline instance (built at reload; owns module registry).
/// Cap055 rate-limit token state is process-lifetime (`PROCESS_MODULE_LIMITER`).
pub struct CrossCuttingPipeline {
    registry: ModuleRegistry,
    wire_ratelimit_enabled: bool,
    wire_ratelimit_requests_per_second: u32,
    wire_ratelimit_burst: u32,
    wire_ratelimit_path: Option<String>,
    wire_metrics_enabled: bool,
}

impl CrossCuttingPipeline {
    pub fn from_config(
        config: &ModulesConfig,
        core_version: &str,
    ) -> Result<Self, exyonq_module_api::AddonError> {
        let mut registry = ModuleRegistry::new();
        // Install stable process hooks once. Commit-time atomic enable flags select
        // no-op vs active behavior without replacing the module-api hook table.
        let _ = exyonq_module_api::install_wire_module_hooks(exyonq_module_api::WireModuleHooks {
            admit: RateLimitModule::wire_admit_client_ip,
            admit_path: RateLimitModule::wire_admit_client_ip_for_path,
            admit_active: RateLimitModule::wire_admit_active,
            record_response: MetricsModule::wire_record_response,
            record_exchange: MetricsModule::wire_record_exchange,
        });
        // LA-CAP054-008 / KD4.4: rate-limit short-circuit must run before metrics scrape
        // short-circuit so public /metrics cannot bypass Cap055 when both are enabled.
        if config.ratelimit.enabled {
            registry.register_addon(Arc::new(RateLimitModule::new()))?;
        }
        if config.metrics.enabled {
            registry.register_addon(Arc::new(MetricsModule::new(
                config.metrics.path.clone(),
                config.metrics.health_path.clone(),
                core_version,
                config.metrics.scrape_bearer_token.clone(),
            )))?;
        }
        if config.compression.enabled {
            registry.register_addon(Arc::new(CompressionModule::with_level(
                config.compression.min_bytes,
                config.compression.level,
            )))?;
        }
        let core_version = Version::parse(core_version).unwrap_or_else(|_| Version::new(0, 1, 0));
        registry.validate_handshake(&core_version, RuntimeProfile::Edge)?;
        Ok(Self {
            registry,
            wire_ratelimit_enabled: config.ratelimit.enabled,
            wire_ratelimit_requests_per_second: config.ratelimit.requests_per_second,
            wire_ratelimit_burst: config.ratelimit.burst,
            wire_ratelimit_path: config.ratelimit.path.clone(),
            wire_metrics_enabled: config.metrics.enabled,
        })
    }

    /// Publish process-wide wire flags only when the owning runtime snapshot commits.
    pub fn publish_wire_module_state(&self) {
        RateLimitModule::publish_wire_config(
            self.wire_ratelimit_enabled,
            self.wire_ratelimit_requests_per_second,
            self.wire_ratelimit_burst,
            self.wire_ratelimit_path.as_deref(),
        );
        MetricsModule::set_wire_enabled(self.wire_metrics_enabled);
    }

    pub fn module_count(&self) -> usize {
        self.registry.modules().len()
    }

    pub fn is_empty(&self) -> bool {
        self.module_count() == 0
    }

    pub async fn init(&self) -> anyhow::Result<()> {
        self.registry
            .init_all()
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.registry
            .shutdown_all()
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
    }

    async fn on_module_request(&self, module_req: &mut HttpRequest) -> anyhow::Result<()> {
        for module in self.registry.modules() {
            module
                .on_request(module_req)
                .await
                .map_err(|err| anyhow::anyhow!("{err}"))?;
        }
        Ok(())
    }

    async fn on_route_shortcircuit_for_module(
        &self,
        module_req: &HttpRequest,
    ) -> anyhow::Result<Option<Response<BoxBody>>> {
        for module in self.registry.modules() {
            let response = module
                .on_route(module_req)
                .await
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            if let Some(response) = response {
                return Ok(Some(boxed_from_module_response(response).await?));
            }
        }
        Ok(None)
    }

    pub async fn module_request(&self, req: &Request<Incoming>) -> anyhow::Result<HttpRequest> {
        module_request_from_incoming(req).await
    }

    async fn transform_response(
        &self,
        module_req: &HttpRequest,
        response: Response<BoxBody>,
    ) -> anyhow::Result<Response<BoxBody>> {
        if self.registry.modules().is_empty() {
            return Ok(response);
        }
        // Cap032: never collect text/event-stream — streaming must reach the client.
        // Cap054: modules observe real status + real headers with StreamingUnavailable body.
        if response_is_event_stream(&response) {
            let mut obs = ResponseObservation::streaming_unavailable(
                response.status(),
                response.headers().clone(),
            );
            for module in self.registry.modules() {
                module
                    .on_response(module_req, &mut obs)
                    .await
                    .map_err(|err| anyhow::anyhow!("{err}"))?;
            }
            // Re-apply metadata mutations only; never replace the streaming body.
            let (mut parts, body) = response.into_parts();
            parts.status = obs.status();
            parts.headers = obs.headers().clone();
            return Ok(Response::from_parts(parts, body));
        }
        let module_resp = module_response_from_boxed(response, MODULE_BODY_LIMIT).await?;
        let mut obs = ResponseObservation::from_http_response(module_resp);
        for module in self.registry.modules() {
            module
                .on_response(module_req, &mut obs)
                .await
                .map_err(|err| anyhow::anyhow!("{err}"))?;
        }
        let module_resp = obs
            .try_into_http_response()
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        boxed_from_module_response(module_resp).await
    }

    /// HTTP/1.x path: request filters → route short-circuit → core dispatch → response filters.
    pub async fn handle_incoming_request<F, Fut>(
        &self,
        req: Request<Incoming>,
        dispatch_core: F,
    ) -> Result<Response<BoxBody>, anyhow::Error>
    where
        F: FnOnce(Request<Incoming>) -> Fut,
        Fut: Future<Output = Response<BoxBody>>,
    {
        // One module_req for the whole chain so on_request extensions (e.g. timing)
        // survive into on_response (Cap054).
        let mut module_req = self.module_request(&req).await?;
        self.on_module_request(&mut module_req).await?;

        if let Some(response) = self.on_route_shortcircuit_for_module(&module_req).await? {
            return self.transform_response(&module_req, response).await;
        }

        let response = dispatch_core(req).await;
        self.transform_response(&module_req, response).await
    }

    /// HTTP/3 path: module request already built with client IP.
    pub async fn handle_http3_request<F, Fut>(
        &self,
        mut module_req: HttpRequest,
        dispatch_core: F,
    ) -> Result<Response<BoxBody>, anyhow::Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Response<BoxBody>>,
    {
        self.on_module_request(&mut module_req).await?;

        if let Some(response) = self.on_route_shortcircuit_for_module(&module_req).await? {
            return self.transform_response(&module_req, response).await;
        }

        let response = dispatch_core().await;
        self.transform_response(&module_req, response).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_config_ir::{CompressionConfig, MetricsConfig, RateLimitConfig};

    #[test]
    fn la_cap054_008_ratelimit_registers_before_metrics() {
        let config = ModulesConfig {
            metrics: MetricsConfig {
                enabled: true,
                ..MetricsConfig::default()
            },
            compression: CompressionConfig {
                enabled: true,
                ..CompressionConfig::default()
            },
            ratelimit: RateLimitConfig {
                enabled: true,
                requests_per_second: 10,
                burst: 10,
                path: None,
            },
        };
        let pipeline = CrossCuttingPipeline::from_config(&config, "0.4.4").expect("pipeline");
        let names: Vec<_> = pipeline
            .registry
            .modules()
            .iter()
            .map(|m| m.info().name.to_string())
            .collect();
        assert_eq!(
            names,
            vec!["ratelimit", "metrics", "compression"],
            "rate-limit must precede metrics scrape short-circuit"
        );
    }

    #[test]
    fn wire_cheap_hooks_install_without_compression() {
        // Process-global OnceLock: only assert when install succeeds (first in process).
        let config = ModulesConfig {
            metrics: MetricsConfig {
                enabled: true,
                ..MetricsConfig::default()
            },
            compression: CompressionConfig {
                enabled: false,
                ..CompressionConfig::default()
            },
            ratelimit: RateLimitConfig {
                enabled: true,
                requests_per_second: 1_000_000,
                burst: 1_000_000,
                path: None,
            },
        };
        let pipeline = CrossCuttingPipeline::from_config(&config, "0.4.4").expect("pipeline");
        assert_eq!(pipeline.module_count(), 2);
        // Admit must Allow under high burst (hooks installed or already present).
        assert_eq!(
            exyonq_module_api::wire_admit("203.0.113.50"),
            exyonq_module_api::WireAdmit::Allow
        );
        exyonq_module_api::wire_record_response(200);
    }

    /// Independent oracle: SSE observation carries real status/headers and
    /// StreamingUnavailable — not a fabricated empty body (RC-CRIT-002 / ZF-0123).
    #[tokio::test]
    async fn sse_observation_preserves_metadata_without_empty_body_lie() {
        use async_trait::async_trait;
        use bytes::Bytes;
        use exyonq_module_api::{
            Body, BoxError, HttpRequest, Module, ModuleInfo, ResponseBodyState, ResponseObservation,
        };
        use http::header::{CACHE_CONTROL, CONTENT_TYPE};
        use http::{Request, StatusCode};
        use http_body_util::BodyExt;
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Probe {
            seen: Mutex<Option<(u16, String, String, ResponseBodyState)>>,
        }

        #[async_trait]
        impl Module for Probe {
            fn info(&self) -> ModuleInfo {
                ModuleInfo {
                    name: "probe",
                    version: "0.0.0",
                    api_version: "1.0",
                }
            }

            async fn on_response(
                &self,
                _req: &HttpRequest,
                obs: &mut ResponseObservation,
            ) -> Result<(), BoxError> {
                let ct = obs
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let cc = obs
                    .headers()
                    .get(CACHE_CONTROL)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                *self.seen.lock().expect("lock") =
                    Some((obs.status().as_u16(), ct, cc, obs.body().clone()));
                Ok(())
            }
        }

        let probe = Arc::new(Probe::default());
        let mut registry = ModuleRegistry::new();
        registry.register(probe.clone());
        let pipeline = CrossCuttingPipeline {
            registry,
            wire_ratelimit_enabled: false,
            wire_ratelimit_requests_per_second: 1,
            wire_ratelimit_burst: 1,
            wire_ratelimit_path: None,
            wire_metrics_enabled: false,
        };

        let event_payload = Bytes::from_static(b"data: event-one\n\ndata: event-two\n\n");
        let response = http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream; charset=utf-8")
            .header(CACHE_CONTROL, "no-cache")
            .header("x-trace", "rc-crit-002")
            .body(
                http_body_util::Full::from(event_payload.clone())
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .unwrap();

        let req = Request::builder()
            .uri("http://127.0.0.1/sse/paced")
            .body(Body::from(Bytes::new()))
            .unwrap();

        let out = pipeline
            .transform_response(&req, response)
            .await
            .expect("transform");

        let seen = probe.seen.lock().expect("lock").clone().expect("observed");
        assert_eq!(seen.0, 200);
        assert!(
            seen.1.to_ascii_lowercase().starts_with("text/event-stream"),
            "observer must see real Content-Type, got {}",
            seen.1
        );
        assert_eq!(seen.2, "no-cache");
        assert_eq!(seen.3, ResponseBodyState::StreamingUnavailable);
        assert_ne!(seen.3, ResponseBodyState::Empty);

        assert_eq!(out.status(), StatusCode::OK);
        assert_eq!(
            out.headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream; charset=utf-8")
        );
        assert_eq!(
            out.headers()
                .get(CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-cache")
        );
        assert_eq!(
            out.headers().get("x-trace").and_then(|v| v.to_str().ok()),
            Some("rc-crit-002")
        );
        let client_bytes = BodyExt::collect(out.into_body())
            .await
            .expect("collect client body")
            .to_bytes();
        assert_eq!(
            client_bytes, event_payload,
            "module observation must not consume or alter SSE bytes"
        );
    }

    #[tokio::test]
    async fn sse_hostile_clear_body_cannot_force_406_with_live_stream() {
        use async_trait::async_trait;
        use bytes::Bytes;
        use exyonq_config_ir::{CompressionConfig, MetricsConfig, RateLimitConfig};
        use exyonq_module_api::{
            Body, BoxError, HttpRequest, Module, ModuleInfo, ResponseObservation,
        };
        use http::header::{ACCEPT_ENCODING, CONTENT_TYPE};
        use http::{Request, StatusCode};
        use http_body_util::BodyExt;
        use std::sync::Arc;

        struct Hostile;
        #[async_trait]
        impl Module for Hostile {
            fn info(&self) -> ModuleInfo {
                ModuleInfo {
                    name: "hostile",
                    version: "0.0.0",
                    api_version: "1.0",
                }
            }
            async fn on_response(
                &self,
                _req: &HttpRequest,
                obs: &mut ResponseObservation,
            ) -> Result<(), BoxError> {
                // Attempt EMPTY fabrication — API must refuse.
                assert!(obs.clear_body().is_err());
                assert!(obs.is_streaming_unavailable());
                Ok(())
            }
        }

        let config = ModulesConfig {
            metrics: MetricsConfig {
                enabled: false,
                ..MetricsConfig::default()
            },
            compression: CompressionConfig {
                enabled: true,
                min_bytes: 1,
                ..CompressionConfig::default()
            },
            ratelimit: RateLimitConfig {
                enabled: false,
                ..RateLimitConfig::default()
            },
        };
        let mut pipeline = CrossCuttingPipeline::from_config(&config, "0.4.4").expect("pipeline");
        pipeline.registry.register(Arc::new(Hostile));

        let event_payload = Bytes::from_static(b"data: keep-streaming\n\n");
        let response = http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream")
            .body(
                http_body_util::Full::from(event_payload.clone())
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .unwrap();
        let req = Request::builder()
            .uri("http://127.0.0.1/sse/hostile")
            .header(ACCEPT_ENCODING, "identity;q=0")
            .body(Body::from(Bytes::new()))
            .unwrap();
        let out = pipeline
            .transform_response(&req, response)
            .await
            .expect("transform");
        assert_eq!(
            out.status(),
            StatusCode::OK,
            "must not become 406 while streaming"
        );
        let client_bytes = BodyExt::collect(out.into_body())
            .await
            .expect("collect")
            .to_bytes();
        assert_eq!(client_bytes, event_payload);
    }

    #[tokio::test]
    async fn metrics_on_sse_records_status_without_claiming_empty_body() {
        use async_trait::async_trait;
        use bytes::Bytes;
        use exyonq_config_ir::{CompressionConfig, MetricsConfig, RateLimitConfig};
        use exyonq_module_api::{
            Body, BoxError, HttpRequest, Module, ModuleInfo, ResponseBodyState, ResponseObservation,
        };
        use http::header::{CACHE_CONTROL, CONTENT_TYPE};
        use http::{Request, StatusCode};
        use http_body_util::BodyExt;
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Probe {
            seen: Mutex<Option<ResponseBodyState>>,
        }
        #[async_trait]
        impl Module for Probe {
            fn info(&self) -> ModuleInfo {
                ModuleInfo {
                    name: "probe",
                    version: "0.0.0",
                    api_version: "1.0",
                }
            }
            async fn on_response(
                &self,
                _req: &HttpRequest,
                obs: &mut ResponseObservation,
            ) -> Result<(), BoxError> {
                *self.seen.lock().expect("lock") = Some(obs.body().clone());
                Ok(())
            }
        }

        let probe = Arc::new(Probe::default());
        let config = ModulesConfig {
            metrics: MetricsConfig {
                enabled: true,
                ..MetricsConfig::default()
            },
            compression: CompressionConfig {
                enabled: true,
                ..CompressionConfig::default()
            },
            ratelimit: RateLimitConfig {
                enabled: false,
                ..RateLimitConfig::default()
            },
        };
        let mut pipeline = CrossCuttingPipeline::from_config(&config, "0.4.4").expect("pipeline");
        pipeline.registry.register(probe.clone());
        let event_payload = Bytes::from_static(b"data: m1\n\ndata: m2\n\n");
        let response = http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream")
            .header(CACHE_CONTROL, "no-cache")
            .body(
                http_body_util::Full::from(event_payload.clone())
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .unwrap();
        let req = Request::builder()
            .uri("http://127.0.0.1/sse/metrics")
            .body(Body::from(Bytes::new()))
            .unwrap();
        let out = pipeline
            .transform_response(&req, response)
            .await
            .expect("transform");
        assert_eq!(out.status(), StatusCode::OK);
        let client_bytes = BodyExt::collect(out.into_body())
            .await
            .expect("collect")
            .to_bytes();
        assert_eq!(client_bytes, event_payload);
        assert_eq!(
            probe.seen.lock().expect("lock").clone(),
            Some(ResponseBodyState::StreamingUnavailable)
        );
    }

    #[tokio::test]
    async fn finite_empty_body_remains_empty_not_streaming() {
        use async_trait::async_trait;
        use bytes::Bytes;
        use exyonq_module_api::{
            Body, BoxError, HttpRequest, Module, ModuleInfo, ResponseBodyState, ResponseObservation,
        };
        use http::{Request, StatusCode};
        use http_body_util::BodyExt;
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Probe {
            seen: Mutex<Option<ResponseBodyState>>,
        }

        #[async_trait]
        impl Module for Probe {
            fn info(&self) -> ModuleInfo {
                ModuleInfo {
                    name: "probe",
                    version: "0.0.0",
                    api_version: "1.0",
                }
            }

            async fn on_response(
                &self,
                _req: &HttpRequest,
                obs: &mut ResponseObservation,
            ) -> Result<(), BoxError> {
                *self.seen.lock().expect("lock") = Some(obs.body().clone());
                Ok(())
            }
        }

        let probe = Arc::new(Probe::default());
        let mut registry = ModuleRegistry::new();
        registry.register(probe.clone());
        let pipeline = CrossCuttingPipeline {
            registry,
            wire_ratelimit_enabled: false,
            wire_ratelimit_requests_per_second: 1,
            wire_ratelimit_burst: 1,
            wire_ratelimit_path: None,
            wire_metrics_enabled: false,
        };

        let response = http::Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header(CONTENT_TYPE, "text/plain")
            .body(
                http_body_util::Full::from(Bytes::new())
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .unwrap();
        let req = Request::builder()
            .uri("http://127.0.0.1/empty")
            .body(Body::from(Bytes::new()))
            .unwrap();
        let _ = pipeline
            .transform_response(&req, response)
            .await
            .expect("transform");
        assert_eq!(
            probe.seen.lock().expect("lock").clone(),
            Some(ResponseBodyState::Empty)
        );
    }
}
