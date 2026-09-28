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
//! Response compression — gzip + zlib deflate + Brotli + Cap071 Zstd via shared negotiation.

mod negotiate;

pub use negotiate::{negotiate, ContentCoding, NegotiateOutcome, Negotiation};

use async_trait::async_trait;
use brotli::enc::backward_references::BrotliEncoderMode;
use brotli::enc::{BrotliCompress, BrotliEncoderParams};
use exyonq_addon_sdk::{
    official_core_compat, static_descriptor, Addon, AddonDescriptor, Capability, CostClass,
};
#[cfg(test)]
use exyonq_module_api::Body;
use exyonq_module_api::{HttpRequest, Module, ModuleInfo, ResponseObservation};
use flate2::write::{GzEncoder, ZlibEncoder};
use flate2::Compression;
use http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, VARY};
use http::{HeaderValue, Method, StatusCode};
use std::io::{Cursor, Write};

/// Cap023: server-controlled Brotli quality (0–11). Bounded for dynamic HTTP.
const BROTLI_QUALITY: i32 = 4;
/// Cap023: lgwin 18 → 2^18 window (256 KiB). Bounded memory; not client-controlled.
const BROTLI_LGWIN: i32 = 18;
/// Cap071: libzstd default dynamic level (same as `encode_all(..., 0)` → 3).
/// Server-controlled; not client-influenced. Not maximum compression.
const ZSTD_COMPRESSION_LEVEL: i32 = 3;

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

    fn mime_compressible(obs: &ResponseObservation) -> bool {
        if Self::is_event_stream(obs) {
            return false;
        }
        obs.headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                let mime = value.split(';').next().unwrap_or(value).trim();
                mime.starts_with("text/")
                    || value.contains("json")
                    || value.contains("javascript")
                    || value.contains("xml")
            })
            .unwrap_or(false)
    }

    fn is_event_stream(obs: &ResponseObservation) -> bool {
        obs.headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
            })
    }

    fn ensure_vary_accept_encoding(headers: &mut http::HeaderMap) {
        // Cap044: AE variance must always be recorded. Never silently skip.
        // Non-UTF8 / unreadable Vary → replace with Accept-Encoding (fail closed for AE).
        match headers.get(VARY).and_then(|v| v.to_str().ok()) {
            None => {
                headers.insert(VARY, HeaderValue::from_static("Accept-Encoding"));
            }
            Some(existing) => {
                let has_ae = existing
                    .split(',')
                    .any(|t| t.trim().eq_ignore_ascii_case("accept-encoding"));
                if has_ae {
                    return;
                }
                let merged = format!("{existing}, Accept-Encoding");
                match HeaderValue::from_str(&merged) {
                    Ok(hv) => {
                        headers.insert(VARY, hv);
                    }
                    Err(_) => {
                        // Cap043/044: merge rejected → still guarantee AE token.
                        headers.insert(VARY, HeaderValue::from_static("Accept-Encoding"));
                    }
                }
            }
        }
    }

    fn apply_not_acceptable(obs: &mut ResponseObservation, is_head: bool) {
        *obs.status_mut() = StatusCode::NOT_ACCEPTABLE;
        obs.headers_mut().remove(CONTENT_ENCODING);
        obs.headers_mut().remove(CONTENT_LENGTH);
        Self::ensure_vary_accept_encoding(obs.headers_mut());
        // Cap071: HEAD must never carry an entity body (RFC 9110).
        if is_head {
            obs.clear_body()
                .expect("apply_not_acceptable only runs on finite bodies");
        } else {
            obs.set_body_bytes(bytes::Bytes::from_static(b"Not Acceptable"))
                .expect("apply_not_acceptable only runs on finite bodies");
        }
    }

    fn clear_body_if_head(req: &HttpRequest, obs: &mut ResponseObservation) {
        if *req.method() == Method::HEAD && !obs.is_streaming_unavailable() {
            obs.clear_body().expect("HEAD clear only on finite bodies");
        }
    }

    /// Cap044: RFC 9110 field combination — join all Accept-Encoding values.
    /// `HeaderMap::get` returns only the first; multi-line AE must not drop later offers.
    /// Non-UTF8 fields are skipped (LA-CAP044-001): never map a poisoned sibling to
    /// missing-AE identity when valid offers were already collected.
    fn accept_encoding_joined(headers: &http::HeaderMap) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        for value in headers.get_all(ACCEPT_ENCODING) {
            let Ok(s) = value.to_str() else {
                continue;
            };
            let t = s.trim();
            if !t.is_empty() {
                parts.push(t);
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }

    fn encode(coding: ContentCoding, plain: &[u8], level: Compression) -> std::io::Result<Vec<u8>> {
        match coding {
            ContentCoding::Gzip => {
                let mut enc = GzEncoder::new(Vec::new(), level);
                enc.write_all(plain)?;
                enc.finish()
            }
            ContentCoding::Deflate => {
                // Cap022: HTTP Content-Encoding: deflate = zlib wrap (RFC 1950).
                let mut enc = ZlibEncoder::new(Vec::new(), level);
                enc.write_all(plain)?;
                enc.finish()
            }
            ContentCoding::Brotli => {
                // Cap023: Content-Encoding: br — real Brotli (brotli 8.0.4).
                // BrotliCompress returns Result (unlike CompressorWriter::into_inner
                // which swallows FINISH errors). Mode = GENERIC; q/lgwin server-fixed.
                let params = BrotliEncoderParams {
                    quality: BROTLI_QUALITY,
                    lgwin: BROTLI_LGWIN,
                    mode: BrotliEncoderMode::BROTLI_MODE_GENERIC,
                    ..Default::default()
                };
                let mut input = Cursor::new(plain);
                let mut output = Vec::new();
                BrotliCompress(&mut input, &mut output, &params)?;
                Ok(output)
            }
            ContentCoding::Zstd => {
                // Cap071: Content-Encoding: zstd — reference libzstd via `zstd` 0.13.3.
                // encode_all returns io::Result; no unwrap/expect on the product path.
                // No dictionaries / dcz. Level is server-fixed.
                zstd::encode_all(plain, ZSTD_COMPRESSION_LEVEL)
            }
        }
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
        obs: &mut ResponseObservation,
    ) -> Result<(), exyonq_module_api::BoxError> {
        let result = self.on_response_inner(req, obs).await;
        // Cap071: defense-in-depth — HEAD never retains filter-produced entity bytes.
        Self::clear_body_if_head(req, obs);
        result
    }
}

