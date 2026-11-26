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
//! Convert between hyper handler types and module-api HTTP types.

use exyonq_module_api::{Body, HttpRequest, HttpResponse, CLIENT_IP_HEADER};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::{Request, Response};
use std::fmt;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

pub async fn module_request_from_incoming(req: &Request<Incoming>) -> anyhow::Result<HttpRequest> {
    let mut builder = Request::builder().method(req.method()).uri(req.uri());
    for (name, value) in req.headers() {
        builder = builder.header(name, value);
    }
    Ok(builder.body(Body::from(bytes::Bytes::new()))?)
}

pub fn module_request_from_parts(
    req: &http::Request<()>,
    client_ip: &str,
) -> anyhow::Result<HttpRequest> {
    // P14C-C-A: clone HeaderMap once instead of per-header builder.append churn.
    let mut out = Request::builder()
        .method(req.method())
        .uri(req.uri())
        .body(Body::from(bytes::Bytes::new()))?;
    *out.headers_mut() = req.headers().clone();
    out.headers_mut().insert(
        CLIENT_IP_HEADER,
        HeaderValue::from_str(client_ip).expect("valid client ip header"),
    );
    Ok(out)
}

pub async fn module_response_from_boxed(
    response: Response<BoxBody>,
    limit: usize,
) -> anyhow::Result<HttpResponse> {
    let (parts, body) = response.into_parts();
    let bytes = BodyExt::collect(body)
        .await
        .map_err(|err| anyhow::anyhow!("collect response body: {err}"))?
        .to_bytes();
    if bytes.len() > limit {
        return Err(anyhow::Error::new(ModuleBodyTooLarge {
            limit,
            observed: bytes.len(),
        }));
    }
    Ok(Response::from_parts(parts, Body::from(bytes)))
}

/// Typed pipeline failure: the response body is larger than the configured
/// module collection ceiling. The caller turns this into `413`, not `500`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleBodyTooLarge {
    pub limit: usize,
    pub observed: usize,
}

impl fmt::Display for ModuleBodyTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "response body is {} bytes, over the {} byte module response limit",
            self.observed, self.limit
        )
    }
}

impl std::error::Error for ModuleBodyTooLarge {}

pub async fn boxed_from_module_response(
    response: HttpResponse,
) -> anyhow::Result<Response<BoxBody>> {
    let (parts, body) = response.into_parts();
    let bytes = BodyExt::collect(body)
        .await
        .map_err(|err| anyhow::anyhow!("collect module body: {err}"))?
        .to_bytes();
    Ok(Response::from_parts(
        parts,
        http_body_util::Full::from(bytes)
            .map_err(|never| match never {})
            .boxed(),
    ))
}

pub fn inject_client_ip(req: Request<Incoming>, client_ip: &str) -> Request<Incoming> {
    let (mut parts, body) = req.into_parts();
    parts.headers.insert(
        CLIENT_IP_HEADER,
        client_ip.parse().expect("valid client ip header"),
    );
    Request::from_parts(parts, body)
}
