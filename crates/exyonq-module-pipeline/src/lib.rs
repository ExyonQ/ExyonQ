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
use exyonq_module_api::{HttpRequest, ModuleRegistry, RuntimeProfile};
use exyonq_ratelimit::RateLimitModule;
use hyper::body::Incoming;
use hyper::{Request, Response};
use semver::Version;
use std::future::Future;
use std::sync::Arc;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

const MODULE_BODY_LIMIT: usize = 4 * 1024 * 1024;

/// Per-snapshot pipeline instance (built at reload; owns module registry + rate-limit state).
pub struct CrossCuttingPipeline {
    registry: ModuleRegistry,
}

impl CrossCuttingPipeline {
    pub fn from_config(
        config: &ModulesConfig,
        core_version: &str,
    ) -> Result<Self, exyonq_module_api::AddonError> {
        let mut registry = ModuleRegistry::new();
        if config.metrics.enabled {
            registry.register_addon(Arc::new(MetricsModule::new(
                config.metrics.path.clone(),
                config.metrics.health_path.clone(),
                core_version,
            )))?;
        }
        if config.compression.enabled {
            registry.register_addon(Arc::new(CompressionModule::with_level(
                config.compression.min_bytes,
                config.compression.level,
            )))?;
        }
        if config.ratelimit.enabled {
            registry.register_addon(Arc::new(RateLimitModule::new(
                config.ratelimit.requests_per_second,
                config.ratelimit.burst,
            )))?;
        }
        let core_version = Version::parse(core_version).unwrap_or_else(|_| Version::new(0, 1, 0));
        registry.validate_handshake(&core_version, RuntimeProfile::Edge)?;
        Ok(Self { registry })
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

    async fn on_module_request(&self, module_req: &HttpRequest) -> anyhow::Result<()> {
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

    async fn on_request(&self, req: &Request<Incoming>) -> anyhow::Result<()> {
        let module_req = module_request_from_incoming(req).await?;
        self.on_module_request(&module_req).await
    }

    async fn on_route_shortcircuit(
        &self,
        req: &Request<Incoming>,
    ) -> anyhow::Result<Option<Response<BoxBody>>> {
        let module_req = module_request_from_incoming(req).await?;
        self.on_route_shortcircuit_for_module(&module_req).await
    }

    async fn transform_response(
        &self,
        module_req: &HttpRequest,
        response: Response<BoxBody>,
    ) -> anyhow::Result<Response<BoxBody>> {
        if self.registry.modules().is_empty() {
            return Ok(response);
        }
        let mut module_resp = module_response_from_boxed(response, MODULE_BODY_LIMIT).await?;
        for module in self.registry.modules() {
            module
                .on_response(module_req, &mut module_resp)
                .await
                .map_err(|err| anyhow::anyhow!("{err}"))?;
        }
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
        self.on_request(&req).await?;

        if let Some(response) = self.on_route_shortcircuit(&req).await? {
            let module_req = self.module_request(&req).await?;
            return self.transform_response(&module_req, response).await;
        }

        let module_req = self.module_request(&req).await?;
        let response = dispatch_core(req).await;
        self.transform_response(&module_req, response).await
    }

    /// HTTP/3 path: module request already built with client IP.
    pub async fn handle_http3_request<F, Fut>(
        &self,
        module_req: HttpRequest,
        dispatch_core: F,
    ) -> Result<Response<BoxBody>, anyhow::Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Response<BoxBody>>,
    {
        self.on_module_request(&module_req).await?;

        if let Some(response) = self.on_route_shortcircuit_for_module(&module_req).await? {
            return self.transform_response(&module_req, response).await;
        }

        let response = dispatch_core().await;
        self.transform_response(&module_req, response).await
    }
}
