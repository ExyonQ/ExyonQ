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
//! Streaming WAF response-body inspection for large/unknown-length bodies.
//!
//! Small bodies are buffered by the handler gate (403 possible). This wrapper
//! streams large bodies, inspects the cumulative prefix up to the configured
//! limit and aborts the connection (body error) if a CRS rule blocks.

use bytes::{Bytes, BytesMut};
use exyonq_module_api::BoxError;
use exyonq_waf_api::{HeaderView, WafEngine, WafPhase, WafRequest, WafResponse};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

pub(crate) type FinalBoxBody = http_body_util::combinators::BoxBody<Bytes, BoxError>;

/// Marker: the buffered handler gate already inspected this response body.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WafBodyChecked;

#[derive(Debug)]
struct WafBodyError(&'static str);

impl std::fmt::Display for WafBodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for WafBodyError {}

#[derive(Clone)]
pub(crate) struct OwnedHeaders(pub Vec<(String, Vec<u8>)>);

impl OwnedHeaders {
    pub(crate) fn from_pairs(pairs: &[(String, String)]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.as_bytes().to_vec()))
                .collect(),
        )
    }
}

impl HeaderView for OwnedHeaders {
    fn get(&self, name: &str) -> Option<&[u8]> {
        self.0
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_slice())
    }

    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) {
        for (name, value) in &self.0 {
            f(name.as_bytes(), value);
        }
    }
}

struct WafBody {
    inner: FinalBoxBody,
    engine: Arc<dyn WafEngine>,
    method: String,
    host: Option<String>,
    path: String,
    query: Option<String>,
    request_headers: OwnedHeaders,
    client_ip: IpAddr,
    route_id: Option<String>,
    status: u16,
    response_headers: OwnedHeaders,
    max_bytes: usize,
    buffer: BytesMut,
    limit_reached: bool,
    done: bool,
    error: Option<BoxError>,
}

impl WafBody {
    fn inspect(&mut self, final_chunk: bool) -> Result<(), BoxError> {
        if self.limit_reached {
            return Ok(());
        }
        let inspect_len = if self.max_bytes == 0 {
            self.buffer.len()
        } else {
            self.buffer.len().min(self.max_bytes)
        };
        let req = WafRequest {
            method: &self.method,
            host: self.host.as_deref(),
            path: &self.path,
            query: self.query.as_deref(),
            headers: &self.request_headers,
            client_ip: self.client_ip,
            route_id: self.route_id.as_deref(),
        };
        let resp = WafResponse {
            status: self.status,
            headers: &self.response_headers,
            body: Some(&self.buffer[..inspect_len]),
        };
        let started = std::time::Instant::now();
        let decision = self
            .engine
            .inspect_response(WafPhase::ResponseBodyChunk, &req, &resp);
        crate::waf_metrics::note_inspection(WafPhase::ResponseBodyChunk, started.elapsed());
        crate::waf_metrics::note_decision(&decision);
        if decision.action == exyonq_waf_api::WafAction::Block {
            let _ = final_chunk;
            return Err(Box::new(WafBodyError("waf response body blocked")));
        }
        if self.max_bytes > 0 && self.buffer.len() > self.max_bytes {
            self.limit_reached = true;
        }
        Ok(())
    }
}

impl Body for WafBody {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        loop {
            if let Some(err) = this.error.take() {
                return Poll::Ready(Some(Err(err)));
            }
            if this.done {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.inner).poll_frame(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    if let Err(err) = this.inspect(true) {
                        this.error = Some(err);
                        continue;
                    }
                    this.done = true;
                    return Poll::Ready(None);
                }
                Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
                Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                    Ok(data) => {
                        // Cap the retained buffer at max_bytes; a configured
                        // inspection window must not grow into whole-body.
                        if this.max_bytes == 0 {
                            this.buffer.extend_from_slice(&data);
                        } else if this.buffer.len() < this.max_bytes {
                            let take = (this.max_bytes - this.buffer.len()).min(data.len());
                            this.buffer.extend_from_slice(&data[..take]);
                        }
                        if let Err(err) = this.inspect(false) {
                            this.error = Some(err);
                            continue;
                        }
                        return Poll::Ready(Some(Ok(Frame::data(data))));
                    }
                    Err(frame) => return Poll::Ready(Some(Ok(frame))),
                },
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.done
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

fn empty_body() -> FinalBoxBody {
    http_body_util::Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed()
}

/// Wrap a large/unknown-length response body for streaming CRS inspection.
#[allow(clippy::too_many_arguments)]
pub(crate) fn maybe_wrap_response(
    engine: Arc<dyn WafEngine>,
    method: &str,
    host: Option<&str>,
    path: &str,
    query: Option<&str>,
    request_headers: &[(String, String)],
    client_ip: IpAddr,
    route_id: Option<&str>,
    max_bytes: usize,
    response: &mut hyper::Response<FinalBoxBody>,
) {
    if !engine.inspects_response_body() || response.extensions().get::<WafBodyChecked>().is_some() {
        return;
    }
    if let Some(len) = response
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        if max_bytes > 0 && len <= max_bytes as u64 {
            // Small body: the handler gate already buffered and inspected it.
            return;
        }
    }
    let status = response.status().as_u16();
    let response_headers = OwnedHeaders(
        response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.as_bytes().to_vec(),
                )
            })
            .collect(),
    );
    let body = std::mem::replace(response.body_mut(), empty_body());
    *response.body_mut() = WafBody {
        inner: body,
        engine,
        method: method.to_string(),
        host: host.map(str::to_string),
        path: path.to_string(),
        query: query.map(str::to_string),
        request_headers: OwnedHeaders::from_pairs(request_headers),
        client_ip,
        route_id: route_id.map(str::to_string),
        status,
        response_headers,
        max_bytes,
        buffer: BytesMut::new(),
        limit_reached: false,
        done: false,
        error: None,
    }
    .boxed();
}
