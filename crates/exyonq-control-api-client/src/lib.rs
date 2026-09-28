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
//! Cap016 Control API client — generated transport + handwritten facade.
//!
//! openapi-to-rust maps OpenAPI `apiKey` to Bearer via `with_api_key`.
//! Cap016 product auth uses header `X-ExyonQ-Server-Token`, so the facade
//! injects that header and never calls `with_api_key`.

#![allow(clippy::all)]

pub mod generated;

pub use generated::client::HttpClient;
pub use generated::types::*;

pub const SERVER_TOKEN_HEADER: &str = "X-ExyonQ-Server-Token";

pub fn control_api_client(
    base_url: impl Into<String>,
    server_token: impl Into<String>,
) -> HttpClient {
    HttpClient::new()
        .with_base_url(base_url)
        .with_header(SERVER_TOKEN_HEADER, server_token.into())
}