impl CompressionModule {
    async fn on_response_inner(
        &self,
        req: &HttpRequest,
        obs: &mut ResponseObservation,
    ) -> Result<(), exyonq_module_api::BoxError> {
        // Cap032 / Cap054: never transform streaming observations, and never
        // buffer text/event-stream even if a prior module corrupted body_state.
        if obs.is_streaming_unavailable() || Self::is_event_stream(obs) {
            return Ok(());
        }

        let is_head = *req.method() == Method::HEAD;
        // Cap019 / Cap020: never transform non-200 (206/416/304/…).
        if obs.status() != StatusCode::OK {
            return Ok(());
        }
        // Already-encoded: no double compression; do not rewrite status.
        if obs.headers().contains_key(CONTENT_ENCODING) {
            return Ok(());
        }

        let ae_owned = Self::accept_encoding_joined(req.headers());
        let neg = negotiate(ae_owned.as_deref());

        // MIME gate: if we cannot encode this representation, resolve with can_apply=false.
        if !Self::mime_compressible(obs) {
            return match neg.resolve(false) {
                NegotiateOutcome::NotAcceptable => {
                    Self::apply_not_acceptable(obs, is_head);
                    Ok(())
                }
                NegotiateOutcome::Identity | NegotiateOutcome::Encode(_) => {
                    Self::ensure_vary_accept_encoding(obs.headers_mut());
                    Ok(())
                }
            };
        }

        match neg.resolve(true) {
            NegotiateOutcome::Identity => {
                // AE negotiation participated — identity is still a Vary-sensitive variant.
                Self::ensure_vary_accept_encoding(obs.headers_mut());
                Ok(())
            }
            NegotiateOutcome::NotAcceptable => {
                Self::apply_not_acceptable(obs, is_head);
                Ok(())
            }
            NegotiateOutcome::Encode(coding) => {
                let Some(bytes) = obs.take_finite_bytes() else {
                    // StreamingUnavailable restored by take_finite_bytes — no-op.
                    return Ok(());
                };

                // Cap044: HEAD often arrives with empty entity bytes while Content-Length
                // still describes the GET representation size. Use that for min_bytes gate
                // so negotiation CE matches GET (body remains empty after clear_body_if_head).
                let size_for_gate = if is_head && bytes.is_empty() {
                    obs.headers()
                        .get(CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(0)
                } else {
                    bytes.len()
                };

                if size_for_gate < self.min_bytes {
                    // Cannot apply selected coding — fall back only if identity allowed.
                    return match neg.resolve(false) {
                        NegotiateOutcome::NotAcceptable => {
                            Self::apply_not_acceptable(obs, is_head);
                            Ok(())
                        }
                        NegotiateOutcome::Identity | NegotiateOutcome::Encode(_) => {
                            Self::ensure_vary_accept_encoding(obs.headers_mut());
                            obs.set_body_bytes(bytes)
                                .map_err(|err| -> exyonq_module_api::BoxError { Box::new(err) })?;
                            Ok(())
                        }
                    };
                }

                // HEAD with empty body but eligible size: apply CE/Vary without inventing
                // a coded body. Strip Content-Length (encoded length unknown without entity).
                if is_head && bytes.is_empty() {
                    obs.headers_mut()
                        .insert(CONTENT_ENCODING, HeaderValue::from_static(coding.as_str()));
                    Self::ensure_vary_accept_encoding(obs.headers_mut());
                    obs.headers_mut().remove(CONTENT_LENGTH);
                    obs.clear_body()
                        .map_err(|err| -> exyonq_module_api::BoxError { Box::new(err) })?;
                    return Ok(());
                }

                let compressed = Self::encode(coding, &bytes, self.level)
                    .map_err(|err| -> exyonq_module_api::BoxError { Box::new(err) })?;

                obs.headers_mut()
                    .insert(CONTENT_ENCODING, HeaderValue::from_static(coding.as_str()));
                Self::ensure_vary_accept_encoding(obs.headers_mut());
                // Encoded Content-Length must describe coded bytes, never plain.
                let cl = HeaderValue::from_str(&compressed.len().to_string())
                    .map_err(|err| -> exyonq_module_api::BoxError { Box::new(err) })?;
                obs.headers_mut().insert(CONTENT_LENGTH, cl);
                obs.set_body_bytes(bytes::Bytes::from(compressed))
                    .map_err(|err| -> exyonq_module_api::BoxError { Box::new(err) })?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::{GzDecoder, ZlibDecoder};
    use http::{Request, Response};
    use std::io::Read;

    async fn apply_on_response(
        module: &CompressionModule,
        req: &HttpRequest,
        resp: Response<Body>,
    ) -> Response<Body> {
        let mut obs = ResponseObservation::from_http_response(resp);
        module.on_response(req, &mut obs).await.unwrap();
        obs.try_into_http_response().expect("finite observation")
    }

    async fn run(
        ae: &str,
        body: &str,
        ctype: &str,
        min: usize,
    ) -> (StatusCode, Option<String>, Vec<u8>, Option<usize>) {
        let module = CompressionModule::new(min);
        let req = Request::builder()
            .header(ACCEPT_ENCODING, ae)
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, ctype)
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = apply_on_response(&module, &req, resp).await;
        let status = resp.status();
        let ce = resp
            .headers()
            .get(CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let cl = resp
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok());
        let out = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, ce, out, cl)
    }

    #[tokio::test]
    async fn compresses_zstd() {
        let payload = "cap071-zstd-payload-xxxxxxxxxxxxxxxx";
        let (st, ce, raw, cl) = run("zstd", payload, "text/plain", 1).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(ce.as_deref(), Some("zstd"));
        assert_eq!(cl, Some(raw.len()));
        let plain = zstd::decode_all(&raw[..]).unwrap();
        assert_eq!(plain, payload.as_bytes());
        // Magic number of a Zstandard frame (RFC 8878).
        assert!(raw.len() >= 4 && raw[..4] == [0x28, 0xB5, 0x2F, 0xFD]);
    }

    #[tokio::test]
    async fn compresses_brotli() {
        let payload = "cap023-brotli-payload-xxxxxxxxxxxxxxxx";
        let (st, ce, raw, cl) = run("br", payload, "text/plain", 1).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(ce.as_deref(), Some("br"));
        assert_eq!(cl, Some(raw.len()));
        let mut plain = Vec::new();
        brotli::BrotliDecompress(&mut std::io::Cursor::new(&raw), &mut plain).unwrap();
        assert_eq!(plain, payload.as_bytes());
    }

    #[tokio::test]
    async fn compresses_gzip() {
        let (st, ce, raw, _) = run("gzip", "hello world hello world", "text/plain", 1).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(ce.as_deref(), Some("gzip"));
        let mut d = GzDecoder::new(&raw[..]);
        let mut plain = String::new();
        d.read_to_string(&mut plain).unwrap();
        assert_eq!(plain, "hello world hello world");
    }

    #[tokio::test]
    async fn compresses_deflate_zlib_wrap() {
        let payload = "cap022-deflate-payload-xxxxxxxx";
        let (st, ce, raw, _) = run("deflate", payload, "text/plain", 1).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(ce.as_deref(), Some("deflate"));
        let mut d = ZlibDecoder::new(&raw[..]);
        let mut plain = String::new();
        d.read_to_string(&mut plain).unwrap();
        assert_eq!(plain, payload);
        assert!(raw.len() >= 2 && raw[0] == 0x78, "expected zlib CMF byte");
    }

    #[tokio::test]
    async fn q_prefers_gzip_over_br() {
        let (st, ce, _, _) = run(
            "gzip;q=1, br;q=0.5",
            "hello world hello world",
            "text/plain",
            1,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(ce.as_deref(), Some("gzip"));
    }

    #[tokio::test]
    async fn equal_q_tie_prefers_zstd() {
        let (st, ce, _, _) = run(
            "zstd;q=1, br;q=1, gzip;q=1, deflate;q=1",
            "hello world hello world",
            "text/plain",
            1,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(ce.as_deref(), Some("zstd"));
    }

    #[tokio::test]
    async fn equal_q_without_zstd_prefers_br() {
        let (st, ce, _, _) = run(
            "gzip, deflate, br",
            "hello world hello world",
            "text/plain",
            1,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(ce.as_deref(), Some("br"));
    }

    #[tokio::test]
    async fn zstd_q_zero_not_selected() {
        let (st, ce, body, _) = run("zstd;q=0", "hello world hello world", "text/plain", 1).await;
        assert_eq!(st, StatusCode::OK);
        assert!(ce.is_none());
        assert_eq!(body, b"hello world hello world");
    }

    #[tokio::test]
    async fn br_q_zero_not_selected() {
        let (st, ce, body, _) = run("br;q=0", "hello world hello world", "text/plain", 1).await;
        assert_eq!(st, StatusCode::OK);
        assert!(ce.is_none());
        assert_eq!(body, b"hello world hello world");
    }

    #[tokio::test]
    async fn not_acceptable_when_identity_forbidden() {
        let (st, ce, _, _) = run("identity;q=0", "hello world hello world", "text/plain", 1).await;
        assert_eq!(st, StatusCode::NOT_ACCEPTABLE);
        assert!(ce.is_none());
    }

    #[tokio::test]
    async fn min_bytes_with_identity_forbidden_is_406() {
        let (st, ce, _, _) = run("br, identity;q=0", "tiny", "text/plain", 32).await;
        assert_eq!(st, StatusCode::NOT_ACCEPTABLE);
        assert!(ce.is_none());
    }

    #[tokio::test]
    async fn skips_already_encoded() {
        let module = CompressionModule::new(1);
        let req = Request::builder()
            .header(ACCEPT_ENCODING, "zstd")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .header(CONTENT_ENCODING, "gzip")
            .body(Body::from("hello world hello world"))
            .unwrap();
        let resp = apply_on_response(&module, &req, resp).await;
        assert_eq!(
            resp.headers()
                .get(CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("gzip")
        );
    }

    #[tokio::test]
    async fn skips_304_and_206() {
        let module = CompressionModule::new(1);
        for status in [StatusCode::NOT_MODIFIED, StatusCode::PARTIAL_CONTENT] {
            let req = Request::builder()
                .header(ACCEPT_ENCODING, "zstd")
                .body(Body::from(bytes::Bytes::new()))
                .unwrap();
            let resp = Response::builder()
                .status(status)
                .header(http::header::CONTENT_TYPE, "text/plain")
                .body(Body::from("partial-or-empty-body-bytes!!!!"))
                .unwrap();
            let resp = apply_on_response(&module, &req, resp).await;
            assert!(resp.headers().get(CONTENT_ENCODING).is_none());
            assert_eq!(resp.status(), status);
        }
    }

    #[tokio::test]
    async fn identity_outcome_sets_vary() {
        let (st, ce, body, _) = run("br;q=0", "hello world hello world", "text/plain", 1).await;
        assert_eq!(st, StatusCode::OK);
        assert!(ce.is_none());
        assert_eq!(body, b"hello world hello world");
        let module = CompressionModule::new(1);
        let req = Request::builder()
            .header(ACCEPT_ENCODING, "br;q=0")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .body(Body::from("hello world hello world"))
            .unwrap();
        let resp = apply_on_response(&module, &req, resp).await;
        let vary = resp
            .headers()
            .get(VARY)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(vary.to_ascii_lowercase().contains("accept-encoding"));
    }

    #[tokio::test]
    async fn vary_merges_existing() {
        let module = CompressionModule::new(1);
        let req = Request::builder()
            .header(ACCEPT_ENCODING, "br")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .header(VARY, "Origin")
            .body(Body::from("hello world hello world"))
            .unwrap();
        let resp = apply_on_response(&module, &req, resp).await;
        let vary = resp
            .headers()
            .get(VARY)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(vary.to_ascii_lowercase().contains("origin"));
        assert!(vary.to_ascii_lowercase().contains("accept-encoding"));
        assert_eq!(
            resp.headers()
                .get(CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("br")
        );
    }

    #[tokio::test]
    async fn head_not_acceptable_has_empty_body() {
        let module = CompressionModule::new(1);
        let req = Request::builder()
            .method(Method::HEAD)
            .header(ACCEPT_ENCODING, "identity;q=0")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .body(Body::from("should-not-leak-on-head!!!!!!"))
            .unwrap();
        let resp = apply_on_response(&module, &req, resp).await;
        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
        assert!(resp.headers().get(CONTENT_ENCODING).is_none());
        let out = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert!(out.is_empty(), "HEAD 406 must not carry entity body");
    }

    #[tokio::test]
    async fn head_encode_path_clears_body() {
        let module = CompressionModule::new(1);
        let req = Request::builder()
            .method(Method::HEAD)
            .header(ACCEPT_ENCODING, "zstd")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .body(Body::from("hello world hello world hello"))
            .unwrap();
        let resp = apply_on_response(&module, &req, resp).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("zstd")
        );
        let out = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert!(out.is_empty(), "HEAD zstd must not leak encoded body");
    }

    #[tokio::test]
    async fn multi_accept_encoding_fields_are_joined() {
        // Cap044: two AE fields combine; equal q → br over gzip.
        let module = CompressionModule::new(1);
        let mut req = Request::builder()
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        req.headers_mut()
            .append(ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
        req.headers_mut()
            .append(ACCEPT_ENCODING, HeaderValue::from_static("br"));
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .body(Body::from("hello world hello world hello"))
            .unwrap();
        let resp = apply_on_response(&module, &req, resp).await;
        assert_eq!(
            resp.headers()
                .get(CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("br")
        );
    }

    #[tokio::test]
    async fn multi_ae_skips_non_utf8_sibling_keeps_valid_offers() {
        // LA-CAP044-001: poison field must not erase identity;q=0 + gzip.
        let module = CompressionModule::new(1);
        let mut req = Request::builder()
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        req.headers_mut().append(
            ACCEPT_ENCODING,
            HeaderValue::from_static("identity;q=0, gzip"),
        );
        req.headers_mut().append(
            ACCEPT_ENCODING,
            HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .body(Body::from("hello world hello world hello"))
            .unwrap();
        let resp = apply_on_response(&module, &req, resp).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("gzip"),
            "non-UTF8 AE sibling must not fail-open to identity"
        );
    }

    #[tokio::test]
    async fn head_empty_body_uses_content_length_for_gate() {
        // Cap044: static/proxy HEAD may arrive with empty body + CL of GET size.
        let module = CompressionModule::new(32);
        let req = Request::builder()
            .method(Method::HEAD)
            .header(ACCEPT_ENCODING, "gzip")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .header(CONTENT_LENGTH, "128")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let resp = apply_on_response(&module, &req, resp).await;
        assert_eq!(
            resp.headers()
                .get(CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("gzip")
        );
        assert!(resp.headers().get(CONTENT_LENGTH).is_none());
        let out = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn event_stream_not_compressible() {
        // Cap032: text/event-stream must stay identity even with Accept-Encoding.
        let (st, ce, body, _) = run(
            "gzip, br, zstd, deflate",
            "data: tiny-sse-event\n\n",
            "text/event-stream; charset=utf-8",
            1,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert!(
            ce.is_none(),
            "SSE must not attach Content-Encoding, got {ce:?}"
        );
        assert_eq!(body, b"data: tiny-sse-event\n\n");
    }

    #[tokio::test]
    async fn streaming_unavailable_observation_is_noop() {
        use http::header::HeaderMap;
        let module = CompressionModule::new(1);
        let req = Request::builder()
            .header(ACCEPT_ENCODING, "gzip")
            .body(Body::from(bytes::Bytes::new()))
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            "text/event-stream".parse().unwrap(),
        );
        let mut obs = ResponseObservation::streaming_unavailable(StatusCode::OK, headers);
        module.on_response(&req, &mut obs).await.unwrap();
        assert!(obs.is_streaming_unavailable());
        assert!(obs.headers().get(CONTENT_ENCODING).is_none());
        assert_eq!(obs.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn zstd_frame_settings_observed() {
        let payload = "cap071-frame-settings-probe-xxxxxxxx";
        let (st, ce, raw, _) = run("zstd", payload, "text/plain", 1).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(ce.as_deref(), Some("zstd"));
        // Descriptor byte after magic (RFC 8878): bit2=Content_Checksum; bits6-7=FCS_Field_Size.
        assert_eq!(&raw[..4], &[0x28, 0xB5, 0x2F, 0xFD]);
        let descriptor = raw[4];
        let checksum = (descriptor & 0x04) != 0;
        let fcs_field_size = match descriptor >> 6 {
            0 => 0usize,
            1 => 2,
            2 => 4,
            3 => 8,
            _ => unreachable!(),
        };
        let single_segment = (descriptor & 0x20) != 0;
        // Observed `zstd::encode_all` / stream Encoder defaults (not assumed a priori):
        // no content checksum; FCS may be absent on multi-frame stream path.
        assert!(
            !checksum,
            "encode_all default unexpectedly set content checksum"
        );
        let _ = (fcs_field_size, single_segment);
        let plain = zstd::decode_all(&raw[..]).unwrap();
        assert_eq!(plain, payload.as_bytes());
    }
}
