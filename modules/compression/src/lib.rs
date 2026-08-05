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
//! Gzip response compression module (Fase 2).

use async_trait::async_trait;
use exyonq_addon_sdk::{
    official_core_compat, static_descriptor, Addon, AddonDescriptor, Capability, CostClass,
};
use exyonq_module_api::{Body, HttpRequest, HttpResponse, Module, ModuleInfo};
use flate2::write::GzEncoder;
use flate2::Compression;
use http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, VARY};
use http::{HeaderValue, StatusCode};
use std::io::Write;

static COMPRESSION_DESCRIPTOR: std::sync::LazyLock<AddonDescriptor> =
    std::sync::LazyLock::new(|| {
        static_descriptor(
            "compression",
            env!("CARGO_PKG_VERSION"),
            official_core_compat(),
            &[Capability::ResponseFilter],
            CostClass::LowCostHeaderOnly,
        )
    });

/// Static manifest for handshake registration (addon-api 1.x).
pub fn descriptor() -> &'static AddonDescriptor {
    &COMPRESSION_DESCRIPTOR
}

pub struct CompressionModule {
    min_bytes: usize,
    level: Compression,
}

impl CompressionModule {
    pub fn new(min_bytes: usize) -> Self {
        Self::with_level(min_bytes, 1)
    }

    pub fn with_level(min_bytes: usize, level: u32) -> Self {
        Self {
            min_bytes,
            level: Compression::new(level.clamp(1, 9)),
        }
    }

    fn accepts_gzip(req: &HttpRequest) -> bool {
        req.headers()
            .get(ACCEPT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.contains("gzip"))
            .unwrap_or(false)
    }

    fn compressible(resp: &HttpResponse) -> bool {
        if resp.status() != StatusCode::OK {
            return false;
        }
        if resp.headers().contains_key(CONTENT_ENCODING) {
            return false;
        }
        resp.headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value.starts_with("text/")
                    || value.contains("json")
                    || value.contains("javascript")
                    || value.contains("xml")
            })
            .unwrap_or(false)
    }
}

impl Addon for CompressionModule {
    fn descriptor(&self) -> &'static AddonDescriptor {
        descriptor()
    }
}

#[async_trait]
impl Module for CompressionModule {
    fn info(&self) -> ModuleInfo {
        ModuleInfo::from_descriptor(descriptor())
    }

    async fn on_response(
        &self,
        req: &HttpRequest,
        resp: &mut HttpResponse,
    ) -> Result<(), exyonq_module_api::BoxError> {
        if !Self::accepts_gzip(req) || !Self::compressible(resp) {
            return Ok(());
        }

        let body = std::mem::replace(resp.body_mut(), Body::from(bytes::Bytes::new()));
        let bytes = http_body_util::BodyExt::collect(body)
            .await
            .map_err(|err| -> exyonq_module_api::BoxError { Box::new(err) })?
            .to_bytes();

        if bytes.len() < self.min_bytes {
            *resp.body_mut() = Body::from(bytes);
            return Ok(());
        }

        let mut encoder = GzEncoder::new(Vec::new(), self.level);
        encoder.write_all(&bytes)?;
        let compressed = encoder.finish()?;

        resp.headers_mut()
            .insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        resp.headers_mut()
            .insert(VARY, HeaderValue::from_static("Accept-Encoding"));
        resp.headers_mut().remove(CONTENT_LENGTH);
        *resp.body_mut() = Body::from(compressed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{Request, Response};

    #[tokio::test]
    async fn compresses_text_response() {
        let module = CompressionModule::new(1);
        let req = Request::builder()
            .header(ACCEPT_ENCODING, "gzip")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let mut resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .body(Body::from("hello world hello world"))
            .unwrap();
        module.on_response(&req, &mut resp).await.unwrap();
        assert_eq!(
            resp.headers()
                .get(CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("gzip")
        );
    }
}
