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
//! Mechanical HTTP-01 response adapter — delegates to registered ACME integration service.

use exyonq_module_api::acme_integration::acme_integration_service;
use http::{Response, StatusCode};
use http_body_util::BodyExt;
use http_body_util::Full;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

/// Serve HTTP-01 key authorization when registered (no ACME policy in core).
pub fn try_http01_response(path: &str) -> Option<Response<BoxBody>> {
    let body = acme_integration_service()?.lookup_http01_key_authorization(path)?;
    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(
                Full::from(bytes::Bytes::from(body))
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .expect("valid acme challenge response"),
    )
}
