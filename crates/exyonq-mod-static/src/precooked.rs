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
//! HTTP responses built once at preload, cloned cheaply per request.

use super::body::{BoxBody, StaticBody};
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::header::{HeaderValue, CONTENT_LENGTH, CONTENT_TYPE};
use hyper::{Response, StatusCode};
use std::sync::Arc;

pub struct PrecookedResponse {
    pub(crate) content_type: &'static str,
    pub(crate) body: Arc<Bytes>,
}

impl PrecookedResponse {
    pub fn new(body: Arc<Bytes>, content_type: &'static str) -> Arc<Self> {
        Arc::new(Self { content_type, body })
    }

    #[inline]
    pub fn to_response(self: &Arc<Self>) -> Response<BoxBody> {
        let mut response = Response::new(StaticBody::from_arc(Arc::clone(&self.body)).boxed());
        *response.status_mut() = StatusCode::OK;
        let headers = response.headers_mut();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(self.content_type));
        headers.insert(CONTENT_LENGTH, content_length_header(self.body.len()));
        response
    }

    #[inline]
    pub fn to_head_response(self: &Arc<Self>) -> Response<BoxBody> {
        let mut response = Response::new(
            http_body_util::Empty::<Bytes>::new()
                .map_err(|never| match never {})
                .boxed(),
        );
        *response.status_mut() = StatusCode::OK;
        let headers = response.headers_mut();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(self.content_type));
        headers.insert(CONTENT_LENGTH, content_length_header(self.body.len()));
        response
    }
}

/// HEAD response for octet-stream bench assets (Content-Length set, empty body).
#[inline]
pub fn octet_stream_head_response(body_len: usize) -> Response<BoxBody> {
    let mut response = Response::new(
        http_body_util::Empty::<Bytes>::new()
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(CONTENT_LENGTH, content_length_header(body_len));
    response
}

/// P7 HEAD: shared headers for preloaded routeNNN.bin.
#[inline]
pub fn route_bin_head_response(body_len: usize) -> Response<BoxBody> {
    octet_stream_head_response(body_len)
}

/// P7 hot path: shared headers for preloaded routeNNN.bin (512 B octet-stream).
#[inline]
pub fn route_bin_response(body: &Arc<Bytes>) -> Response<BoxBody> {
    let mut response = Response::new(StaticBody::from_arc(Arc::clone(body)).boxed());
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(CONTENT_LENGTH, content_length_header(body.len()));
    response
}

fn content_length_header(len: usize) -> HeaderValue {
    match len {
        512 => HeaderValue::from_static("512"),
        1024 => HeaderValue::from_static("1024"),
        65536 => HeaderValue::from_static("65536"),
        1048576 => HeaderValue::from_static("1048576"),
        _ => HeaderValue::from_str(&len.to_string()).expect("valid content-length"),
    }
}
